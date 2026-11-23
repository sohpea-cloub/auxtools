use std::{
	collections::{BTreeMap, HashMap},
	sync::{
		atomic::{AtomicBool, Ordering},
		OnceLock
	}
};

use auxtools::{byond_string, Proc, Value, WorldLoad};
use auxtools_protocol::{HotReloadRequest, HELPER_FILE};
use dmasm::reload::{NewProc, ProcKey, Snapshot};

use crate::{connection, hot_reload, server::Server, waker};

pub const DEBUG_VAR: &str = "AUXTOOLS_DEBUG";

const HELPER_PROCS: &str = r#"
/proc/auxtools_expr_stub()
	CRASH("auxtools_expr_stub is where the debugger evaluates expressions")

/proc/auxtools_stack_trace(msg)
	CRASH(msg)

/proc/auxtools_keepalive()
	set waitfor = FALSE
	var/list/inbox = auxtools_spawn_inbox()
	while(TRUE)
		if(inbox && inbox.len)
			inbox.len = 0
			var/list/next = auxtools_spawn_next()
			while(next)
				auxtools_spawned(next[1], next[2])
				next = auxtools_spawn_next()
		sleep(world.tick_lag)

/proc/auxtools_spawn_inbox()
	return null

/proc/auxtools_spawn_next()
	return null

/proc/auxtools_spawn_stub()
	CRASH("auxtools_spawn_stub is where the debugger runs what it's asked to spawn")

/proc/auxtools_spawned(number, list/given)
	set waitfor = FALSE
	auxtools_spawn_done(number, auxtools_spawn_stub(arglist(given)))

/proc/auxtools_spawn_done(number, result)
	CRASH("the debugger's library isn't loaded")

/proc/auxtools_kept()
	var/static/list/kept = list()
	return kept

/proc/auxtools_launched_world_del(failed)
	CRASH("the debugger's library isn't loaded")

/proc/auxtools_launched_world_reboot()
	CRASH("the debugger's library isn't loaded")

/proc/auxtools_launched_world_reboot_declined()
	CRASH("the debugger's library isn't loaded")

/world/Reboot()
	auxtools_launched_world_reboot()
	try
		. = ..()
	catch(var/exception/e)
		auxtools_launched_world_reboot_declined()
		throw e
	auxtools_launched_world_reboot_declined()

/proc/auxtools_launched_world_new()
	CRASH("the debugger's library isn't loaded")

/world/New()
	auxtools_launched_world_new()
	return ..()

/world/Del()
	try
		..()
	catch(var/exception/e)
		auxtools_launched_world_del(TRUE)
		throw e
	auxtools_launched_world_del()
"#;

const HELPER_OVERRIDES: [&str; 3] = ["/world/Del", "/world/Reboot", "/world/New"];

static REBOOTING: AtomicBool = AtomicBool::new(false);

static KEEPALIVE_OURS: AtomicBool = AtomicBool::new(false);

static SPAWNS_AVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn can_spawn() -> Result<(), String> {
	if SPAWNS_AVAILABLE.load(Ordering::Relaxed) {
		return Ok(());
	}
	if !KEEPALIVE_OURS.load(Ordering::Relaxed) && Proc::find("/proc/auxtools_keepalive").is_some() {
		return Err("this world can't run spawned code: it has an /proc/auxtools_keepalive of its own, which doesn't run it".to_owned());
	}
	Err("this world can't run spawned code: it doesn't have the debugger's helper procs for it".to_owned())
}

pub fn runs_spawns(proc: &Proc) -> bool {
	KEEPALIVE_OURS.load(Ordering::Relaxed) && matches!(proc.path.as_str(), "/auxtools_keepalive" | "/auxtools_spawned")
}

pub fn is_helper(proc: &Proc) -> bool {
	proc.path.trim_start_matches("/proc").starts_with("/auxtools_")
}

static WORLD_LOAD: WorldLoad = WorldLoad { prepare, ready, failed };

pub fn launched() -> bool {
	std::env::var_os(DEBUG_VAR).is_some()
}

#[cfg(unix)]
#[auxtools::ctor::ctor]
fn on_load() {
	if !launched() {
		return;
	}

	std::env::remove_var("LD_PRELOAD");
	attach();
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn auxtools_debug_attach() {
	if launched() {
		if std::env::var(crate::socket_hold_windows::HOLD_VAR).is_ok_and(|value| value == "0") {
			eprintln!("auxtools: socket reuse workaround disabled ({}=0)", crate::socket_hold_windows::HOLD_VAR);
		} else {
			match crate::socket_hold_windows::install() {
				Ok(_) => eprintln!(
					"auxtools: socket reuse workaround enabled (90s hold; {}=0 to disable)",
					crate::socket_hold_windows::HOLD_VAR
				),
				Err(e) => eprintln!("auxtools: socket reuse workaround failed: {}", e)
			}
		}
		attach();
	}
}

static COMMAND_LINE: OnceLock<(Vec<String>, String)> = OnceLock::new();

pub fn command_line() -> (Vec<String>, String) {
	COMMAND_LINE.get().cloned().unwrap_or_else(read_command_line)
}

fn read_command_line() -> (Vec<String>, String) {
	let cwd = std::env::current_dir().map(|dir| dir.to_string_lossy().into_owned()).unwrap_or_default();
	(std::env::args_os().map(|arg| arg.to_string_lossy().into_owned()).collect(), cwd)
}

fn attach() {
	let attached = crate::guard(|| {
		let _ = COMMAND_LINE.set(read_command_line());
		auxtools::init_on_world_load(&WORLD_LOAD)
	});
	if let Err(e) = attached.and_then(|initialised| initialised) {
		let _ = crate::guard(|| failed(&e));
	}
}

fn prepare() -> Result<(), String> {
	crate::guard(add_helper_procs)?
}

fn add_helper_procs() -> Result<(), String> {
	crate::world_resources::loading();
	let request = helper_procs()?;
	KEEPALIVE_OURS.store(
		request
			.plan
			.new_procs
			.iter()
			.any(|proc| proc.type_path.is_empty() && proc.name == "auxtools_keepalive"),
		Ordering::Relaxed
	);
	if request.is_empty() {
		return Ok(());
	}

	let mut warnings = vec![];
	let appended = hot_reload::prepare(&request, &mut warnings, &mut vec![])
		.and_then(hot_reload::append)
		.map_err(|errors| errors.join("; "))?;
	let mut failures = vec![];
	hot_reload::apply(
		appended,
		&mut HashMap::<_, BTreeMap<(), ()>>::new(),
		&mut HashMap::new(),
		&mut warnings,
		&mut failures
	);
	for warning in warnings {
		eprintln!("auxtools: {}", warning);
	}
	for failure in failures {
		eprintln!("auxtools: the debugger's helper procs are partly added: {}", failure);
	}
	Ok(())
}

fn helper_procs() -> Result<HotReloadRequest, String> {
	let ctx = dreammaker::Context::default();
	let pp = dreammaker::Preprocessor::from_buffer(&ctx, HELPER_FILE.into(), HELPER_PROCS);
	let mut parser = dreammaker::Parser::new(&ctx, pp);
	parser.enable_procs();
	let tree = parser.parse_object_tree();
	let mut snapshot = Snapshot::from_tree(ctx, tree);
	snapshot.set_byond(crate::world_byond());

	let mut request = HotReloadRequest::default();
	let keys: Vec<ProcKey> = snapshot.procs().cloned().collect();
	for key in keys {
		let (type_path, name) = key.path.rsplit_once('/').ok_or_else(|| format!("{} has no name", key.path))?;
		let (type_path, name) = (type_path.to_owned(), name.to_owned());

		let declared = type_path.is_empty();
		if declared && Proc::find(&key.path).is_some() {
			continue;
		}

		let mut code = snapshot.compile_reload(&key).map_err(|e| format!("{}: {}", key.path, e))?;
		code.proc.override_id = if declared { 0 } else { definition_count(&key.path) };
		request.plan.new_procs.push(NewProc {
			type_path,
			name,
			declared,
			verb: false,
			code
		});
	}
	Ok(request)
}

fn definition_count(path: &str) -> u32 {
	(0..).take_while(|&id| Proc::find_override(path, id).is_some()).count() as u32
}

fn ready() {
	if let Err(e) = crate::guard(start) {
		failed(&e);
	}
}

fn start() {
	if let Some(proc) = Proc::find("/proc/auxtools_launched_world_del") {
		let _ = proc.hook(world_del);
	}
	if let Some(proc) = Proc::find("/proc/auxtools_launched_world_reboot") {
		let _ = proc.hook(world_reboot);
	}
	if let Some(proc) = Proc::find("/proc/auxtools_launched_world_reboot_declined") {
		let _ = proc.hook(world_reboot_declined);
	}
	if !Proc::find("/proc/auxtools_launched_world_new").is_some_and(|proc| proc.hook(world_new).is_ok()) {
		crate::world_resources::loaded();
	}
	REBOOTING.store(false, Ordering::Relaxed);
	let hooked =
		|path: &str, hook: fn(&Value, &Value, Vec<Value>) -> auxtools::DMResult| Proc::find(path).is_some_and(|proc| proc.hook(hook).is_ok());
	let spawns = KEEPALIVE_OURS.load(Ordering::Relaxed)
		& hooked("/proc/auxtools_spawn_inbox", spawn_inbox)
		& hooked("/proc/auxtools_spawn_next", spawn_next)
		& hooked("/proc/auxtools_spawn_done", spawn_done)
		&& ["/proc/auxtools_spawn_stub", "/proc/auxtools_spawned"]
			.into_iter()
			.all(|path| Proc::find(path).is_some());
	SPAWNS_AVAILABLE.store(spawns, Ordering::Relaxed);
	for path in HELPER_OVERRIDES {
		if let Some(proc) = definition_count(path).checked_sub(1).and_then(|last| Proc::find_override(path, last)) {
			proc.set_apart();
			hot_reload::pin_world_proc(proc.id.0);
		}
	}

	if let Ok(port) = Value::WORLD.get_number(byond_string!("port")) {
		waker::set_port(port as u16);
	}
	crate::journal::world_loaded();
	let mut server = match Server::listen() {
		Ok(server) => server,
		Err(e) => {
			eprintln!("auxtools: couldn't listen for debuggers at {}: {}", connection::world_name(), e);
			return;
		}
	};
	if std::env::var(DEBUG_VAR).is_ok_and(|value| value == "wait") {
		let ends = crate::WORLD_ENDS.load(Ordering::Relaxed);
		let _ = crate::guard_server(&mut server, Server::process_until_configured);
		if crate::WORLD_ENDS.load(Ordering::Relaxed) != ends {
			return;
		}
	}
	crate::install_server(server);

	if let Some(proc) = Proc::find("/proc/auxtools_keepalive") {
		let _ = proc.call(&[]);
	}
}

fn world_new(_: &Value, _: &Value, _: Vec<Value>) -> auxtools::DMResult {
	crate::world_resources::loaded();
	crate::on_server(Server::world_data_loaded);
	Ok(Value::NULL)
}

fn world_del(_: &Value, _: &Value, args: Vec<Value>) -> auxtools::DMResult {
	let failed = args.first().is_some_and(|failed| failed.as_number().is_ok_and(|failed| failed != 0.0));
	if REBOOTING.swap(false, Ordering::Relaxed) {
		let ours = Proc::find_override("/world/Del", Proc::APART);
		auxtools::init_on_next_world_load(ours.as_ref());
	} else if !failed {
		crate::on_server(Server::world_shutting_down);
		auxtools::shut_down_with_world();
	} else {
		crate::on_server(Server::world_carries_on);
	}
	Ok(Value::NULL)
}

fn world_reboot(_: &Value, _: &Value, _: Vec<Value>) -> auxtools::DMResult {
	REBOOTING.store(true, Ordering::Relaxed);
	Ok(Value::NULL)
}

fn world_reboot_declined(_: &Value, _: &Value, _: Vec<Value>) -> auxtools::DMResult {
	REBOOTING.store(false, Ordering::Relaxed);
	crate::on_server(Server::world_carries_on);
	Ok(Value::NULL)
}

fn spawn_inbox(_: &Value, _: &Value, _: Vec<Value>) -> auxtools::DMResult {
	Ok(crate::on_server(Server::spawn_inbox).unwrap_or(Value::NULL))
}

fn spawn_next(_: &Value, _: &Value, _: Vec<Value>) -> auxtools::DMResult {
	crate::on_server(Server::next_spawn).unwrap_or(Ok(Value::NULL))
}

fn spawn_done(_: &Value, _: &Value, args: Vec<Value>) -> auxtools::DMResult {
	let number = args.first().and_then(|number| number.as_number().ok());
	if let Some(number) = number {
		crate::on_server(|server| server.spawn_done(number as u64, args.get(1).unwrap_or(&Value::NULL)));
	}
	Ok(Value::NULL)
}

fn failed(error: &str) {
	eprintln!("auxtools: couldn't start the debugger: {}", error);
	Server::refuse(error.to_owned());
}
