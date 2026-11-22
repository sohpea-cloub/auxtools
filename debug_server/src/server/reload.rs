use super::*;

pub(super) struct Reloads {
	pub(super) pending_skin: Option<SkinRequest>,
	pub(super) pending_script: Option<ScriptRequest>,
	pub(super) resource_ids: HashMap<String, Vec<u32>>,
	pub(super) resource_origins: HashMap<String, Vec<u32>>,
	pub(super) waiting_reloads: Vec<(Option<ClientId>, HotReloadRequest)>
}

impl Reloads {
	pub(super) fn new() -> Self {
		Self {
			pending_skin: None,
			pending_script: None,
			resource_ids: HashMap::new(),
			resource_origins: HashMap::new(),
			waiting_reloads: vec![]
		}
	}

	pub(super) fn record_named_files(&mut self, request: &HotReloadRequest) {
		use dmasm::assembler::AssembleEnv as _;
		for name in crate::world_resources::not_built(request) {
			let Some(file) = crate::assemble_env::resource_file(&name) else {
				continue;
			};
			let path = file.to_string_lossy().replace('\\', "/");
			if self.resource_ids.contains_key(&path) || crate::world_resources::built_file(&file) {
				continue;
			}
			if let Some(id) = crate::assemble_env::AssembleEnv.get_resource_index(&name) {
				self.resource_ids.insert(path, vec![id]);
			}
		}
	}

	pub(super) fn record_resource_ids(&mut self, reloaded: &[ReloadedResource], warnings: &mut Vec<String>) {
		let built = crate::world_resources::built_count();
		for resource in reloaded {
			let own = self.resource_origins.get(&resource.path).is_some_and(|ids| ids.contains(&resource.id));
			let shared = (!own && built.is_some_and(|count| (resource.id as usize) < count))
				|| self
					.resource_ids
					.iter()
					.any(|(path, ids)| *path != resource.path && ids.contains(&resource.id));
			if shared {
				self.resource_ids.remove(&resource.path);
				warnings.push(format!(
					"resource reload: {} shares existing resource {} (same bytes); later edits to {} will leave appearances using that shared ID \
					 unchanged",
					resource.path, resource.id, resource.path
				));
			} else {
				let ids = self.resource_ids.entry(resource.path.clone()).or_default();
				ids.retain(|&id| id != resource.id);
				ids.push(resource.id);
			}
		}
	}
}

impl Server {
	fn apply_pending_skin(&mut self) {
		let Some(request) = self.reloads.pending_skin.take() else {
			return;
		};
		if let Err(e) = self.evaluating(|| crate::skin::set(&request)) {
			self.notify(format!("could not restore skin {} after reboot: {}", request.path, e));
		}
	}

	fn apply_pending_script(&mut self) {
		let Some(request) = self.reloads.pending_script.take() else {
			return;
		};
		if let Err(e) = self.evaluating(|| crate::client_script::set(&request)) {
			self.notify(format!("could not restore client script after reboot: {}", e));
		}
	}

	pub(super) fn handle_hot_reload(&mut self, request: &HotReloadRequest) {
		if let Some(why) = self.sessions.reloads_refused() {
			let refused = HotReloadResult {
				applied: false,
				errors: vec![why],
				warnings: vec![],
				skipped: vec![],
				resources: vec![]
			};
			self.answer(Response::HotReload(refused));
			return;
		}
		let result = self.hot_reload(request, self.sessions.requester());
		self.answer(Response::HotReload(result));
	}

	fn hot_reload(&mut self, request: &HotReloadRequest, from: Option<ClientId>) -> HotReloadResult {
		resource_reload::use_dirs(&request.resource_dirs);
		if !crate::world_resources::is_loaded() {
			let mut files = crate::world_resources::not_built(request);
			for resource in &request.resources {
				if !files.contains(&resource.path) {
					files.push(resource.path.clone());
				}
			}
			if !files.is_empty() || !self.reloads.waiting_reloads.is_empty() {
				let mut waiting = request.clone();
				let mut early = vec![];
				if let Some(init) = &request.global_init {
					let (globals, others) = request.plan.new_vars.iter().cloned().partition(|var| var.type_path.is_empty());
					let (init, left) = crate::hot_reload::early_global_init(init, request);
					if left > 0 {
						early.push(format!(
							"(global init): {} new static vars will initialize with the deferred reload at world/New",
							left
						));
					}
					let alone = HotReloadRequest {
						global_init: Some(init),
						plan: dmasm::reload::Plan {
							new_vars: globals,
							..Default::default()
						},
						..Default::default()
					};
					if crate::world_resources::not_built(&alone).is_empty() {
						let result = self.apply_hot_reload(&alone, from);
						if result.applied {
							waiting.global_init = None;
							waiting.plan.new_vars = others;
						}
						early.extend(result.errors.into_iter().chain(result.warnings));
					}
				}
				self.reloads.waiting_reloads.push((from, waiting));
				let why = if files.is_empty() {
					"queued behind an earlier reload; applies as world/New starts".to_owned()
				} else {
					format!(
						"resource files need the loaded cache ({}); reload applies as world/New starts. Map atoms use the built code until then",
						files.join(", ")
					)
				};
				return HotReloadResult {
					applied: true,
					errors: vec![],
					warnings: std::iter::once(why).chain(early).collect(),
					skipped: vec![],
					resources: vec![]
				};
			}
		}
		self.apply_hot_reload(request, from)
	}

	fn apply_hot_reload(&mut self, request: &HotReloadRequest, from: Option<ClientId>) -> HotReloadResult {
		let mut id_warnings = vec![];
		let mut with_ids;
		let request = if request.resources.is_empty() {
			request
		} else {
			with_ids = request.clone();
			let named: Vec<(String, Vec<u32>)> = request.resources.iter().map(|r| (r.path.clone(), r.ids.clone())).collect();
			for (path, ids) in &named {
				if !self.reloads.resource_ids.contains_key(path) {
					self.reloads.resource_origins.entry(path.clone()).or_insert_with(|| ids.clone());
				}
			}
			for resource in &mut with_ids.resources {
				if let Some(&last) = self.reloads.resource_ids.get(&resource.path).and_then(|ids| ids.last()) {
					resource.ids = vec![last];
				}
				let path = resource.path.clone();
				let owner = |id: u32| {
					self.reloads
						.resource_ids
						.iter()
						.chain(named.iter().map(|(p, ids)| (p, ids)))
						.find(|(p, ids)| **p != path && ids.contains(&id))
						.map(|(p, _)| p.clone())
				};
				let mut kept = vec![];
				for &id in &resource.ids {
					match owner(id) {
						Some(other) => id_warnings.push(format!(
							"resource reload: {} and {} share an ID (same bytes); appearances are unchanged. Rebuild to separate them",
							path, other
						)),
						None => kept.push(id)
					}
				}
				resource.ids = kept;
			}
			&with_ids
		};
		self.eval.in_eval = true;
		auxtools::instruction_hook::hear_all();
		self.eval.eval_error = None;

		let mut warnings = id_warnings;
		let mut skipped = vec![];
		let prepared = Self::prepare_hot_reload(request, &mut warnings, &mut skipped);
		let result = match prepared {
			Ok((appended, applied)) => {
				hot_reload::apply(appended, &mut self.breakpoints.claims, &mut self.breakpoints.moved, &mut warnings);
				warnings.extend(applied.warnings);
				if let Err(e) = resource_reload::resend(&applied.appearances) {
					warnings.push(format!("resource reload: could not send updated appearances to clients: {}", e));
				}
				self.reloads.record_resource_ids(&applied.reloaded, &mut warnings);
				self.reloads.record_named_files(request);
				HotReloadResult {
					applied: true,
					errors: vec![],
					warnings,
					skipped,
					resources: applied.reloaded
				}
			}

			Err(errors) => HotReloadResult {
				applied: false,
				errors,
				warnings,
				skipped,
				resources: vec![]
			}
		};

		self.eval.in_eval = false;
		if let Some(err) = self.eval.eval_error.take() {
			self.notify(format!("runtime error during hot reload: {}", err));
		}

		if result.applied && !request.is_empty() {
			self.broadcast(Response::CodeChanged, from);
		}

		result
	}

	fn prepare_hot_reload(
		request: &HotReloadRequest,
		warnings: &mut Vec<String>,
		skipped: &mut Vec<String>
	) -> Result<(hot_reload::Appended, resource_reload::Applied), Vec<String>> {
		let added = hot_reload::add_types(&request.plan.new_types)?;
		let unused_types = |mut errors: Vec<String>| {
			if added > 0 {
				errors.push(format!("{} new types remain allocated but unused", added));
			}
			errors
		};
		let prepared = hot_reload::prepare(request, warnings, skipped).map_err(unused_types)?;
		let resources = resource_reload::prepare(&request.resources).map_err(unused_types)?;
		let appended = hot_reload::append(prepared).map_err(unused_types)?;
		match resource_reload::apply(&resources) {
			Ok(applied) => Ok((appended, applied)),
			Err(e) => Err(unused_types(std::iter::once(e).chain(appended.unused()).collect()))
		}
	}

	pub fn world_data_loaded(&mut self) {
		self.apply_pending_script();
		for (from, request) in std::mem::take(&mut self.reloads.waiting_reloads) {
			let result = self.hot_reload(&request, from);
			if !result.applied {
				self.notify(format!("Hot reload failed as world/New started: {}", result.errors.join("; ")));
			}
			for warning in result.warnings {
				self.notify(format!("Hot reload: {}", warning));
			}
		}
		self.apply_pending_skin();
	}

	pub(super) fn handle_skin(&mut self, request: SkinRequest) {
		let result = match self.sessions.reloads_refused() {
			Some(why) => Err(why),
			None if !crate::world_resources::is_loaded() => {
				self.reloads.pending_skin = Some(request);
				Ok(SkinReloaded { clients: 0, hosted: false })
			}
			None => self.evaluating(|| crate::skin::set(&request))
		};
		self.answer(Response::Skin(result));
	}

	pub(super) fn handle_script(&mut self, request: ScriptRequest) {
		let result = match self.sessions.reloads_refused() {
			Some(why) => Err(why),
			None if !crate::world_resources::is_loaded() => {
				self.reloads.pending_script = Some(request);
				Ok(ScriptReloaded { clients: 0, hosted: false })
			}
			None => self.evaluating(|| crate::client_script::set(&request))
		};
		self.answer(Response::Script(result));
	}
}
