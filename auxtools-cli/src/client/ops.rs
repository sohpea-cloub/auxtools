use auxtools_protocol::{BreakpointSetResult, HitCondition, InstructionRef, LogPart, Request, Response, Watch, WatchPlace, HELPER_FILE};

pub trait Asks {
	type Error;

	fn ask(&mut self, request: &Request) -> Result<Response, Self::Error>;

	fn unexpected(response: &Response) -> Self::Error;
}

#[derive(Clone, Default)]
pub struct Trigger {
	pub condition: Option<String>,
	pub hit: Option<HitCondition>
}

pub enum Placed {
	Set {
		at: InstructionRef,
		file: Option<String>,
		line: Option<u32>
	},
	Nowhere,
	Refused(String)
}

pub trait Operations: Asks {
	fn locate(&mut self, file: &str, line: u32) -> Result<Vec<InstructionRef>, Self::Error> {
		match self.ask(&Request::Locate { file: file.to_owned(), line })? {
			Response::Locate { instructions } => Ok(instructions),
			other => Err(Self::unexpected(&other))
		}
	}

	fn place_breakpoint(
		&mut self,
		instruction: &InstructionRef,
		trigger: &Trigger,
		log: Option<Vec<LogPart>>,
		exact: bool
	) -> Result<Placed, Self::Error> {
		let set = Request::BreakpointSet {
			instruction: instruction.clone(),
			condition: trigger.condition.clone(),
			hit: trigger.hit,
			log,
			exact
		};
		match self.ask(&set)? {
			Response::BreakpointSet { result } => Ok(match result {
				BreakpointSetResult::Success { line, at, file } => Placed::Set { at, file, line },
				BreakpointSetResult::Failed => Placed::Nowhere,
				BreakpointSetResult::Refused { reason } => Placed::Refused(reason)
			}),
			other => Err(Self::unexpected(&other))
		}
	}

	fn remove_breakpoint(&mut self, instruction: &InstructionRef, exact: bool) -> Result<u32, Self::Error> {
		let unset = Request::BreakpointUnset {
			instruction: instruction.clone(),
			exact
		};
		match self.ask(&unset)? {
			Response::BreakpointUnset { skipped, .. } => Ok(skipped),
			other => Err(Self::unexpected(&other))
		}
	}

	fn watch(&mut self, place: WatchPlace, trigger: &Trigger) -> Result<Result<Watch, String>, Self::Error> {
		let set = Request::WatchSet {
			place,
			condition: trigger.condition.clone(),
			hit: trigger.hit
		};
		match self.ask(&set)? {
			Response::WatchSet(result) => Ok(result),
			other => Err(Self::unexpected(&other))
		}
	}

	fn has_lines(&mut self) -> bool {
		match self.ask(&Request::Files) {
			Ok(Response::Files { files }) => files.iter().any(|file| file != HELPER_FILE),
			_ => true
		}
	}
}

impl<T: Asks + ?Sized> Operations for T {}

#[cfg(test)]
mod tests {
	use auxtools_protocol::{BreakpointSetResult, HitCondition, InstructionRef, ProcRef, Request, Response, HELPER_FILE};

	use super::{Asks, Operations, Placed, Trigger};

	struct Scripted {
		answers: Vec<Response>,
		asked: Vec<Request>
	}

	impl Scripted {
		fn new(mut answers: Vec<Response>) -> Self {
			answers.reverse();
			Self { answers, asked: vec![] }
		}
	}

	impl Asks for Scripted {
		type Error = String;

		fn ask(&mut self, request: &Request) -> Result<Response, String> {
			let sent = auxtools_protocol::frame(request).unwrap();
			self.asked.push(auxtools_protocol::read(&mut sent.as_slice()).unwrap());
			self.answers.pop().ok_or_else(|| "no answer".to_owned())
		}

		fn unexpected(response: &Response) -> String {
			format!("unexpected: {:?}", response)
		}
	}

	fn at(path: &str, offset: u32) -> InstructionRef {
		InstructionRef {
			proc: ProcRef {
				path: path.to_owned(),
				override_id: 0
			},
			offset
		}
	}

	#[test]
	fn a_breakpoint_is_set_as_asked_and_its_answer_read() {
		let here = at("/proc/main", 4);
		let mut world = Scripted::new(vec![
			Response::BreakpointSet {
				result: BreakpointSetResult::Success {
					line: Some(7),
					at: at("/proc/main", 2),
					file: Some("main.dm".to_owned())
				}
			},
			Response::BreakpointSet {
				result: BreakpointSetResult::Failed
			},
			Response::BreakpointSet {
				result: BreakpointSetResult::Refused {
					reason: "no such var".to_owned()
				}
			},
			Response::Ack,
		]);
		let trigger = Trigger {
			condition: Some("x > 1".to_owned()),
			hit: Some(HitCondition::parse(">= 3").unwrap())
		};
		let placed = world.place_breakpoint(&here, &trigger, None, true).unwrap();
		assert!(matches!(placed, Placed::Set { at: set, file: Some(file), line: Some(7) } if set.offset == 2 && file == "main.dm"));
		assert!(matches!(
			&world.asked[0],
			Request::BreakpointSet { instruction, condition: Some(condition), hit: Some(_), log: None, exact: true }
				if *instruction == here && condition == "x > 1"
		));
		assert!(matches!(
			world.place_breakpoint(&here, &Trigger::default(), None, false),
			Ok(Placed::Nowhere)
		));
		assert!(matches!(&world.asked[1], Request::BreakpointSet {
			condition: None,
			hit: None,
			log: None,
			exact: false,
			..
		}));
		assert!(matches!(world.place_breakpoint(&here, &trigger, None, false), Ok(Placed::Refused(why)) if why == "no such var"));
		assert_eq!(world.place_breakpoint(&here, &trigger, None, false).err().unwrap(), "unexpected: Ack");
		assert_eq!(world.remove_breakpoint(&here, true).err().unwrap(), "no answer");
	}

	#[test]
	fn a_removed_breakpoint_says_what_it_skipped() {
		let here = at("/proc/main", 4);
		let mut world = Scripted::new(vec![Response::BreakpointUnset { success: true, skipped: 3 }]);
		assert_eq!(world.remove_breakpoint(&here, false), Ok(3));
		assert!(matches!(&world.asked[0], Request::BreakpointUnset { instruction, exact: false } if *instruction == here));
	}

	#[test]
	fn a_world_has_lines_unless_it_says_it_has_only_the_helpers() {
		let files = |names: &[&str]| Response::Files {
			files: names.iter().map(|name| (*name).to_owned()).collect()
		};
		let mut world = Scripted::new(vec![files(&[HELPER_FILE, "main.dm"]), files(&[HELPER_FILE]), files(&[]), Response::Ack]);
		assert!(world.has_lines());
		assert!(!world.has_lines());
		assert!(!world.has_lines());
		assert!(world.has_lines());
		assert!(world.has_lines());
	}
}
