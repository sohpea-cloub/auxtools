//! Debugging a world that the launcher (`tools/auxtools-cli`) started.
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

use std::{collections::HashMap, sync::OnceLock};

use auxtools::{Proc, Value, WorldLoad};
use dmasm::reload::{ProcKey, Snapshot};

use crate::{
	connection, hot_reload,
	server::Server,
	server_types::{HotReloadNewProc, HotReloadProc, HotReloadRequest, ProcRef}
};

/// Set by the launcher: `wait` or `run`
pub const DEBUG_VAR: &str = "AUXTOOLS_DEBUG";

/// The procs auxtools wants a world to have, and what the launcher's worlds
/// get. `world/Del` runs before a reboot, which loads the world again: auxtools
/// shuts down after the world's own `world/Del` and starts again with the next
/// world, as worlds that call `auxtools_shutdown` there do.
const HELPER_PROCS: &str = r#"
/proc/auxtools_expr_stub()
	CRASH("auxtools_expr_stub is where the debugger evaluates expressions")

/proc/auxtools_stack_trace(msg)
	CRASH(msg)

/proc/auxtools_keepalive()
	set waitfor = FALSE
	while(TRUE)
		sleep(1)

/proc/auxtools_launched_world_del()
	CRASH("the debugger's library isn't loaded")

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
	if request.is_empty() {
		return Ok(());
	}

	let mut warnings = vec![];
	let prepared = hot_reload::prepare(&request, &mut warnings).map_err(|errors| errors.join("; "))?;
	hot_reload::apply(prepared, &mut HashMap::<_, ()>::new(), &mut HashMap::new(), &mut warnings);
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
	let snapshot = Snapshot::from_tree(ctx, tree);

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
	auxtools::init_on_next_world_load();
	Ok(Value::NULL)
}

/// Tells whoever connects why there's no debugger
fn failed(error: &str) {
	eprintln!("auxtools: couldn't start the debugger: {}", error);
	Server::refuse(format!("auxtools couldn't start: {}", error));
}
