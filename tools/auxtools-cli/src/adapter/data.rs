//! Data breakpoints: the world pauses when a var changes. The client asks
//! what a variable or an expression is (`dataBreakpointInfo`), gets a name
//! for it, and sets its breakpoints by those names (`setDataBreakpoints`,
//! with every one it still wants, each time).

use super::*;

/// Where the records of the data breakpoints set in the world are kept
pub(super) const DATA_BREAKPOINTS: &str = "<data>";

#[derive(Default)]
pub(super) struct DataBreakpoints {
	/// The places the client has asked about, by the name each was given,
	/// with what it's called, and which stop of the world's it was asked
	/// at when it's something of that stop's (a frame, a variable shown)
	places: HashMap<String, (aux::WatchPlace, String, Option<u32>)>,
	asked: u32,
	/// The ones the client has now
	set: Vec<DataBreakpoint>
}

struct DataBreakpoint {
	data_id: String,
	condition: Option<String>,
	hit: Option<String>,
	/// What the client knows it as
	id: i64,
	/// What it watches, in words
	name: String,
	/// The world's watch, while there is one
	watch: Option<u32>,
	/// Why there isn't
	message: Option<String>
}

impl DataBreakpoint {
	fn shown(&self) -> Breakpoint {
		Breakpoint {
			id: Some(self.id),
			verified: self.watch.is_some(),
			message: self.message.clone(),
			..Default::default()
		}
	}
}

/// What `dataBreakpointInfo` takes (dap-types has none of it)
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
	/// Whether a variable (`name` in a container) or an expression can be
	/// watched, and a name to set a breakpoint on it by. The name is good
	/// for this pause: what it's of (a frame, an object the client has
	/// open) is the world's as it is now.
	pub(super) fn data_breakpoint_info(&mut self, args: DataBreakpointInfoArguments) -> Handled {
		let unwatched = |why: &str| Ok(serde_json::json!({ "dataId": null, "description": why }));
		if !self.debugging {
			return unwatched("the world runs without debugging");
		}
		if self.debuggee.is_none() {
			return unwatched("the world isn't running");
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
					// Reads aren't seen, and a write of the value that's
					// there isn't either
					"accessTypes": ["write"],
					"canPersist": false
				}))
			}
			AuxResponse::WatchInfo(Err(why)) => unwatched(&why),
			other => Err(unexpected(other))
		}
	}

	/// The client's data breakpoints, all of them: the ones it had and
	/// still has stay as they are (what they watch may be of a frame that
	/// can't be asked about any more), new ones are set, the rest go
	pub(super) fn set_data_breakpoints(&mut self, args: SetDataBreakpointsArguments) -> Handled {
		let mut had = std::mem::take(&mut self.data.set);
		let mut set = vec![];
		for wanted in args.breakpoints {
			let same = |b: &DataBreakpoint| b.data_id == wanted.data_id && b.condition == wanted.condition && b.hit == wanted.hit_condition;
			if let Some(at) = had.iter().position(same) {
				set.push(had.remove(at));
				continue;
			}

			let id = self.next_breakpoint_id;
			self.next_breakpoint_id += 1;
			let known = self.data.places.get(&wanted.data_id).cloned();
			let stop = self.debuggee.as_ref().map(Debuggee::stop_number);
			const OVER: &str = "it was something of a pause that's over, or of another world: watch it again";
			let watched: Result<aux::Watch, String> = match &known {
				_ if !self.debugging => Err("the world runs without debugging".to_owned()),
				_ if self.debuggee.is_none() => Err("the world isn't running".to_owned()),
				None => Err(OVER.to_owned()),
				Some((_, _, Some(of_stop))) if Some(*of_stop) != stop || !self.debuggee.as_ref().is_some_and(Debuggee::stopped) => Err(OVER.to_owned()),
				Some((place, ..)) => hit_condition(wanted.hit_condition.as_deref()).and_then(|hit| {
					match self.request(&AuxRequest::WatchSet {
						place: place.clone(),
						condition: wanted.condition.clone().filter(|condition| !condition.trim().is_empty()),
						hit
					})? {
						AuxResponse::WatchSet(result) => result,
						other => Err(unexpected(other))
					}
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

	/// Has a stop find the client's ids for the watches it's for
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

	/// The world says a watch is over: the proc it was in returned, the
	/// object was deleted
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

	/// The world is another one: nothing in it is watched. What was
	/// something of a pause in the old one (a frame's var, an object the
	/// client had open) can't be found again; `rewatch` sees to the rest.
	pub(super) fn watches_lost(&mut self) {
		self.data.places.retain(|_, (_, _, of_stop)| of_stop.is_none());
		for breakpoint in &mut self.data.set {
			if breakpoint.watch.take().is_some() && !self.data.places.contains_key(&breakpoint.data_id) {
				breakpoint.message = Some("the world restarted: watch it again".to_owned());
				self.out.event(dap::BreakpointEvent {
					reason: "changed".to_owned(),
					breakpoint: breakpoint.shown()
				});
			}
		}
	}

	/// Watches again, in a world that has just started, what an expression
	/// in no frame names: that means the same in any world
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
			let watched = hit_condition(breakpoint.hit.as_deref()).and_then(|hit| match self.request(&AuxRequest::WatchSet { place, condition, hit })? {
				AuxResponse::WatchSet(result) => result,
				other => Err(unexpected(other))
			});
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

/// What `setDataBreakpoints` answers
#[derive(serde::Serialize)]
struct SetDataBreakpointsResponse {
	breakpoints: Vec<Breakpoint>
}
