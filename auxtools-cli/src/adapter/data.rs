use super::*;

pub(super) const DATA_BREAKPOINTS: &str = "<data>";

#[derive(Default)]
pub(super) struct DataBreakpoints {
	places: HashMap<String, (aux::WatchPlace, String, Option<u32>)>,
	asked: u32,
	set: Vec<DataBreakpoint>
}

struct DataBreakpoint {
	data_id: String,
	condition: Option<String>,
	hit: Option<String>,
	id: i64,
	name: String,
	watch: Option<u32>,
	message: Option<String>
}

impl DataBreakpoint {
	fn shown(&self) -> Breakpoint {
		Breakpoint {
			id: Some(self.id),
			verified: self.watch.is_some() && self.message.is_none(),
			message: self.message.clone(),
			..Default::default()
		}
	}
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DataBreakpointInfoArguments {
	variables_reference: Option<i64>,
	name: String,
	frame_id: Option<i64>
}

#[derive(Deserialize)]
pub(super) struct SetDataBreakpointsArguments {
	breakpoints: Vec<DataBreakpointWanted>
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DataBreakpointWanted {
	data_id: String,
	condition: Option<String>,
	hit_condition: Option<String>
}

impl Adapter {
	pub(super) fn data_breakpoint_info(&mut self, args: DataBreakpointInfoArguments) -> Handled {
		let unwatched = |why: &str| Ok(serde_json::json!({ "dataId": null, "description": why }));
		if !self.debugging {
			return unwatched("debugging is disabled");
		}
		if self.debuggee.is_none() {
			return unwatched(self.not_connected());
		}
		let place = match args.variables_reference.filter(|&reference| reference > 0) {
			Some(reference) => aux::WatchPlace::Variable {
				vars: aux::VariablesRef(reference as i32),
				name: args.name
			},
			None => aux::WatchPlace::Expression {
				frame_id: args.frame_id.map(|id| id as u32),
				expression: args.name
			}
		};
		match self.request(&AuxRequest::WatchInfo { place: place.clone() })? {
			AuxResponse::WatchInfo(Ok(name)) => {
				self.data.asked += 1;
				let data_id = format!("watch {}: {}", self.data.asked, name);
				let of_stop = match &place {
					aux::WatchPlace::Expression { frame_id: None, .. } => None,
					_ => self.debuggee.as_ref().map(Debuggee::stop_number)
				};
				self.data.places.insert(data_id.clone(), (place, name.clone(), of_stop));
				Ok(serde_json::json!({
					"dataId": data_id,
					"description": name,
					"accessTypes": ["write"],
					"canPersist": false
				}))
			}
			AuxResponse::WatchInfo(Err(why)) => unwatched(&why),
			other => Err(unexpected(other))
		}
	}

	pub(super) fn set_data_breakpoints(&mut self, args: SetDataBreakpointsArguments) -> Handled {
		let mut had = std::mem::take(&mut self.data.set);
		let mut set = vec![];
		for wanted in args.breakpoints {
			let same = |b: &DataBreakpoint| b.data_id == wanted.data_id && b.condition == wanted.condition && b.hit == wanted.hit_condition;
			if let Some(at) = had.iter().position(same) {
				let mut breakpoint = had.remove(at);
				if breakpoint.watch.is_some() {
					breakpoint.message = None;
				}
				set.push(breakpoint);
				continue;
			}
			let live = |b: &DataBreakpoint| b.watch.filter(|_| b.data_id == wanted.data_id);
			if let Some((at, watch)) = had.iter().enumerate().find_map(|(at, b)| live(b).map(|watch| (at, watch))) {
				let mut breakpoint = had.remove(at);
				let condition = wanted.condition.clone().filter(|condition| !condition.trim().is_empty());
				let changed = hit_condition(wanted.hit_condition.as_deref()).and_then(|hit| {
					match self.request(&AuxRequest::WatchTrigger { id: watch, condition, hit })? {
						AuxResponse::WatchSet(result) => result.map(drop),
						other => Err(unexpected(other))
					}
				});
				match changed {
					Ok(()) => {
						breakpoint.condition = wanted.condition;
						breakpoint.hit = wanted.hit_condition;
						breakpoint.message = None;
					}
					Err(why) => breakpoint.message = Some(why)
				}
				set.push(breakpoint);
				continue;
			}

			let id = self.next_breakpoint_id;
			self.next_breakpoint_id += 1;
			let known = self.data.places.get(&wanted.data_id).cloned();
			let stop = self.debuggee.as_ref().map(Debuggee::stop_number);
			const OVER: &str = "the watch target expired. Select it again";
			let watched: Result<aux::Watch, String> = match &known {
				_ if !self.debugging => Err("debugging is disabled".to_owned()),
				_ if self.debuggee.is_none() => Err(self.not_connected().to_owned()),
				None => Err(OVER.to_owned()),
				Some((_, _, Some(of_stop))) if Some(*of_stop) != stop || !self.debuggee.as_ref().is_some_and(Debuggee::stopped) => {
					Err(OVER.to_owned())
				}
				Some((place, ..)) => hit_condition(wanted.hit_condition.as_deref()).and_then(|hit| {
					let condition = wanted.condition.clone().filter(|condition| !condition.trim().is_empty());
					self.watch(place.clone(), &Trigger { condition, hit })?
				})
			};
			let name = known.map_or_else(|| wanted.data_id.clone(), |(_, name, _)| name);
			set.push(match watched {
				Ok(watch) => DataBreakpoint {
					data_id: wanted.data_id,
					condition: wanted.condition,
					hit: wanted.hit_condition,
					id,
					name: watch.name,
					watch: Some(watch.id),
					message: None
				},
				Err(why) => DataBreakpoint {
					data_id: wanted.data_id,
					condition: wanted.condition,
					hit: wanted.hit_condition,
					id,
					name,
					watch: None,
					message: Some(why)
				}
			});
		}
		for gone in had {
			if let Some(id) = gone.watch {
				let _ = self.request(&AuxRequest::WatchUnset { id });
			}
		}
		let breakpoints: Vec<Breakpoint> = set.iter().map(DataBreakpoint::shown).collect();
		self.data.set = set;
		self.record_data_breakpoints();
		json(SetDataBreakpointsResponse { breakpoints })
	}

	fn record_data_breakpoints(&mut self) {
		let records = self
			.data
			.set
			.iter()
			.filter(|breakpoint| breakpoint.watch.is_some())
			.map(|breakpoint| SetBreakpoint {
				asked: InstructionRef {
					proc: aux::ProcRef {
						path: String::new(),
						override_id: 0
					},
					offset: 0
				},
				at: None,
				watch: breakpoint.watch,
				id: breakpoint.id
			})
			.collect();
		self.set.lock().unwrap().insert(DATA_BREAKPOINTS.to_owned(), records);
	}

	pub(super) fn watch_ended(&mut self, watch: u32, reason: &str) {
		let Some(breakpoint) = self.data.set.iter_mut().find(|breakpoint| breakpoint.watch == Some(watch)) else {
			return;
		};
		breakpoint.watch = None;
		breakpoint.message = Some(reason.to_owned());
		self.out.console(format!("No longer watching {}: {}", breakpoint.name, reason));
		self.out.event(dap::BreakpointEvent {
			reason: "changed".to_owned(),
			breakpoint: breakpoint.shown()
		});
		self.record_data_breakpoints();
	}

	pub(super) fn watches_lost(&mut self) {
		self.data.places.retain(|_, (_, _, of_stop)| of_stop.is_none());
		for breakpoint in &mut self.data.set {
			if breakpoint.watch.take().is_some() && !self.data.places.contains_key(&breakpoint.data_id) {
				breakpoint.message = Some("the watch ended when the world rebooted. Select it again".to_owned());
				self.out.event(dap::BreakpointEvent {
					reason: "changed".to_owned(),
					breakpoint: breakpoint.shown()
				});
			}
		}
	}

	pub(super) fn rewatch(&mut self) {
		for at in 0..self.data.set.len() {
			let breakpoint = &self.data.set[at];
			let Some((place, ..)) = self.data.places.get(&breakpoint.data_id).cloned() else {
				continue;
			};
			if breakpoint.watch.is_some() || breakpoint.message.is_some() {
				continue;
			}
			let condition = breakpoint.condition.clone().filter(|condition| !condition.trim().is_empty());
			let watched = hit_condition(breakpoint.hit.as_deref()).and_then(|hit| self.watch(place, &Trigger { condition, hit })?);
			let breakpoint = &mut self.data.set[at];
			match watched {
				Ok(watch) => breakpoint.watch = Some(watch.id),
				Err(why) => {
					breakpoint.message = Some(why);
					self.out.event(dap::BreakpointEvent {
						reason: "changed".to_owned(),
						breakpoint: breakpoint.shown()
					});
				}
			}
		}
		self.record_data_breakpoints();
	}
}

#[derive(serde::Serialize)]
struct SetDataBreakpointsResponse {
	breakpoints: Vec<Breakpoint>
}
