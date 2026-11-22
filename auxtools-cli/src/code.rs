use std::{
	cell::{Cell, RefCell},
	collections::{HashMap, HashSet},
	ffi::{OsStr, OsString},
	path::{Path, PathBuf},
	sync::{mpsc, Arc, OnceLock},
	time::{Duration, Instant, SystemTime}
};

use auxtools_cli::reload::{self, merge, refuse_hidden, refuse_other_maps};
use auxtools_protocol::{HotReloadRequest, HotReloadResource, Macro, ReloadedResource, ScriptRequest, SkinRequest};
use dmasm::reload::{diff, Changes, ParseOptions, Refusal, Snapshot, Until};

use crate::{adapter::Input, dap::Output, resources::Resources, script::Script, skin::Skin};

const INTERVAL: Duration = Duration::from_millis(500);
const SETTLE: Duration = Duration::from_millis(200);
const INTERVAL_VAR: &str = "AUXTOOLS_WATCH_INTERVAL";
const LOOKING_SHARE: u32 = 5;
const STAMP_LAG: Duration = Duration::from_millis(50);

fn interval(usual: Duration) -> Duration {
	static GIVEN: OnceLock<Option<Duration>> = OnceLock::new();
	let given = GIVEN.get_or_init(|| std::env::var(INTERVAL_VAR).ok()?.parse().ok().map(Duration::from_millis));
	given.map_or(usual, |given| given.min(usual))
}

pub struct Options {
	pub dme: PathBuf,
	pub dmb: PathBuf,
	pub defines: Vec<(String, Option<String>)>,
	pub hot_reload: bool
}

pub enum Event {
	Changed,
	Failed(String),
	Ready,
	Watching,
	Reloading,
	Looked,
	Refusals(Vec<Refusal>),
	Outdated(Vec<String>),
	Reload {
		request: Box<HotReloadRequest>,
		summary: Summary,
		notes: Vec<String>,
		world: u64
	},
	NotReloaded {
		summary: String,
		errors: Vec<String>,
		refused: Vec<Refusal>,
		warnings: Vec<String>
	},
	NothingChanged {
		warnings: Vec<String>
	},
	Macros(Vec<Macro>),
	Skin {
		request: SkinRequest,
		file: PathBuf,
		images: Vec<String>,
		restored: bool,
		world: u64
	},
	Script {
		request: ScriptRequest,
		restored: bool,
		world: u64
	}
}

pub enum Command {
	Connected { byond: (u32, u32), world: u64 },
	Reloaded(bool),
	Resources(Vec<ReloadedResource>),
	Defines(Vec<(String, Option<String>)>)
}

pub fn start(mut options: Options, out: Arc<Output>, inputs: mpsc::Sender<Input>, worker: u64) -> mpsc::Sender<Command> {
	if let Ok(dme) = std::path::absolute(&options.dme) {
		options.dme = dme;
	}
	let (commands, receiver) = mpsc::channel();
	std::thread::Builder::new()
		.name("code".to_owned())
		.spawn(move || {
			let code = Code {
				root: options.dme.parent().map(Path::to_owned).unwrap_or_default(),
				parse_options: RefCell::new(ParseOptions {
					defines: options.defines.clone(),
					byond: None
				}),
				wanted_defines: RefCell::new(None),
				defines_moved: Cell::new(false),
				running_defines: RefCell::new(options.defines.clone()),
				options,
				out,
				inputs,
				worker,
				commands: receiver,
				reconnected: Cell::new(false),
				world: Cell::new(0),
				parsed: Cell::new(false),
				parse_started: Cell::new(SystemTime::UNIX_EPOCH),
				macros: RefCell::new(None),
				resource_dirs: RefCell::new(vec![]),
				reloaded_resources: RefCell::new(vec![]),
				named_resources: RefCell::new(vec![]),
				refusals: RefCell::new(vec![]),
				reported: RefCell::new(vec![])
			};
			code.run();
		})
		.expect("couldn't start the code's thread");
	commands
}

struct Code {
	options: Options,
	parse_options: RefCell<ParseOptions>,
	wanted_defines: RefCell<Option<Vec<(String, Option<String>)>>>,
	defines_moved: Cell<bool>,
	running_defines: RefCell<Vec<(String, Option<String>)>>,
	root: PathBuf,
	out: Arc<Output>,
	inputs: mpsc::Sender<Input>,
	worker: u64,
	commands: mpsc::Receiver<Command>,
	reconnected: Cell<bool>,
	world: Cell<u64>,
	parsed: Cell<bool>,
	parse_started: Cell<SystemTime>,
	macros: RefCell<Option<Vec<Macro>>>,
	resource_dirs: RefCell<Vec<String>>,
	reloaded_resources: RefCell<Vec<ReloadedResource>>,
	named_resources: RefCell<Vec<String>>,
	refusals: RefCell<Vec<Refusal>>,
	reported: RefCell<Vec<Refusal>>
}

struct Watching {
	built: Snapshot,
	edited_since_built: HashSet<PathBuf>,
	dmb_modified: Option<SystemTime>,
	applied: Option<Snapshot>,
	files: Watched,
	skin: Skin,
	script: Script,
	resources: Resources,
	built_resources: Resources
}

impl Code {
	fn log(&self, message: impl Into<String>) {
		self.out.console(format!("Hot reload: {}", message.into()));
	}

	fn connected(&self, world: u64) {
		self.world.set(world);
		self.reconnected.set(true);
	}

	fn send(&self, event: Event) -> bool {
		self.inputs.send(Input::Code { worker: self.worker, event }).is_ok()
	}

	fn parse(&self) -> Result<Snapshot, String> {
		let defines = self.parse_options.borrow().defines.clone();
		self.parse_with(defines)
	}

	fn parse_with(&self, defines: Vec<(String, Option<String>)>) -> Result<Snapshot, String> {
		self.parsed.set(true);
		self.parse_started.set(SystemTime::now());
		let byond = self.parse_options.borrow().byond;
		Snapshot::parse(&self.options.dme, &ParseOptions { defines, byond })
	}

	fn selected_defines(&self) -> Vec<(String, Option<String>)> {
		match &*self.wanted_defines.borrow() {
			Some(defines) => defines.clone(),
			None => self.parse_options.borrow().defines.clone()
		}
	}

	fn run(self) {
		let byond = loop {
			match self.commands.recv() {
				Ok(Command::Connected { byond, world }) => {
					self.world.set(world);
					break byond;
				}
				Ok(Command::Defines(defines)) => self.defines_wanted(defines),
				Ok(Command::Reloaded(_) | Command::Resources(_)) => {}
				Err(_) => return
			}
		};
		self.parse_options.borrow_mut().byond = Some(byond);
		let dmb_modified = self.dmb_modified();

		let built = match self.parse() {
			Ok(snapshot) => snapshot,
			Err(e) => {
				self.send(Event::Failed(e));
				return;
			}
		};
		if !self.send(Event::Changed) || !self.send(Event::Ready) || !self.options.hot_reload {
			return;
		}

		self.warn_of_errors(&built);
		let files = files(&built, &self.options.dme);
		let resources = self.read_resources(&files);
		if !self.warn_of_later_edits(&files, &resources, dmb_modified) {
			return;
		}
		let script = Script::built(built.client_script());
		let mut watched = files.clone();
		watched.extend(resources.files.keys().cloned());
		let mut watching = Watching {
			files: Watched::new(&self.root, &watched, self.parse_started.get()),
			skin: Skin::built(&self.root, built.skin()),
			script,
			dmb_modified,
			edited_since_built: HashSet::new(),
			applied: None,
			built,
			built_resources: Resources {
				files: resources.files.clone(),
				dirs: resources.dirs.clone()
			},
			resources
		};
		if !self.give_macros(Some(&watching.built)) {
			return;
		}
		self.out.console("Hot reload active");
		if !self.send(Event::Watching) {
			return;
		}
		self.watch(&mut watching);
	}

	fn dmb_modified(&self) -> Option<SystemTime> {
		std::fs::metadata(&self.options.dmb).and_then(|m| m.modified()).ok()
	}

	fn watch(&self, watching: &mut Watching) {
		loop {
			let wait = interval(INTERVAL).max(watching.files.took * (LOOKING_SHARE - 1));
			match self.commands.recv_timeout(wait) {
				Ok(Command::Connected { world, .. }) => self.connected(world),
				Ok(Command::Defines(defines)) => self.defines_wanted(defines),
				Ok(Command::Reloaded(_) | Command::Resources(_)) => {}
				Err(mpsc::RecvTimeoutError::Disconnected) => return,
				Err(mpsc::RecvTimeoutError::Timeout) => {
					if !self.look(watching) {
						return;
					}
				}
			}
			while self.reconnected.take() {
				if !self.catch_up(watching) {
					return;
				}
			}
			if self.defines_moved.take() {
				let edited = watching.edited_since_built.clone();
				if !self.send(Event::Reloading) || !self.reload_code(watching, edited) || !self.send(Event::Looked) {
					return;
				}
			}
			if self.parsed.take() {
				give_back_memory();
			}
		}
	}

	fn look(&self, watching: &mut Watching) -> bool {
		let mut changed = watching.files.changed(&self.root);
		if changed.is_empty() {
			return true;
		}

		loop {
			std::thread::sleep(interval(SETTLE));
			let more = watching.files.changed(&self.root);
			if more.is_empty() {
				break;
			}
			changed.extend(more);
		}

		if !self.send(Event::Reloading) {
			return false;
		}
		watching.skin.take_edit(&mut changed);
		let resource_files: Vec<PathBuf> = changed.iter().filter(|f| watching.resources.files.contains_key(*f)).cloned().collect();
		if !resource_files.is_empty() {
			changed.retain(|f| !watching.resources.files.contains_key(f));
			if !self.reload_resources(watching, &resource_files) {
				return false;
			}
		}
		if !changed.is_empty() && !self.reload_code(watching, changed) {
			return false;
		}
		self.reload_skin(watching, false) && self.reload_script(watching, false) && self.send(Event::Looked)
	}

	fn reload_code(&self, watching: &mut Watching, changed: HashSet<PathBuf>) -> bool {
		let selected = self.selected_defines();
		let other_defines = selected != *self.running_defines.borrow();
		let held = !self.outstanding().is_empty();
		if let (false, false, [file]) = (held, other_defines, changed.iter().collect::<Vec<_>>().as_slice()) {
			let file = (*file).clone();
			let current = match watching.applied.as_mut() {
				Some(applied) => applied,
				None => &mut watching.built
			};
			match current.reload_file(&self.root, &file) {
				Ok(reload) => {
					watching.edited_since_built.insert(file);
					return match self.reload(&reload.changes, current, vec![], false, true, None) {
						None => false,
						Some(true) => {
							let current = match watching.applied.as_mut() {
								Some(applied) => applied,
								None => &mut watching.built
							};
							current.keep(reload);
							self.watch_named(watching);
							self.send(Event::Changed);
							true
						}
						Some(false) => true
					};
				}
				Err(_) => {}
			}
		}

		let new = match self.parse_with(selected.clone()) {
			Ok(new) => new,
			Err(e) => {
				self.log(format!("couldn't read the code: {}", e));
				return self.not_reloaded(e);
			}
		};
		watching.skin.parsed(new.skin());
		watching.script.parsed(new.client_script());
		let current = watching.applied.as_ref().unwrap_or(&watching.built);
		let mut changes = diff(current, &new, (!other_defines).then_some(&changed));
		refuse_hidden(current, &new, &mut changes);
		refuse_other_maps(&watching.built, &new, &mut changes);
		changes.settings_as_built(&watching.built, &new);
		watching.edited_since_built.extend(changed.iter().cloned());

		for file in files(&new, &self.options.dme) {
			watching.files.add(&self.root, &file, self.parse_started.get());
		}

		let read_with = other_defines.then_some(selected);
		match self.reload(
			&changes,
			watching.applied.as_ref().unwrap_or(&watching.built),
			vec![],
			false,
			true,
			read_with
		) {
			None => false,
			Some(true) => {
				self.watch_named(watching);
				self.send(Event::Changed);
				let given = self.give_macros(Some(&new));
				watching.applied = Some(new);
				watching.built.clear_patches();
				given
			}
			Some(false) => true
		}
	}

	fn catch_up(&self, watching: &mut Watching) -> bool {
		let dmb_modified = self.dmb_modified();
		if dmb_modified != watching.dmb_modified {
			if !self.ready() {
				return false;
			}
			self.log("using the rebuilt .dmb");
			watching.dmb_modified = dmb_modified;
			if let Some(defines) = self.wanted_defines.take() {
				self.parse_options.borrow_mut().defines = defines;
			}
			*self.running_defines.borrow_mut() = self.parse_options.borrow().defines.clone();
			watching.files.changed(&self.root);
			let parsed = self.parse();
			let read = parsed.is_ok();
			match parsed {
				Ok(built) => {
					self.warn_of_errors(&built);
					let files = files(&built, &self.options.dme);
					let resources = self.read_resources(&files);
					if !self.warn_of_later_edits(&files, &resources, dmb_modified) {
						return false;
					}
					for file in files.iter().chain(resources.files.keys()) {
						watching.files.add(&self.root, file, self.parse_started.get());
					}
					watching.skin = Skin::built(&self.root, built.skin());
					watching.script = Script::built(built.client_script());
					watching.built_resources = Resources {
						files: resources.files.clone(),
						dirs: resources.dirs.clone()
					};
					watching.resources = resources;
					watching.built = built;
					if !self.rebuilt() {
						return false;
					}
				}
				Err(e) => self.log(format!("couldn't read the code: {}", e))
			}
			watching.applied = None;
			watching.edited_since_built.clear();
			return self.send(Event::Changed) && self.give_macros(read.then_some(&watching.built));
		}

		watching.applied = None;
		*self.running_defines.borrow_mut() = self.parse_options.borrow().defines.clone();
		let selected = self.selected_defines();
		let other_defines = selected != *self.running_defines.borrow();
		watching.built.clear_patches();
		watching.skin.rebooted();
		if !self.reload_skin(watching, true) {
			return false;
		}
		watching.resources.restarted(&watching.built_resources);
		watching.script.rebooted();
		if !self.reload_script(watching, true) {
			return false;
		}
		let resource_files: Vec<PathBuf> = watching
			.resources
			.edited_since_built(&self.root)
			.into_iter()
			.map(|(path, _)| path)
			.collect();
		if watching.edited_since_built.is_empty() && !other_defines {
			return self.restore_resources(watching, &resource_files) && self.ready();
		}

		let current = match self.parse_with(selected.clone()) {
			Ok(current) => current,
			Err(e) => {
				self.log(format!("couldn't read the code: {}", e));
				return self.not_reloaded(e) && self.ready();
			}
		};
		let mut changes = diff(&watching.built, &current, (!other_defines).then_some(&watching.edited_since_built));
		refuse_hidden(&watching.built, &current, &mut changes);
		refuse_other_maps(&watching.built, &current, &mut changes);
		let read_with = other_defines.then_some(selected);
		let Some(applied) = self.reload(&changes, &watching.built, vec![], true, true, read_with) else {
			return false;
		};
		if applied {
			watching.applied = Some(current);
			watching.built.clear_patches();
			self.watch_named(watching);
		}
		if !self.restore_resources(watching, &resource_files) || !self.ready() {
			return false;
		}
		!applied || self.give_macros(watching.applied.as_ref())
	}

	fn read_resources(&self, sources: &[PathBuf]) -> Resources {
		let resources = match Resources::read(&self.root, &self.options.dmb, sources) {
			Ok(resources) => resources,
			Err(e) => {
				self.log(format!("resource watching unavailable: {}", e));
				Resources::default()
			}
		};
		*self.resource_dirs.borrow_mut() = resources.dirs.clone();
		resources
	}

	fn restore_resources(&self, watching: &mut Watching, files: &[PathBuf]) -> bool {
		if files.is_empty() {
			return true;
		}
		self.reload_resources(watching, files)
	}

	fn reload_resources(&self, watching: &mut Watching, files: &[PathBuf]) -> bool {
		let changed = watching.resources.changed(&self.root, files);
		if changed.is_empty() {
			return true;
		}
		let current = watching.applied.as_ref().unwrap_or(&watching.built);
		let mut changes = Changes::default();
		let mut resources = vec![];
		for (path, _) in &changed {
			let file = &watching.resources.files[path];
			merge(&mut changes, current.resource_users(&file.names));
			let (ids, sharing) = watching.resources.replaced(path);
			if !sharing.is_empty() {
				let sharing: Vec<String> = sharing.iter().map(|other| other.display().to_string()).collect();
				self.log(format!(
					"{} shares a resource ID with {} (same bytes); appearances are unchanged. Rebuild to separate them",
					path.display(),
					sharing.join(", ")
				));
			}
			resources.push(HotReloadResource {
				path: path.to_string_lossy().replace('\\', "/"),
				ids
			});
		}
		self.reloaded_resources.borrow_mut().clear();
		match self.reload(&changes, current, resources, false, false, None) {
			None => false,
			Some(true) => {
				let reloaded = self.reloaded_resources.take();
				for (path, crc) in &changed {
					let name = path.to_string_lossy().replace('\\', "/");
					match reloaded.iter().find(|r| r.path == name) {
						Some(r) => watching.resources.reloaded(path, r.id, *crc),
						None => watching.resources.shown(path, *crc)
					}
				}
				self.send(Event::Changed)
			}
			Some(false) => true
		}
	}

	fn watch_named(&self, watching: &mut Watching) {
		for name in self.named_resources.take() {
			if let Some(file) = watching.resources.named(&self.root, &name) {
				watching.files.add(&self.root, &file, self.parse_started.get());
			}
		}
	}

	fn warn_of_errors(&self, built: &Snapshot) {
		let errors = built.parse_errors();
		if errors.is_empty() {
			return;
		}
		self.out
			.console("The source has parse errors. Fix them and rebuild before reloading the affected code:");
		for error in errors.iter().take(5) {
			self.out
				.console_at(format!("    {}", error.description), &self.root.join(&error.file), Some(error.line));
		}
		if errors.len() > 5 {
			self.out.console(format!("    and {} more", errors.len() - 5));
		}
	}

	fn warn_of_later_edits(&self, files: &[PathBuf], resources: &Resources, built: Option<SystemTime>) -> bool {
		let mut later: Vec<String> = resources
			.edited_since_built(&self.root)
			.into_iter()
			.map(|(path, _)| path.display().to_string())
			.collect();
		if let Some(built) = built {
			later.extend(
				files
					.iter()
					.filter(|file| modified_time(&self.root, file).is_some_and(|time| time > built))
					.map(|file| file.display().to_string())
			);
		}
		if later.is_empty() {
			return true;
		}
		later.sort();
		later.dedup();
		self.send(Event::Outdated(later))
	}

	fn ready(&self) -> bool {
		while let Ok(command) = self.commands.try_recv() {
			match command {
				Command::Connected { world, .. } => self.connected(world),
				Command::Defines(defines) => self.defines_wanted(defines),
				Command::Reloaded(_) | Command::Resources(_) => {}
			}
		}
		self.reconnected.get() || self.send(Event::Ready)
	}

	fn give_macros(&self, parsed: Option<&Snapshot>) -> bool {
		let macros = match parsed.and_then(Snapshot::macro_history) {
			Some(history) => source_macros(history),
			None => {
				let options = ParseOptions {
					defines: self.running_defines.borrow().clone(),
					byond: self.parse_options.borrow().byond
				};
				match preprocessed_macros(&self.options.dme, &options) {
					Ok(macros) => macros,
					Err(e) => {
						self.log(format!("source macros unavailable for evaluation: {}", e));
						return true;
					}
				}
			}
		};
		if self.macros.borrow().as_ref() == Some(&macros) {
			return true;
		}
		*self.macros.borrow_mut() = Some(macros.clone());
		self.send(Event::Macros(macros))
	}

	fn not_reloaded(&self, error: String) -> bool {
		self.send(Event::NotReloaded {
			summary: "couldn't read the code".to_owned(),
			errors: vec![error],
			refused: vec![],
			warnings: vec![]
		})
	}

	fn reload(
		&self,
		changes: &Changes,
		before: &Snapshot,
		resources: Vec<HotReloadResource>,
		restarted: bool,
		whole: bool,
		read_with: Option<Vec<(String, Option<String>)>>
	) -> Option<bool> {
		let applies_now = |deferred: &&Refusal| restarted && deferred.until == Until::Reboot;
		let changes_nothing = changes.refused.is_empty()
			&& changes.deferred.is_empty()
			&& changes.plan.is_empty()
			&& changes.plan.removed_vars.is_empty()
			&& resources.is_empty();
		let defines = self
			.defines_refusal()
			.filter(|refusal| !applies_now(&refusal) && !(read_with.is_some() && changes_nothing));
		if !changes.refused.is_empty() || !changes.deferred.iter().all(|d| applies_now(&d)) || defines.is_some() {
			let refused = changes
				.refused
				.iter()
				.chain(&changes.deferred)
				.map(|refusal| Refusal {
					file: refusal.file.as_ref().map(|file| self.root.join(file).display().to_string()),
					..refusal.clone()
				})
				.collect::<Vec<_>>();
			let summary = if refused.iter().chain(&defines).all(|refusal| refusal.until == Until::Reboot) {
				"reboot required"
			} else {
				"rebuild required"
			};
			let event = Event::NotReloaded {
				summary: summary.to_owned(),
				errors: vec![],
				refused: refused.iter().cloned().chain(defines).collect(),
				warnings: changes.warnings.clone()
			};
			return (self.send(event) && self.refused_by(refused, whole)).then_some(false);
		}
		let mut request = reload::request(changes, before, restarted);
		*self.named_resources.borrow_mut() = reload::resource_names(&request);
		let files: Vec<String> = resources.iter().map(|r| r.path.clone()).collect();
		request.resources = resources;
		request.resource_dirs = self.resource_dirs.borrow().clone();
		let removed: Vec<String> = changes.plan.removed_vars.iter().map(|var| var.warning()).collect();
		let notes: Vec<String> = changes.warnings.iter().filter(|note| !removed.contains(note)).cloned().collect();
		if request.is_empty() && request.plan.removed_vars.is_empty() {
			for why in &notes {
				self.out.console(format!("    note: {}", why));
			}
			self.log("no changes to the running code");
			if let Some(defines) = read_with {
				*self.running_defines.borrow_mut() = defines;
			}
			let sent = self.send(Event::NothingChanged {
				warnings: changes.warnings.clone()
			});
			return (sent && self.refused_by(vec![], whole)).then_some(true);
		}

		if !self.send(Event::Reload {
			request: Box::new(request),
			summary: Summary::of(changes).with_resources(files),
			notes,
			world: self.world.get()
		}) {
			return None;
		}
		let applied = loop {
			match self.commands.recv() {
				Ok(Command::Reloaded(applied)) => break applied,
				Ok(Command::Resources(reloaded)) => *self.reloaded_resources.borrow_mut() = reloaded,
				Ok(Command::Connected { world, .. }) => self.connected(world),
				Ok(Command::Defines(defines)) => self.defines_wanted(defines),
				Err(_) => return None
			}
		};
		if applied {
			if let Some(defines) = read_with {
				*self.running_defines.borrow_mut() = defines;
			}
			if !self.refused_by(vec![], whole) {
				return None;
			}
		}
		Some(applied)
	}

	fn refused_by(&self, refused: Vec<Refusal>, whole: bool) -> bool {
		let mut now = if whole { vec![] } else { self.refusals.borrow().clone() };
		for refusal in refused {
			if !now.contains(&refusal) {
				now.push(refusal);
			}
		}
		*self.refusals.borrow_mut() = now;
		self.report()
	}

	fn report(&self) -> bool {
		let now = self.outstanding();
		if *self.reported.borrow() == now {
			return true;
		}
		*self.reported.borrow_mut() = now.clone();
		self.send(Event::Refusals(now))
	}

	fn rebuilt(&self) -> bool {
		self.refusals.borrow_mut().clear();
		self.report() && self.send(Event::Watching)
	}

	fn outstanding(&self) -> Vec<Refusal> {
		let mut outstanding = self.refusals.borrow().clone();
		outstanding.extend(self.defines_refusal());
		outstanding
	}

	fn defines_wanted(&self, defines: Vec<(String, Option<String>)>) {
		if self.wanted_defines.borrow().as_ref() != Some(&defines) {
			*self.wanted_defines.borrow_mut() = Some(defines);
			self.defines_moved.set(true);
		}
	}

	fn defines_refusal(&self) -> Option<Refusal> {
		let wanted = self.wanted_defines.borrow();
		let wanted = wanted.as_ref()?;
		let built = &*self.running_defines.borrow();
		let listed = |defines: &[(String, Option<String>)], others: &[(String, Option<String>)]| {
			defines
				.iter()
				.filter(|define| !others.contains(define))
				.map(|(name, value)| match value {
					Some(value) => format!("{}={}", name, value),
					None => name.clone()
				})
				.collect::<Vec<_>>()
		};
		let (added, removed) = (listed(wanted, built), listed(built, wanted));
		let mut changed = vec![];
		if !added.is_empty() {
			changed.push(format!("added {}", added.join(", ")));
		}
		if !removed.is_empty() {
			changed.push(format!("removed {}", removed.join(", ")));
		}
		(!changed.is_empty()).then(|| Refusal {
			until: Until::Reboot,
			..Refusal::new("", format!("build flags changed ({})", changed.join("; ")))
		})
	}

	fn reload_skin(&self, watching: &mut Watching, restored: bool) -> bool {
		let Some(change) = watching.skin.change(&self.root) else {
			return true;
		};
		if !self.send(Event::Skin {
			request: change.request.clone(),
			file: change.file.clone(),
			images: change.images.clone(),
			restored,
			world: self.world.get()
		}) {
			return false;
		}
		loop {
			match self.commands.recv() {
				Ok(Command::Reloaded(applied)) => {
					if applied {
						watching.skin.applied(change);
					}
					return true;
				}
				Ok(Command::Resources(_)) => {}
				Ok(Command::Connected { world, .. }) => self.connected(world),
				Ok(Command::Defines(defines)) => self.defines_wanted(defines),
				Err(_) => return false
			}
		}
	}

	fn reload_script(&self, watching: &mut Watching, restored: bool) -> bool {
		if let Some(why) = watching.script.problem() {
			self.log(format!("client script: {}; the world keeps the script it has", why));
		}
		let Some(change) = watching.script.change() else {
			return true;
		};
		if !self.send(Event::Script {
			request: change.request.clone(),
			restored,
			world: self.world.get()
		}) {
			return false;
		}
		loop {
			match self.commands.recv() {
				Ok(Command::Reloaded(applied)) => {
					if applied {
						watching.script.applied(change);
					}
					return true;
				}
				Ok(Command::Resources(_)) => {}
				Ok(Command::Connected { world, .. }) => self.connected(world),
				Ok(Command::Defines(defines)) => self.defines_wanted(defines),
				Err(_) => return false
			}
		}
	}
}

pub struct Summary {
	parts: Vec<(&'static str, Vec<(String, Option<String>)>, &'static str)>,
	line_moves: usize
}

impl Summary {
	fn of(changes: &Changes) -> Self {
		let named = |names: Vec<String>| names.into_iter().map(|name| (name, None)).collect();
		let var = |type_path: &str, var: &str| format!("{}/var/{}", type_path, var);
		let parts = vec![
			("added", named(changes.plan.new_types.iter().map(|t| t.path.clone()).collect()), "types"),
			(
				"reparented",
				named(
					changes
						.plan
						.reparented
						.iter()
						.map(|r| format!("{} (now under {})", r.type_path, r.parent))
						.collect()
				),
				"types"
			),
			(
				"recompiled users of",
				named(changes.consts.iter().chain(&changes.retyped).cloned().collect()),
				"vars"
			),
			(
				"reloaded",
				named(changes.plan.reloads.iter().map(|r| r.proc.to_string()).collect()),
				"procs"
			),
			(
				"updated settings for",
				named(changes.plan.settings.iter().map(|c| c.proc.to_string()).collect()),
				"procs"
			),
			(
				"added",
				named(changes.plan.new_procs.iter().map(|p| p.code.proc.to_string()).collect()),
				"procs"
			),
			(
				"unlinked",
				changes
					.plan
					.removed_procs
					.iter()
					.map(|p| (p.proc.to_string(), Some(p.proc.path.clone())))
					.collect(),
				"procs"
			),
			(
				"added",
				named(changes.plan.new_vars.iter().map(|v| var(&v.type_path, &v.var)).collect()),
				"vars"
			),
			(
				"updated declarations for",
				named(changes.plan.redeclared.iter().map(|v| var(&v.type_path, &v.var)).collect()),
				"vars"
			),
			(
				"set the default of",
				named(
					changes
						.plan
						.var_defaults
						.iter()
						.chain(&changes.plan.builtin_defaults)
						.map(|d| var(&d.types[0], &d.var))
						.collect()
				),
				"vars"
			),
			(
				"restored inherited defaults for",
				named(changes.plan.builtin_inherited.iter().map(|d| var(&d.types[0], &d.var)).collect()),
				"vars"
			),
			(
				"rebuilt the init procs of",
				named(changes.plan.init_procs.iter().map(|i| i.type_path.clone()).collect()),
				"types"
			),
		];
		Summary {
			parts,
			line_moves: changes.plan.line_moves.len()
		}
	}

	fn with_resources(mut self, files: Vec<String>) -> Self {
		if !files.is_empty() {
			self.parts
				.insert(0, ("reloaded", files.into_iter().map(|f| (f, None)).collect(), "resource files"));
		}
		self
	}

	pub fn text(&self, skipped: &[String]) -> String {
		let mut said: Vec<String> = self
			.parts
			.iter()
			.filter_map(|(done, names, what)| {
				let names: Vec<&str> = names
					.iter()
					.filter(|(_, path)| path.as_ref().is_none_or(|path| !skipped.contains(path)))
					.map(|(name, _)| name.as_str())
					.collect();
				match names.len() {
					0 => None,
					1..=5 => Some(format!("{} {}", done, names.join(", "))),
					n => Some(format!("{} {} {}", done, n, what))
				}
			})
			.collect();
		if self.line_moves > 0 {
			said.push(format!("remapped lines in {} additional procs", self.line_moves));
		}
		if said.is_empty() {
			return "no runtime changes".to_owned();
		}
		said.join("; ")
	}
}

fn files(snapshot: &Snapshot, dme: &Path) -> Vec<PathBuf> {
	let mut files = snapshot.files();
	files.push(dme.to_owned());
	files.extend(snapshot.skin().map(Path::to_owned));
	files
}

fn give_back_memory() {
	#[cfg(all(target_os = "linux", target_env = "gnu"))]
	unsafe {
		libc::malloc_trim(0);
	}
}

fn modified_time(root: &Path, file: &Path) -> Option<SystemTime> {
	std::fs::metadata(root.join(file)).and_then(|m| m.modified()).ok()
}

#[derive(Default)]
struct Watched {
	dirs: HashMap<PathBuf, HashMap<OsString, Vec<PathBuf>>>,
	times: HashMap<PathBuf, Option<SystemTime>>,
	took: Duration
}

impl Watched {
	fn new(root: &Path, files: &[PathBuf], read: SystemTime) -> Self {
		let mut watched = Self::default();
		for file in files {
			watched.insert(file);
		}
		watched.changed(root);
		for time in watched.times.values_mut() {
			*time = unless_saved_since(*time, read);
		}
		watched
	}

	fn add(&mut self, root: &Path, file: &Path, read: SystemTime) {
		let Some((dir, name)) = self.insert(file) else {
			return;
		};
		let names = &self.dirs[&dir];
		let time = listed_times(&root.join(&dir), names)
			.get(&name)
			.copied()
			.unwrap_or_else(|| modified_time(root, file));
		self.times.insert(file.to_owned(), unless_saved_since(time, read));
	}

	fn insert(&mut self, file: &Path) -> Option<(PathBuf, OsString)> {
		if self.times.contains_key(file) {
			return None;
		}
		let dir = file.parent().map(Path::to_owned).unwrap_or_default();
		let name = file.file_name().map(listed).unwrap_or_default();
		self.dirs
			.entry(dir.clone())
			.or_default()
			.entry(name.clone())
			.or_default()
			.push(file.to_owned());
		self.times.insert(file.to_owned(), None);
		Some((dir, name))
	}

	fn changed(&mut self, root: &Path) -> HashSet<PathBuf> {
		let started = Instant::now();
		let mut changed = HashSet::new();
		for (dir, names) in &self.dirs {
			let listed = listed_times(&root.join(dir), names);
			for (name, files) in names {
				for file in files {
					let time = listed.get(name).copied().unwrap_or_else(|| modified_time(root, file));
					if let Some(was) = self.times.get_mut(file).filter(|was| **was != time) {
						*was = time;
						changed.insert(file.clone());
					}
				}
			}
		}
		self.took = started.elapsed();
		changed
	}
}

fn unless_saved_since(time: Option<SystemTime>, read: SystemTime) -> Option<SystemTime> {
	time.filter(|time| *time + STAMP_LAG < read)
}

fn listed_times<'a>(dir: &Path, names: &'a HashMap<OsString, Vec<PathBuf>>) -> HashMap<&'a OsString, Option<SystemTime>> {
	let Some(entries) = cfg!(windows).then(|| std::fs::read_dir(dir).ok()).flatten() else {
		return HashMap::new();
	};
	entries
		.flatten()
		.filter(|entry| entry.file_type().is_ok_and(|kind| !kind.is_symlink()))
		.filter_map(|entry| {
			let (name, _) = names.get_key_value(&listed(&entry.file_name()))?;
			Some((name, entry.metadata().and_then(|m| m.modified()).ok()))
		})
		.collect()
}

fn listed(name: &OsStr) -> OsString {
	if cfg!(windows) {
		name.to_string_lossy().to_lowercase().into()
	} else {
		name.to_owned()
	}
}

const NOT_THE_CODES: [&str; 4] = ["DM_VERSION", "DM_BUILD", "FILE_DIR", "SPACEMAN_DMM"];

fn preprocessed_macros(dme: &Path, options: &ParseOptions) -> Result<Vec<Macro>, String> {
	let ctx = dreammaker::Context::default();
	let mut pp = dreammaker::Preprocessor::new(&ctx, dme.to_owned()).map_err(|e| format!("couldn't read {}: {}", dme.display(), e))?;
	let mut prelude = String::from("#undef SPACEMAN_DMM\n");
	if let Some((version, build)) = options.byond {
		prelude += &format!(
			"#undef DM_VERSION\n#define DM_VERSION {}\n#undef DM_BUILD\n#define DM_BUILD {}\n",
			version, build
		);
	}
	for (name, value) in &options.defines {
		prelude += &format!("#define {} {}\n", name, value.as_deref().unwrap_or(""));
	}
	pp.push_file(PathBuf::from("<auxtools defines>"), std::io::Cursor::new(prelude.into_bytes()))
		.map_err(|e| e.to_string())?;
	pp.by_ref().for_each(drop);
	Ok(source_macros(&pp.finalize()))
}

fn source_macros(history: &dreammaker::preprocessor::DefineHistory) -> Vec<Macro> {
	let mut defined: HashMap<String, (u16, Macro)> = HashMap::new();
	for (range, (name, define)) in history.iter() {
		let at_end = range.end.file == dreammaker::FileId::UNKNOWN && range.end.line == !0;
		if !at_end || define.location.file == dreammaker::FileId::BUILTINS || NOT_THE_CODES.contains(&name.as_str()) {
			continue;
		}
		if defined.get(name.as_str()).is_some_and(|(order, _)| *order > range.end.column) {
			continue;
		}
		let mut params: Vec<String> = define.params.iter().map(|param| param.to_string()).collect();
		if define.variadic {
			if let Some(last) = params.last_mut() {
				last.push_str("...");
			}
		}
		let text = auxtools_cli::macros::text_of(&define.subst);
		defined.insert(
			name.to_string(),
			(range.end.column, Macro {
				name: name.to_string(),
				params,
				text
			})
		);
	}
	let mut macros: Vec<Macro> = defined.into_values().map(|(_, define)| define).collect();
	macros.sort_by(|a, b| a.name.cmp(&b.name));
	macros
}

pub fn parse_define(define: &str) -> (String, Option<String>) {
	match define.split_once('=') {
		Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
		None => (define.to_owned(), None)
	}
}

#[cfg(test)]
mod tests {
	use std::time::{Duration, SystemTime};

	use super::*;

	struct Project {
		root: PathBuf
	}

	impl Project {
		fn new(name: &str, files: &[(&str, &str)]) -> Self {
			let root = std::env::temp_dir().join(format!("auxtools-code-{}-{}", name, std::process::id()));
			let _ = std::fs::remove_dir_all(&root);
			let project = Project { root };
			for (file, text) in files {
				project.write(file, text);
			}
			project
		}

		fn write(&self, file: &str, text: &str) {
			let path = self.root.join(file);
			std::fs::create_dir_all(path.parent().unwrap()).unwrap();
			std::fs::write(path, text).unwrap();
		}

		fn dme(&self) -> PathBuf {
			self.root.join("world.dme")
		}
	}

	impl Drop for Project {
		fn drop(&mut self) {
			let _ = std::fs::remove_dir_all(&self.root);
		}
	}

	#[test]
	fn the_parse_gives_the_macros_a_preprocessor_pass_gives() {
		let project = Project::new("macros", &[
			(
				"world.dme",
				"#define FIRST 1\n#define TWICE(x) ((x) * 2)\n#define GONE 3\n#undef GONE\n#define FIRST 2\n#define MANY(a, rest...) list(a, \
				 rest)\n#include \"code/code.dm\"\n"
			),
			(
				"code/code.dm",
				"#ifdef FLAG\n#define FLAGGED FLAG\n#endif\n#ifdef ABSENT\n#define NEVER 1\n#endif\n/proc/f()\n\treturn FIRST\n#define LAST \
				 \"text\"\n"
			)
		]);
		for byond in [None, Some((516, 1689)), Some((515, 1637))] {
			let options = ParseOptions {
				defines: vec![("FLAG".to_owned(), Some("7".to_owned())), ("BARE".to_owned(), None)],
				byond
			};
			let snapshot = Snapshot::parse(&project.dme(), &options).unwrap();
			let macros = source_macros(snapshot.macro_history().expect("the parse read all the code"));
			assert_eq!(macros, preprocessed_macros(&project.dme(), &options).unwrap());
			let texts: Vec<(&str, &str)> = macros.iter().map(|define| (define.name.as_str(), define.text.as_str())).collect();
			assert_eq!(texts, [
				("BARE", ""),
				("FIRST", "2"),
				("FLAG", "7"),
				("FLAGGED", "FLAG"),
				("LAST", "\"text\""),
				("MANY", "list(a, rest)"),
				("TWICE", "((x) * 2)")
			]);
			assert_eq!(macros[5].params, ["a", "rest..."]);
		}
	}

	#[test]
	fn a_parse_that_stopped_early_leaves_the_macros_to_a_preprocessor_pass() {
		let project = Project::new("macros-stopped", &[
			("world.dme", "#define BEFORE 1\n#include \"code/code.dm\"\n"),
			(
				"code/code.dm",
				"/proc/f()\n\treturn BEFORE\n)\n#define AFTER 2\n/proc/g()\n\treturn AFTER\n"
			)
		]);
		let options = ParseOptions::default();
		let snapshot = Snapshot::parse(&project.dme(), &options).unwrap();
		assert!(!snapshot.parse_errors().is_empty());
		assert!(snapshot.macro_history().is_none());
		let names: Vec<String> = preprocessed_macros(&project.dme(), &options)
			.unwrap()
			.into_iter()
			.map(|define| define.name)
			.collect();
		assert_eq!(names, ["AFTER", "BEFORE"]);
	}

	#[test]
	fn changed_files_are_found_by_their_directories() {
		let root = std::env::temp_dir().join(format!("auxtools-watched-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		let at = |seconds: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + seconds);
		let write = |path: &str, time: SystemTime| {
			let path = root.join(path);
			std::fs::create_dir_all(path.parent().unwrap()).unwrap();
			std::fs::write(&path, "").unwrap();
			std::fs::File::options().write(true).open(&path).unwrap().set_modified(time).unwrap();
		};
		for file in ["world.dme", "code/a.dm", "code/b.dm", "code/sub/c.dm"] {
			write(file, at(0));
		}
		let files: Vec<PathBuf> = ["world.dme", "code/a.dm", "code/b.dm", "code/sub/c.dm", "code/new.dm"]
			.iter()
			.map(PathBuf::from)
			.collect();
		let mut watched = Watched::new(&root, &files, at(1));
		let changed = |watched: &mut Watched| {
			let mut changed: Vec<String> = watched.changed(&root).iter().map(|f| f.to_string_lossy().into_owned()).collect();
			changed.sort();
			changed
		};
		assert_eq!(changed(&mut watched), Vec::<String>::new());

		write("code/b.dm", at(5));
		write("code/sub/c.dm", at(5));
		write("code/new.dm", at(5));
		assert_eq!(changed(&mut watched), ["code/b.dm", "code/new.dm", "code/sub/c.dm"]);
		assert_eq!(changed(&mut watched), Vec::<String>::new());
		std::fs::remove_file(root.join("code/a.dm")).unwrap();
		assert_eq!(changed(&mut watched), ["code/a.dm"]);

		write("code/later.dm", at(0));
		watched.add(&root, Path::new("code/later.dm"), at(1));
		assert_eq!(changed(&mut watched), Vec::<String>::new());
		write("code/later.dm", at(9));
		assert_eq!(changed(&mut watched), ["code/later.dm"]);

		if cfg!(windows) {
			watched.add(&root, Path::new("code/SUB/C.dm"), at(1));
			write("code/sub/c.dm", at(12));
			assert_eq!(changed(&mut watched), ["code/SUB/C.dm", "code/sub/c.dm"]);
		}

		write("code/b.dm", at(20));
		write("code/during.dm", at(20));
		let mut watched = Watched::new(&root, &files[..3], at(19));
		watched.add(&root, Path::new("code/during.dm"), at(19));
		assert_eq!(changed(&mut watched), ["code/b.dm", "code/during.dm"]);
		assert_eq!(changed(&mut watched), Vec::<String>::new());
		let _ = std::fs::remove_dir_all(&root);
	}
}
