use super::*;

const EXIT_GRACE: Duration = Duration::from_secs(5);
const TERMINATE_GRACE: Duration = Duration::from_secs(5);
const EXIT_INTERVAL: Duration = Duration::from_millis(100);
const REBOOT_PATIENCE: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
pub(super) struct RestartArguments {
	arguments: Option<Value>
}

pub(super) struct Prepared {
	program: PathBuf,
	dmb: PathBuf,
	dme: Option<PathBuf>,
	library: PathBuf,
	args: Vec<String>
}

impl Prepared {
	fn of(args: &LaunchArguments) -> Result<Self, String> {
		let dme = args.dme.clone().or_else(|| args.dmb.as_ref().map(|dmb| dmb.with_extension("dme")));
		let dmb = args
			.dmb
			.clone()
			.or_else(|| args.dme.as_ref().map(|dme| dme.with_extension("dmb")))
			.ok_or("specify dme or dmb to select a world")?;

		let bin = byond::bin_dir(args.byond_path.as_deref())?;
		let server = args.server.unwrap_or_else(Server::default_for_platform);
		let program = bin.join(server.program());
		if !program.is_file() {
			return Err(format!("{} was not found in {}", server.program(), bin.display()));
		}
		if !dmb.is_file() {
			return Err(format!("file not found: {}", dmb.display()));
		}
		let library = match &args.debug_library {
			Some(library) => library.clone(),
			None => default_library()?
		};
		if args.port.is_some() && server == Server::DreamSeeker && cfg!(windows) {
			return Err("port is unsupported with DreamSeeker; use server dreamdaemon".to_owned());
		}
		let mut game_args: Vec<String> = args.port.iter().map(u16::to_string).collect();
		game_args.extend(args.args.clone().unwrap_or_else(|| default_args(server)));
		Ok(Self {
			program,
			dmb,
			dme,
			library,
			args: game_args
		})
	}
}

impl Adapter {
	pub(super) fn launch(&mut self, args: LaunchArguments) -> Handled {
		if self.pid.is_some() {
			return Err("a world is already running".to_owned());
		}
		let prepared = Prepared::of(&args)?;
		self.configured_flags = args.build_flags.clone();
		self.start(args, prepared)?;
		Ok(Value::Null)
	}

	pub(super) fn start(&mut self, args: LaunchArguments, prepared: Prepared) -> Result<(), String> {
		self.debugging = !args.no_debug;
		self.sleeping_threads = args.sleeping_threads.unwrap_or(aux::DEFAULT_SLEEPING_STACKS);
		let same_code = self.launched.as_ref().is_some_and(|old| {
			(&old.dme, &old.dmb, &old.defines, &old.build_flags, old.hot_reload, &old.byond_path)
				== (&args.dme, &args.dmb, &args.defines, &args.build_flags, args.hot_reload, &args.byond_path)
		});
		if !same_code {
			self.drop_code();
			self.macros = None;
			self.code_read = false;
			self.watching = false;
			self.code_settings = Some(CodeSettings {
				dme: prepared.dme.clone().filter(|dme| dme.is_file()),
				defines: all_defines(&args),
				hot_reload: args.hot_reload
			});
			self.wanted_defines = None;
		}

		let launch = Launch {
			program: prepared.program,
			dmb: &prepared.dmb,
			args: &prepared.args,
			env: &args.env,
			debug: Some((prepared.library, self.debugging || args.hot_reload))
		};
		let (out, inputs, files) = (self.out.clone(), self.inputs.clone(), self.files.clone());
		let batch = journal::batch_output(self.inputs.clone());
		let game = Game::start(
			launch,
			move |category, line| {
				if category != "console" {
					let _ = batch.send(line.trim_end_matches(['\r', '\n']).to_owned());
				}
				match runtime_place(&line) {
					Some((file, number)) => {
						let path = files.lock().unwrap().path(file);
						out.event(OutputEvent {
							category: Some(category.to_owned()),
							source: Some(source_at(&path)),
							line: Some(number),
							output: line,
							..Default::default()
						});
					}
					None => out.output(category, line)
				}
			},
			move |code| {
				let _ = inputs.send(Input::GameExited(code));
			}
		)?;
		self.pid = Some(game.pid());
		self.game = Some(game);
		self.launched = Some(args);
		self.out.event(dap::ProcessEvent {
			name: prepared.dmb.to_string_lossy().into_owned(),
			systemProcessId: self.pid.unwrap_or_default(),
			isLocalProcess: true,
			startMethod: "launch".to_owned()
		});
		self.progress_start(STARTING, "Starting the world".to_owned());
		self.started(None);
		Ok(())
	}

	pub(super) fn attach(&mut self, args: AttachArguments) -> Handled {
		if self.pid.is_some() {
			return Err("a world is already running".to_owned());
		}
		let pid = match &args.process_id {
			Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
			Value::String(s) => s.trim().parse().ok(),
			_ => None
		}
		.ok_or("specify processId to attach")?;
		if !local::alive(pid) {
			return Err(format!("process not found: {}", pid));
		}
		let stream = local::connect(pid).map_err(|e| {
			format!(
				"process {} isn't a world with this version of auxtools' debugger in it (protocol {}): {}",
				pid,
				aux::PROTOCOL_VERSION,
				e
			)
		})?;

		self.sleeping_threads = args.sleeping_threads.unwrap_or(aux::DEFAULT_SLEEPING_STACKS);
		self.code_settings = Some(CodeSettings {
			dme: args.dme,
			defines: args.defines,
			hot_reload: args.hot_reload
		});
		self.pid = Some(pid);
		self.out.console(format!("Attaching to process {}", pid));
		self.out.event(dap::ProcessEvent {
			name: format!("process {}", pid),
			systemProcessId: pid,
			isLocalProcess: true,
			startMethod: "attach".to_owned()
		});
		self.started(Some(stream));
		Ok(Value::Null)
	}

	pub(super) fn started(&mut self, stream: Option<Stream>) {
		if !self.debugging && !self.initialized {
			self.initialized = true;
			self.out.event(InitializedEvent);
		}
		match stream {
			Some(stream) => {
				self.generation += 1;
				let _ = self.inputs.send(Input::Connected {
					generation: self.generation,
					stream
				});
			}
			None => self.connect()
		}
	}

	pub(super) fn connect(&mut self) {
		let Some(pid) = self.pid.filter(|_| !self.unavailable) else {
			return;
		};
		self.generation += 1;
		let generation = self.generation;
		let inputs = self.inputs.clone();
		let launched = self.game.is_some();
		std::thread::Builder::new()
			.name("connect".to_owned())
			.spawn(move || match when_listening(pid, None, local::connect) {
				Ok(stream) => {
					let _ = inputs.send(Input::Connected { generation, stream });
				}
				Err(_) if launched => {}
				Err(_) => {
					let _ = inputs.send(Input::ProcessGone);
				}
			})
			.expect("couldn't start a thread");
	}

	pub(super) fn connected(&mut self, stream: Stream) {
		self.progress_end(STARTING);
		self.progress_end(RESTARTING);
		let debuggee = Debuggee::new(
			stream,
			self.generation,
			self.debugging,
			self.out.clone(),
			self.inputs.clone(),
			self.last_error.clone(),
			self.set.clone()
		);
		match debuggee {
			Ok(debuggee) => self.debuggee = Some(debuggee),
			Err(e) => {
				self.out.console(format!("Debug server connection failed: {}", e));
				return;
			}
		}
		self.set.lock().unwrap().clear();
		self.watches_lost();

		let byond = match self.request(&AuxRequest::ByondVersion) {
			Ok(AuxResponse::ByondVersion { version, build }) => (version, build),
			_ => return
		};
		self.shutting_down = false;
		self.rebooting = false;
		self.send_output();
		self.give_macros();
		self.follow_reloads();
		if !self.debugging {
			let _ = self.ack(&AuxRequest::Watcher);
		}
		if self.stddef.is_none() {
			if let Ok(AuxResponse::StdDef(Some(text))) = self.request(&AuxRequest::StdDef) {
				self.stddef = Some(Stddef::new(text));
			}
		}

		let reloads = match &self.code_settings {
			Some(settings) => settings.hot_reload,
			None => self.hot_reload && self.code.is_some()
		};
		if reloads && !self.claim_reloads() {
			match &mut self.code_settings {
				Some(settings) => settings.hot_reload = false,
				None => {
					self.hot_reload = false;
					self.drop_code();
				}
			}
		}

		match self.code_settings.take() {
			Some(settings) => self.start_code(settings),
			None => self.out.console("The world restarted")
		}
		self.waiting_for_code = match &self.code {
			Some(code) => {
				let world = self.generation;
				code.send(code::Command::Connected { byond, world }).is_ok() && self.code_read
			}
			None => false
		};
		if !self.waiting_for_code {
			self.ready();
		}
	}

	pub(super) fn claim_reloads(&mut self) -> bool {
		let who = match self.game {
			Some(_) => format!("the launcher (pid {})", std::process::id()),
			None => format!("another debugger session (pid {})", std::process::id())
		};
		match self.request(&AuxRequest::Reloader { who }) {
			Ok(AuxResponse::Reloader(Err(why))) => {
				self.out.console(format!("Hot reload disabled: {}", why));
				self.reload_state("off", Some(why));
				false
			}
			_ => true
		}
	}

	pub(super) fn configuration_done(&mut self) -> Handled {
		if self.debugging && self.debuggee.is_some() {
			self.catch_runtimes()?;
			self.ack(&AuxRequest::Configured)?;
			self.catch_up_on_pause();
			self.announce_world();
			return Ok(Value::Null);
		}
		self.catch_up_on_pause();
		Ok(Value::Null)
	}

	pub(super) fn ready(&mut self) {
		if !self.initialized {
			self.initialized = true;
			self.out.event(InitializedEvent);
			return;
		}
		self.configure();
	}

	pub(super) fn configure(&mut self) {
		if self.debugging {
			let _ = self.catch_runtimes();
			let files: Vec<String> = self.breakpoints.keys().filter(|file| !self.deferred.contains(*file)).cloned().collect();
			for file in files {
				let breakpoints = self.apply_breakpoints(&file);
				self.tell(breakpoints, false);
			}
			let breakpoints = self.apply_function_breakpoints();
			self.tell(breakpoints, false);
			self.rewatch();
			let _ = self.apply_instruction_breakpoints();
		}
		let _ = self.ack(&AuxRequest::Configured);
		self.catch_up_on_pause();
		self.configured = true;
		self.announce_world();
		self.say_ready();
	}

	pub(super) fn say_ready(&mut self) {
		if !self.said_ready && self.configured && (self.code.is_none() || self.watching) {
			self.said_ready = true;
			self.out.ready();
		}
	}

	pub(super) fn catch_up_on_pause(&mut self) {
		if self.debugging {
			if let Some(debuggee) = &mut self.debuggee {
				let _ = debuggee.catch_up_on_pause();
			}
		}
	}

	pub(super) fn connection_closed(&mut self, generation: u64, reason: &str) {
		if self.debuggee.as_ref().is_none_or(|d| d.generation != generation) {
			return;
		}
		if let Some(mut debuggee) = self.debuggee.take() {
			debuggee.gone();
		}
		self.waiting_for_code = false;
		self.set.lock().unwrap().clear();

		if self.pid.is_some() && !self.ending {
			if !self.shutting_down && !self.rebooting {
				self.out.console(format!("Debug server disconnected ({}); waiting to reconnect", reason));
				self.progress_start(RESTARTING, "Waiting to reconnect".to_owned());
			}
			self.connect();
		}
	}

	pub(super) fn debugger_unavailable(&mut self, generation: u64, reason: &str) {
		if self.debuggee.as_ref().is_none_or(|d| d.generation != generation) {
			return;
		}
		if let Some(mut debuggee) = self.debuggee.take() {
			debuggee.gone();
		}
		self.unavailable = true;
		self.waiting_for_code = false;
		self.set.lock().unwrap().clear();
		self.progress_end(STARTING);
		self.progress_end(RESTARTING);
		let process = self.world_process().unwrap_or_default();
		self.out
			.console(format!("Debugger initialization failed for process {}: {}", process, reason));
		if self.game.is_none() {
			self.out.console("The world is running without debugging. Detaching from this session.");
			self.pid = None;
			self.out.event(TerminatedEvent::default());
			return;
		}
		self.out
			.console("The world is running without debugging or hot reload. Stop the session to terminate it.");
		if !self.initialized {
			self.initialized = true;
			self.out.event(InitializedEvent);
		}
		self.out.unavailable(reason);
	}

	pub(super) fn disconnect(&mut self, terminate: Option<bool>) {
		if let Some(debuggee) = &mut self.debuggee {
			debuggee.leave();
		}
		self.debuggee = None;
		if terminate == Some(false) && self.game.is_some() {
			self.out
				.console("Stopping the world launched by this session. Attach to an existing world to leave it running on disconnect.");
		}
		if terminate.unwrap_or(false) || self.game.is_some() {
			if let Some(process) = self.world_process() {
				self.out.console(format!("Ending the world (process {})", process));
				self.kill_world();
			}
		}
		self.pid = None;
		self.generation += 1;
	}

	pub(super) fn world_process(&self) -> Option<u32> {
		self.game.as_ref().map(Game::pid).or(self.pid)
	}

	pub(super) fn kill_world(&mut self) {
		if let Some(game) = &self.game {
			game.kill();
		} else if let Some(pid) = self.pid {
			local::kill(pid);
		}
	}

	pub(super) fn terminate(&mut self) -> Handled {
		let Some(process) = self.world_process() else {
			return Ok(Value::Null);
		};
		if self.ending {
			if self.relaunching.take().is_some() {
				self.progress_end(RESTARTING);
				self.out.console("Restart cancelled: the world is stopping");
			}
			return Ok(Value::Null);
		}
		self.ending = true;
		let asked = self.debuggee.as_mut().is_some_and(|debuggee| debuggee.end(Ending::Stop).is_ok());
		if asked {
			self.out.console("Stopping the world");
			let inputs = self.inputs.clone();
			let launched = self.game.is_some();
			std::thread::spawn(move || {
				let deadline = std::time::Instant::now() + local::test_patience(TERMINATE_GRACE);
				let mut overdue = false;
				while launched || local::alive(process) {
					if !overdue && std::time::Instant::now() >= deadline {
						overdue = true;
						if inputs.send(Input::TerminateOverdue { process }).is_err() || launched {
							return;
						}
					}
					std::thread::sleep(EXIT_INTERVAL);
				}
				let _ = inputs.send(Input::ProcessGone);
			});
		} else if self.unavailable {
			self.out
				.console(format!("Cannot call world.Del() without the debugger; terminating process {}", process));
			self.kill_world();
		} else {
			self.out.console(format!("Debug server disconnected; terminating process {}", process));
			self.kill_world();
		}
		Ok(Value::Null)
	}

	pub(super) fn restart(&mut self, args: RestartArguments) -> Handled {
		if self.ending {
			return Err("the world is already stopping".to_owned());
		}
		let Some(launched) = self.launched.clone() else {
			return self.reboot();
		};
		let mut wanted = match args.arguments {
			Some(config) => arguments::<LaunchArguments>(Some(config))?,
			None => launched.clone()
		};
		if wanted.build_flags == self.configured_flags {
			wanted.build_flags = launched.build_flags.clone();
		} else {
			self.configured_flags = wanted.build_flags.clone();
		}
		if wanted == launched && self.debuggee.is_some() && !self.unavailable {
			return self.reboot();
		}
		let prepared = Prepared::of(&wanted)?;
		self.relaunching = Some((wanted, prepared));
		self.progress_start(RESTARTING, "Restarting the world".to_owned());
		self.terminate()
	}

	pub(super) fn reboot(&mut self) -> Handled {
		if self.ending {
			return Err("the world is already stopping".to_owned());
		}
		self.debuggee()?.end(Ending::Reboot)?;
		self.rebooting = true;
		self.out.console("Rebooting the world");
		self.progress_start(RESTARTING, "Rebooting the world".to_owned());
		let (inputs, generation) = (self.inputs.clone(), self.debuggee()?.generation);
		std::thread::spawn(move || {
			std::thread::sleep(REBOOT_PATIENCE);
			let _ = inputs.send(Input::RebootOverdue { generation });
		});
		Ok(Value::Null)
	}

	pub(super) fn reboot_overdue(&mut self, generation: u64) {
		if self.rebooting && self.debuggee.as_ref().is_some_and(|d| d.generation == generation) {
			self.rebooting = false;
			self.progress_end(RESTARTING);
			self.out.console(format!(
				"The world hasn't rebooted after {} seconds: its world/Reboot() may be waiting, or have declined",
				REBOOT_PATIENCE.as_secs()
			));
		}
	}

	pub(super) fn relaunched(&mut self, args: LaunchArguments, prepared: Prepared) {
		if let Some(mut debuggee) = self.debuggee.take() {
			debuggee.gone();
		}
		self.joined.world_ended();
		self.pid = None;
		self.ending = false;
		self.unavailable = false;
		self.shutting_down = false;
		self.waiting_for_code = false;
		self.set.lock().unwrap().clear();
		self.stddef = None;
		if let Err(e) = self.start(args, prepared) {
			self.out.console(format!("Could not restart the world: {}", e));
			self.ended(-1);
		}
	}

	pub(super) fn game_exited(&mut self, code: Option<i32>) {
		self.game = None;
		if let Some((args, prepared)) = self.relaunching.take() {
			self.relaunched(args, prepared);
			return;
		}
		match code {
			Some(code) => self.out.console(format!("The world exited with code {}", code)),
			None => self.out.console("The world was terminated")
		}
		self.ended(code.unwrap_or(-1));
	}

	pub(super) fn terminate_overdue(&mut self, process: u32) {
		if self.ending && self.world_process() == Some(process) {
			self.out.console(format!(
				"Shutdown timed out after {} seconds; terminating process {}",
				local::test_patience(TERMINATE_GRACE).as_secs(),
				process
			));
			self.kill_world();
		}
	}

	pub(super) fn process_gone(&mut self) {
		if self.game.is_none() {
			self.out.console("The world's process has ended");
			self.ended(0);
		}
	}

	pub(super) fn ended(&mut self, exit_code: i32) {
		self.pid = None;
		self.joined.world_ended();
		if let Some(mut debuggee) = self.debuggee.take() {
			debuggee.gone();
		}
		self.code = None;
		for id in std::mem::take(&mut self.progress) {
			self.out.event(dap::ProgressEnd { progressId: id.to_owned() });
		}
		self.out.event(TerminatedEvent::default());
		self.out.event(ExitedEvent { exitCode: exit_code.into() });

		let inputs = self.inputs.clone();
		std::thread::spawn(move || {
			std::thread::sleep(EXIT_GRACE);
			let _ = inputs.send(Input::Quit);
		});
	}
}

pub fn default_args(server: Server) -> Vec<String> {
	match server {
		Server::DreamDaemonGui if cfg!(windows) => vec!["-trusted".to_owned(), "-close".to_owned()],
		_ => vec!["-trusted".to_owned()]
	}
}

pub(super) fn default_library() -> Result<PathBuf, String> {
	let name = if cfg!(windows) { "debug_server.dll" } else { "libdebug_server.so" };
	let exe = std::env::current_exe().map_err(|e| e.to_string())?;
	let library = exe.parent().map(|dir| dir.join(name)).unwrap_or_else(|| Path::new(name).to_owned());
	if !library.is_file() {
		return Err(format!("debug library not found at {} (debugLibrary)", library.display()));
	}
	Ok(library)
}
