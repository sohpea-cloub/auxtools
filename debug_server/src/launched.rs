//! Debugging a world that the launcher (`auxtools-cli`) started.
//!
//! The launcher loads this library into BYOND before the world is loaded:
//! `LD_PRELOAD` on Linux, and on Windows a call to [`auxtools_debug_attach`]
//! put in front of the program's entry point, which `AUXTOOLS_DEBUG` asks for.
//! The world then listens on a local socket named after its process for any
//! number of debug-clients: `AUXTOOLS_DEBUG=wait` has it wait for one to say
//! it's configured before its code runs, `run` doesn't. The world's code needs
//! none of auxtools' procs:
//! they're added when the world loads, along with one that sleeps in a loop so
//! the debugger's requests are handled even when nothing else runs. This is
//! the only way the debug server runs.

use std::{
	collections::{BTreeMap, HashMap},
	sync::{
		atomic::{AtomicBool, Ordering},
		OnceLock
	}
};

use auxtools::{byond_string, Proc, Value, WorldLoad};
use dmasm::reload::{ProcKey, Snapshot};

use crate::{
	connection, hot_reload,
	server::Server,
	server_types::{HotReloadNewProc, HotReloadProc, HotReloadRequest, ProcRef},
	waker
};

/// Set by the launcher: `wait` or `run`
pub const DEBUG_VAR: &str = "AUXTOOLS_DEBUG";

/// The procs auxtools wants a world to have, and what the launcher's worlds
/// get. auxtools shuts down after the world's own `world/Del`, as worlds that
/// call `auxtools_shutdown` there do, and starts again with the next world
/// when it's a reboot (`world/Reboot` was called), which loads the world
/// again. A world that's ending can still run code on its way out, so that
/// doesn't start it again.
const HELPER_PROCS: &str = r#"
/proc/auxtools_expr_stub()
	CRASH("auxtools_expr_stub is where the debugger evaluates expressions")

/proc/auxtools_stack_trace(msg)
	CRASH(msg)

/proc/auxtools_keepalive()
	set waitfor = FALSE
	// Has an entry when a client asks for code to be run as the world's own.
	// Only then does the loop call a proc: one called while the world reboots
	// would start the debugger in the world that's going.
	var/list/inbox = auxtools_spawn_inbox()
	while(TRUE)
		if(inbox && inbox.len)
			inbox.len = 0
			var/list/next = auxtools_spawn_next()
			while(next)
				auxtools_spawned(next[1], next[2])
				next = auxtools_spawn_next()
		sleep(1)

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

/proc/auxtools_launched_world_del()
	CRASH("the debugger's library isn't loaded")

/proc/auxtools_launched_world_reboot()
	CRASH("the debugger's library isn't loaded")

/world/Reboot()
	auxtools_launched_world_reboot()
	return ..()

/world/Del()
	try
		..()
	catch(var/exception/e)
		auxtools_launched_world_del()
		throw e
	auxtools_launched_world_del()
"#;

/// What the launcher's file is called in stack traces
const HELPER_FILE: &str = "auxtools_launcher.dm";

/// The helpers that override the world's procs. They're set apart from the
/// world's own definitions (see [`Proc::set_apart`]), so those keep the
/// numbers their source gives them, and stay the most derived when a reload
/// adds more.
const HELPER_OVERRIDES: [&str; 2] = ["/world/Del", "/world/Reboot"];

/// Set when `world/Reboot` is called, for `world/Del` to tell a reboot from
/// the end
static REBOOTING: AtomicBool = AtomicBool::new(false);

/// Whether the world's loop is ours, which runs what clients spawn: a world
/// with an `auxtools_keepalive` of its own keeps it
static KEEPALIVE_OURS: AtomicBool = AtomicBool::new(false);

/// Whether the world runs what clients spawn: its loop is ours, and the
/// procs it calls are hooked
static SPAWNS: AtomicBool = AtomicBool::new(false);

/// Why the world can't run what a client spawns, if it can't
pub fn can_spawn() -> Result<(), String> {
	if SPAWNS.load(Ordering::Relaxed) {
		return Ok(());
	}
	if !KEEPALIVE_OURS.load(Ordering::Relaxed) && Proc::find("/proc/auxtools_keepalive").is_some() {
		return Err("this world can't run spawned code: it has an /proc/auxtools_keepalive of its own, which doesn't run it".to_owned());
	}
	Err("this world can't run spawned code: it doesn't have the debugger's helper procs for it".to_owned())
}

static WORLD_LOAD: WorldLoad = WorldLoad { prepare, ready, failed };

/// Whether a launcher started this world
pub fn launched() -> bool {
	std::env::var_os(DEBUG_VAR).is_some()
}

// DreamDaemon's dependencies are loaded by now, libbyond.so among them
#[cfg(unix)]
#[auxtools::ctor::ctor]
fn on_load() {
	if !launched() {
		return;
	}

	// Programs the world runs with shell() aren't BYOND
	std::env::remove_var("LD_PRELOAD");
	attach();
}

/// Called by the code the launcher puts in front of the program's entry point,
/// on its main thread, once byondcore.dll is loaded
#[cfg(windows)]
#[no_mangle]
pub extern "C" fn auxtools_debug_attach() {
	if launched() {
		attach();
	}
}

/// The program's command line and working directory, as they were when the
/// library loaded: DreamDaemon cuts the .dmb's path up in place afterwards
static COMMAND_LINE: OnceLock<(Vec<String>, String)> = OnceLock::new();

/// What the world was started with, which says which .dmb it runs
pub fn command_line() -> (Vec<String>, String) {
	COMMAND_LINE.get().cloned().unwrap_or_else(read_command_line)
}

fn read_command_line() -> (Vec<String>, String) {
	let cwd = std::env::current_dir().map(|dir| dir.to_string_lossy().into_owned()).unwrap_or_default();
	(std::env::args_os().map(|arg| arg.to_string_lossy().into_owned()).collect(), cwd)
}

fn attach() {
	let _ = COMMAND_LINE.set(read_command_line());
	if let Err(e) = auxtools::init_on_world_load(&WORLD_LOAD) {
		failed(&e);
	}
}

/// Adds the procs in [`HELPER_PROCS`] the world doesn't have
fn prepare() -> Result<(), String> {
	let request = helper_procs()?;
	KEEPALIVE_OURS.store(
		request
			.new_procs
			.iter()
			.any(|proc| proc.type_path.is_empty() && proc.name == "auxtools_keepalive"),
		Ordering::Relaxed
	);
	if request.is_empty() {
		return Ok(());
	}

	let mut warnings = vec![];
	let prepared = hot_reload::prepare(&request, &mut warnings).map_err(|errors| errors.join("; "))?;
	hot_reload::apply(prepared, &mut HashMap::<_, BTreeMap<(), ()>>::new(), &mut HashMap::new(), &mut warnings);
	for warning in warnings {
		eprintln!("auxtools: {}", warning);
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
		// `/name` for a global proc
		let (type_path, name) = key.path.rsplit_once('/').ok_or_else(|| format!("{} has no name", key.path))?;
		let (type_path, name) = (type_path.to_owned(), name.to_owned());

		// A world made for auxtools has its own. Ours goes after any
		// world/Del the world has.
		let declared = type_path.is_empty();
		if declared && Proc::find(&key.path).is_some() {
			continue;
		}

		let code = snapshot.compile_reload(&key).map_err(|e| format!("{}: {}", key.path, e))?;
		let override_id = if declared { 0 } else { overrides(&key.path) };
		request.new_procs.push(HotReloadNewProc {
			type_path,
			name,
			declared,
			verb: false,
			code: HotReloadProc {
				proc: ProcRef {
					path: key.path.clone(),
					override_id
				},
				nodes: code.nodes,
				locals: code.locals,
				params: code.params,
				settings: code.settings,
				statics: code.statics
			}
		});
	}
	Ok(request)
}

/// How many definitions a proc has
fn overrides(path: &str) -> u32 {
	(0..).take_while(|&id| Proc::find_override(path, id).is_some()).count() as u32
}

fn ready() {
	if let Some(proc) = Proc::find("/proc/auxtools_launched_world_del") {
		let _ = proc.hook(world_del);
	}
	if let Some(proc) = Proc::find("/proc/auxtools_launched_world_reboot") {
		let _ = proc.hook(world_reboot);
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
	SPAWNS.store(spawns, Ordering::Relaxed);
	for path in HELPER_OVERRIDES {
		// Ours is the last
		if let Some(proc) = overrides(path).checked_sub(1).and_then(|last| Proc::find_override(path, last)) {
			proc.set_apart();
			hot_reload::pin_world_proc(proc.id.0);
		}
	}

	// Where to wake it if it sleeps with no players (see `waker`)
	if let Ok(port) = Value::WORLD.get_number(byond_string!("port")) {
		waker::set_port(port as u16);
	}
	// Before anyone can ask: a client waiting for the reboot hears of it in
	// the first thing it reads
	crate::journal::world_loaded();
	let mut server = match Server::listen() {
		Ok(server) => server,
		Err(e) => {
			eprintln!("auxtools: couldn't listen for debuggers at {}: {}", connection::world_name(), e);
			return;
		}
	};
	if std::env::var(DEBUG_VAR).is_ok_and(|value| value == "wait") {
		server.process_until_configured();
	}
	crate::install_server(server);

	// It sleeps straight away, which returns here
	if let Some(proc) = Proc::find("/proc/auxtools_keepalive") {
		let _ = proc.call(&[]);
	}
}

fn world_del(_: &Value, _: &Value, _: Vec<Value>) -> auxtools::DMResult {
	if REBOOTING.swap(false, Ordering::Relaxed) {
		// Ours, added after the world loaded: the next world doesn't have it
		// until this has started again
		let ours = Proc::find_override("/world/Del", Proc::APART);
		auxtools::init_on_next_world_load(ours.as_ref());
	} else {
		auxtools::shut_down_with_world();
	}
	Ok(Value::NULL)
}

fn world_reboot(_: &Value, _: &Value, _: Vec<Value>) -> auxtools::DMResult {
	REBOOTING.store(true, Ordering::Relaxed);
	Ok(Value::NULL)
}

/// The list the world's loop looks at for spawned code to run
fn spawn_inbox(_: &Value, _: &Value, _: Vec<Value>) -> auxtools::DMResult {
	match unsafe { &mut *crate::DEBUG_SERVER.get() } {
		Some(server) => Ok(server.spawn_inbox()),
		None => Ok(Value::NULL)
	}
}

/// What the world's loop runs next of what clients spawned: its number and
/// the arguments for its code, which is in `/proc/auxtools_spawn_stub` now;
/// null for nothing
fn spawn_next(_: &Value, _: &Value, _: Vec<Value>) -> auxtools::DMResult {
	match unsafe { &mut *crate::DEBUG_SERVER.get() } {
		Some(server) => server.next_spawn(),
		None => Ok(Value::NULL)
	}
}

/// What spawned code gave: the list its code returns, or null if it failed
fn spawn_done(_: &Value, _: &Value, args: Vec<Value>) -> auxtools::DMResult {
	let number = args.first().and_then(|number| number.as_number().ok());
	if let (Some(server), Some(number)) = (unsafe { &mut *crate::DEBUG_SERVER.get() }, number) {
		server.spawn_done(number as u64, args.get(1).unwrap_or(&Value::NULL));
	}
	Ok(Value::NULL)
}

/// Tells whoever connects why there's no debugger
fn failed(error: &str) {
	eprintln!("auxtools: couldn't start the debugger: {}", error);
	Server::refuse(format!("auxtools couldn't start: {}", error));
}
