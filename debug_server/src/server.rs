use std::{
	cell::RefCell,
	collections::{BTreeMap, HashMap},
	error::Error,
	io::{Read, Write},
	sync::{mpsc, Mutex},
	thread
};

use auxtools::{
	raw_types::values::{ValueData, ValueTag},
	*
};
use clap::{Arg, Command};
use instruction_hooking::disassemble_env;

use super::{
	instruction_hooking::{hook_instruction, original_bytecode, unhook_all, unhook_instruction},
	server_types::*
};
use crate::{
	connection::{self, Connection},
	hot_reload, mem_profiler
};

#[derive(Clone, Hash, PartialEq, Eq)]
enum Variables {
	Arguments { frame: u32 },
	Locals { frame: u32 },
	ObjectVars(Value),
	ListContents(Value),
	ListPair { key: Value, value: Value }
}

struct State {
	stacks: debug::CallStacks,
	variables: RefCell<Vec<Variables>>,
	variables_to_refs: RefCell<HashMap<Variables, VariablesRef>>
}

impl State {
	fn new() -> Self {
		Self {
			stacks: debug::CallStacks::new(),
			variables: RefCell::new(vec![]),
			variables_to_refs: RefCell::new(HashMap::new())
		}
	}

	fn invalidate_stacks(&mut self) {
		self.stacks = debug::CallStacks::new();
	}

	fn get_ref(&self, vars: Variables) -> VariablesRef {
		let mut variables_to_refs = self.variables_to_refs.borrow_mut();
		let mut variables = self.variables.borrow_mut();
		(*variables_to_refs.entry(vars.clone()).or_insert_with(|| {
			let reference = VariablesRef(variables.len() as i32 + 1);
			variables.push(vars);
			reference
		}))
		.clone()
	}

	fn get_variables(&self, reference: VariablesRef) -> Option<Variables> {
		let variables = self.variables.borrow();
		variables.get(reference.0 as usize - 1).map(|x| (*x).clone())
	}
}

// Server = main-thread code
// ServerThread = networking-thread code
//
// The ServerThreads tell the Server everything through one channel of Events,
// so they arrive in the order they happened.
//
// The world listens on a local socket named after its process, and takes as
// many debug-clients as connect: the launcher that started it, launchers
// attached to it, a hot reload watcher. Each has its own breakpoints and its
// own say on pausing for runtime errors. A pause stops the world for all of
// them, and any of them can let it carry on.
//

type ClientId = u32;

enum Event {
	// A debug-client arrived. The Connection is for the Server to answer on.
	Connected(ClientId, Connection),

	// A request from a debug-client for the Server to handle
	Request(ClientId, Request),

	// The debug-client left, or its connection broke
	Disconnected(ClientId)
}

struct Client {
	connection: Connection,
	// Whether it wants the world to pause on runtime errors
	catch_runtimes: bool
}

/// The breakpoints, by where they are, and the clients that set each: with a
/// condition, or `None` to stop every time
pub type Breakpoints = HashMap<(raw_types::procs::ProcId, u16), BTreeMap<ClientId, Option<String>>>;

/// How long to wait for a request
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wait {
	No,
	/// While there's a client to send one
	WhileClients,
	/// For as long as it takes, clients coming and going
	Forever
}

/// The world's one listener, which lasts as long as the process. A world
/// reboot puts its Server away, and clients that connect before the next one
/// is made wait for it.
struct Acceptor {
	/// Where the listener's thread sends what it hears
	events: mpsc::Sender<Event>,
	/// The other end, for the next Server to take
	waiting: Option<mpsc::Receiver<Event>>
}

static ACCEPTOR: Mutex<Option<Acceptor>> = Mutex::new(None);

// Nothing a debug-client sends is anywhere near this big. Without a limit, any
// stray bytes on the socket get read as a length and we try to allocate
// it. A hot reload of every proc that uses a changed #define can get big.
const MAX_REQUEST_LEN: u32 = 256 * 1024 * 1024;

pub struct Server {
	events: mpsc::Receiver<Event>,
	clients: BTreeMap<ClientId, Client>,
	// The client whose request is being answered
	current: Option<ClientId>,
	state: Option<State>,
	in_eval: bool,
	eval_error: Option<String>,
	breakpoints: Breakpoints,
	moved_breakpoints: hot_reload::MovedBreakpoints,
	app: Command<'static>
}

struct ServerThread {
	events: mpsc::Sender<Event>
}

impl Server {
	pub fn setup_app() -> Command<'static> {
		Command::new("Auxtools Debug Server")
			.version(env!("CARGO_PKG_VERSION"))
			.subcommand_required(true)
			.no_binary_name(true)
			.color(clap::ColorChoice::Never)
			.disable_version_flag(true)
			.disable_help_flag(true)
			.override_usage("#<SUBCOMMAND>")
			.subcommand(
				Command::new("disassemble")
					.alias("dis")
					.about("Disassembles a proc and displays its bytecode in an assembly-like format")
					.after_help("If no parameters are provided, the proc executing in the currently debugged stack frame will be disassembled")
					.arg(
						Arg::new("proc")
							.help("Path of the proc to disassemble (e.g. /proc/do_stuff)")
							.takes_value(true)
					)
					.arg(
						Arg::new("id")
							.help("Id of the proc to disassemble (for when multiple procs are defined with the same path)")
							.takes_value(true)
					)
			)
			.subcommand(
				Command::new("guest_override")
					.about("Override the CKey used by guest connections")
					.arg(Arg::new("ckey").takes_value(true))
			)
			.subcommand(
				Command::new("mem_profiler")
					.about("Memory profiler")
					.subcommand(
						Command::new("begin")
							.about("Begins memory profiling. Output goes to the specified file path")
							.arg(Arg::new("path").help("Where to output memory profiler results").takes_value(true))
					)
					.subcommand(Command::new("end").about("Finishes current memory profiler."))
			)
	}

	/// Takes the clients from the world's local socket, which it starts
	/// listening on the first time
	pub fn listen() -> std::io::Result<Server> {
		let events = Self::take_events()?;
		Ok(Server {
			events,
			clients: BTreeMap::new(),
			current: None,
			state: None,
			in_eval: false,
			eval_error: None,
			breakpoints: HashMap::new(),
			moved_breakpoints: HashMap::new(),
			app: Self::setup_app()
		})
	}

	/// What the listener hears, for one Server at a time
	fn take_events() -> std::io::Result<mpsc::Receiver<Event>> {
		let mut acceptor = ACCEPTOR.lock().unwrap();
		if acceptor.is_none() {
			let listener = connection::listen(&connection::world_name())?;
			let (events, waiting) = mpsc::channel();
			*acceptor = Some(Acceptor {
				events,
				waiting: Some(waiting)
			});
			thread::Builder::new()
				.name("debug server listener".to_owned())
				.spawn(move || Self::accept(&listener))?;
		}
		acceptor
			.as_mut()
			.and_then(|acceptor| acceptor.waiting.take())
			.ok_or_else(|| std::io::Error::other("there's a debug server already"))
	}

	/// The listener's thread
	fn accept(listener: &connection::Listener) {
		let mut next_id = 0;
		loop {
			let Ok(connection) = listener.accept() else {
				continue;
			};
			next_id += 1;

			// Under the lock, so it goes to the Server that's there now, or
			// the next one if there's none
			let events = {
				let acceptor = ACCEPTOR.lock().unwrap();
				let Some(acceptor) = acceptor.as_ref() else {
					return;
				};
				let _ = acceptor.events.send(Event::Connected(next_id, connection.clone()));
				acceptor.events.clone()
			};

			// It reads until the client hangs up, which it does when told to
			// disconnect, so nothing waits for it
			let server_thread = ServerThread { events };
			let id = next_id;
			thread::spawn(move || server_thread.run(id, connection));
		}
	}

	/// Tells whoever connects that there's no debugger, and why
	pub fn refuse(message: String) {
		let Ok(events) = Self::take_events() else {
			return;
		};
		thread::spawn(move || {
			while let Ok(event) = events.recv() {
				if let Event::Connected(_, mut connection) = event {
					for response in [Response::Notification { message: message.clone() }, Response::Disconnect] {
						if Self::send(&mut connection, &response).is_err() {
							break;
						}
					}
				}
			}
		});
	}

	pub const fn is_in_eval(&self) -> bool {
		self.in_eval
	}

	pub fn set_eval_error(&mut self, err: String) {
		self.eval_error = Some(err);
	}

	fn get_line_number(&self, proc: ProcRef, offset: u32) -> Option<u32> {
		match auxtools::Proc::find_override(proc.path, proc.override_id) {
			Some(proc) => Self::line_number_in(unsafe { proc.bytecode() }, offset),
			None => None
		}
	}

	/// The line a frame is on. A frame that started before a hot reload runs
	/// the code it started with, not the proc's current code.
	fn frame_line_number(&self, frame: &debug::StackFrame) -> Option<u32> {
		let running = unsafe { *(*frame.context).bytecode() };
		let (current, _) = unsafe { frame.proc.bytecode_mut_ptr() };

		if running != current {
			if let Some(bytecode) = auxtools::bytecode_at(running) {
				return Self::line_number_in(&bytecode, frame.offset as u32);
			}
		}

		self.get_line_number(
			ProcRef {
				path: frame.proc.path.clone(),
				override_id: frame.proc.override_id()
			},
			frame.offset as u32
		)
	}

	fn line_number_in(bytecode: &[u32], offset: u32) -> Option<u32> {
		let mut current_line_number = None;
		let mut reached_offset = false;

		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(bytecode, &mut env);

		for node in nodes {
			if let dmasm::Node::Instruction(ins, debug) = node {
				if debug.offset > offset {
					reached_offset = true;
					break;
				}

				if let dmasm::Instruction::DbgLine(line) = ins {
					current_line_number = Some(line);
				}

				if debug.offset == offset {
					reached_offset = true;
					break;
				}
			}
		}

		if reached_offset {
			current_line_number
		} else {
			None
		}
	}

	fn get_offset(&self, proc: ProcRef, line: u32) -> Option<u32> {
		let proc = auxtools::Proc::find_override(proc.path, proc.override_id)?;
		let mut offset = None;
		let mut at_offset = false;

		let bytecode = unsafe { proc.bytecode() };

		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(bytecode, &mut env);

		for node in nodes {
			if let dmasm::Node::Instruction(ins, debug) = node {
				if at_offset {
					offset = Some(debug.offset);
					break;
				}

				if let dmasm::Instruction::DbgLine(current_line) = ins {
					if current_line == line {
						at_offset = true;
					}
				}
			}
		}

		offset
	}

	// 516 primitives have no `vars` list, so their fields are spelled out here.
	// a callee only reads while its proc is still running
	const fn primitive_fields(value: &Value) -> Option<&'static [&'static str]> {
		match value.raw.tag {
			ValueTag::Vector => Some(&["x", "y", "z", "len", "size"]),
			ValueTag::PixLoc => Some(&["x", "y", "z", "step_x", "step_y", "loc"]),
			ValueTag::Callee => Some(&["proc", "file", "line", "src", "usr", "args", "caller"]),
			_ => None
		}
	}

	fn is_object(value: &Value) -> bool {
		// Hack for globals
		if value.raw.tag == ValueTag::World && unsafe { value.raw.data.id == 1 } {
			return true;
		}

		Self::primitive_fields(value).is_some() || value.get(byond_string!("vars")).is_ok()
	}

	fn stringify(value: &Value) -> String {
		if List::is_list(value) {
			match List::from_value(value) {
				Ok(list) => format!("/{} {{len = {}}}", if list.is_alist() { "alist" } else { "list" }, list.len()),
				Err(Runtime { message }) => format!("/list (failed to get len: {:?})", message)
			}
		} else {
			match value.to_string() {
				Ok(v) if v.is_empty() => value.raw.to_string(),
				Ok(value) => value,
				Err(Runtime { message }) => {
					format!("{} -- stringify error: {:?}", value.raw, message)
				}
			}
		}
	}

	fn value_to_variable(&self, name: String, value: &Value) -> Variable {
		let stringified = Self::stringify(value);
		let variables = self.value_to_variables_ref(value);

		Variable {
			name,
			value: stringified,
			variables
		}
	}

	fn value_to_variables_ref(&self, value: &Value) -> Option<VariablesRef> {
		match self.state.as_ref() {
			Some(state) if List::is_list(value) => Some(state.get_ref(Variables::ListContents(value.clone()))),

			Some(state) if Self::is_object(value) => Some(state.get_ref(Variables::ObjectVars(value.clone()))),

			_ => None
		}
	}

	fn list_to_variables(&mut self, value: &Value) -> Result<Vec<Variable>, Runtime> {
		let state = self.state.as_ref().unwrap();
		let list = List::from_value(value)?;

		// numbers are keys in an alist, so the position walk below would read
		// the wrong entries
		if list.is_alist() {
			return Ok(list
				.alist_pairs()?
				.into_iter()
				.enumerate()
				.map(|(i, (key, value))| Variable {
					name: format!("[{}]", i + 1),
					value: format!("{} = {}", Self::stringify(&key), Self::stringify(&value)),
					variables: Some(state.get_ref(Variables::ListPair { key, value }))
				})
				.collect());
		}

		let len = list.len();

		let mut variables = vec![];

		for i in 1..=len {
			let key = list.get(i)?;

			// assoc entry
			if key.raw.tag != raw_types::values::ValueTag::Number {
				if let Ok(value) = list.get(&key) {
					if value.raw.tag != raw_types::values::ValueTag::Null {
						variables.push(Variable {
							name: format!("[{}]", i),
							value: format!("{} = {}", Self::stringify(&key), Self::stringify(&value)),
							variables: Some(state.get_ref(Variables::ListPair { key, value }))
						});

						continue;
					}
				}
			}

			// non-assoc entry
			variables.push(self.value_to_variable(format!("[{}]", i), &key));
		}

		Ok(variables)
	}

	fn object_to_variables(&mut self, value: &Value) -> Result<Vec<Variable>, Runtime> {
		if let Some(fields) = Self::primitive_fields(value) {
			return fields
				.iter()
				.map(|&name| Ok(self.value_to_variable(name.to_owned(), &value.get(StringRef::new(name)?)?)))
				.collect();
		}

		// Grab `value.vars`. We have a little hack for globals which use a
		// special type.
		let vars = List::from_value(&unsafe {
			if value.raw.tag == ValueTag::World && value.raw.data.id == 1 {
				Value::new(ValueTag::GlobalVars, ValueData { id: 0 })
			} else {
				value.get(byond_string!("vars"))?
			}
		})?;

		let mut variables = vec![];
		let mut top_variables = vec![]; // These fields get displayed on top of
										// all others

		for i in 1..=vars.len() {
			let name = vars.get(i)?.as_string()?;
			let value = value.get(StringRef::new(name.as_str())?)?;
			let variable = self.value_to_variable(name, &value);
			if variable.name == "type" {
				top_variables.push(variable);
			} else {
				variables.push(variable);
			}
		}

		// top_variables.sort_by_key(|a| a.name.to_lowercase());
		variables.sort_by_key(|a| a.name.to_lowercase());
		top_variables.append(&mut variables);

		Ok(top_variables)
	}

	fn get_stack(&self, stack_id: u32) -> Option<&Vec<debug::StackFrame>> {
		let stack_id = stack_id as usize;
		let stacks = &self.state.as_ref()?.stacks;

		if stack_id == 0 {
			return Some(&stacks.active);
		}

		stacks.suspended.get(stack_id - 1)
	}

	fn get_stack_base_frame_id(&self, stack_id: u32) -> u32 {
		let stack_id = stack_id as usize;
		let stacks = match &self.state {
			Some(state) => &state.stacks,
			None => return 0
		};

		if stack_id == 0 {
			return 0;
		}

		let mut current_base = stacks.active.len();

		for frame in &stacks.suspended[..stack_id - 1] {
			current_base += frame.len();
		}

		current_base as u32
	}

	fn get_stack_frame(&self, frame_index: u32) -> Option<&debug::StackFrame> {
		let mut frame_index = frame_index as usize;
		let stacks = &self.state.as_ref()?.stacks;

		if frame_index < stacks.active.len() {
			return Some(&stacks.active[frame_index]);
		}

		frame_index -= stacks.active.len();

		for frame in &stacks.suspended {
			if frame_index < frame.len() {
				return Some(&frame[frame_index]);
			}

			frame_index -= frame.len();
		}

		None
	}

	fn get_args(&mut self, frame_index: u32) -> Vec<Variable> {
		match self.get_stack_frame(frame_index) {
			Some(frame) => {
				let mut vars = vec![
					self.value_to_variable("src".to_owned(), &frame.src),
					self.value_to_variable("usr".to_owned(), &frame.usr),
				];

				let mut unnamed_count = 0;
				for (name, value) in &frame.args {
					let name = match name {
						Some(name) => String::from(name),
						None => {
							unnamed_count += 1;
							format!("undefined argument #{}", unnamed_count)
						}
					};
					vars.push(self.value_to_variable(name, value));
				}

				vars
			}

			None => {
				self.notify(format!("tried to read arguments from invalid frame id: {}", frame_index));
				vec![]
			}
		}
	}

	fn get_locals(&mut self, frame_index: u32) -> Vec<Variable> {
		match self.get_stack_frame(frame_index) {
			Some(frame) => {
				let mut vars = vec![self.value_to_variable(".".to_owned(), &frame.dot)];

				for (name, local) in &frame.locals {
					vars.push(self.value_to_variable(String::from(name), local));
				}

				vars
			}

			None => {
				self.notify(format!("tried to read locals from invalid frame id: {}", frame_index));
				vec![]
			}
		}
	}

	fn handle_breakpoint_set(&mut self, instruction: InstructionRef, condition: Option<String>) {
		let line = self.get_line_number(instruction.proc.clone(), instruction.offset);

		let (Some(proc), Some(client)) = (
			auxtools::Proc::find_override(instruction.proc.path, instruction.proc.override_id),
			self.current
		) else {
			self.send_or_disconnect(Response::BreakpointSet {
				result: BreakpointSetResult::Failed
			});
			return;
		};

		match hook_instruction(&proc, instruction.offset) {
			Ok(()) => {
				// The client re-sends breakpoints it already set, and no
				// condition this time means the old one is gone
				let key = (proc.id, instruction.offset as u16);
				self.breakpoints.entry(key).or_default().insert(client, condition);

				// The client has found out where a reload moved a breakpoint to
				if !self.breakpoints.values().any(|claims| claims.len() > 1) {
					self.moved_breakpoints.retain(|(id, _), at| *id != proc.id || *at != instruction.offset);
				}

				self.send_or_disconnect(Response::BreakpointSet {
					result: BreakpointSetResult::Success { line }
				});
			}

			Err(_) => {
				self.send_or_disconnect(Response::BreakpointSet {
					result: BreakpointSetResult::Failed
				});
			}
		}
	}

	fn handle_breakpoint_unset(&mut self, instruction: InstructionRef) {
		let (Some(proc), Some(client)) = (
			auxtools::Proc::find_override(instruction.proc.path, instruction.proc.override_id),
			self.current
		) else {
			self.send_or_disconnect(Response::BreakpointUnset { success: false });
			return;
		};

		// It might be one a reload moved, known to the client by where it was
		let offset = self
			.moved_breakpoints
			.get(&(proc.id, instruction.offset))
			.copied()
			.unwrap_or(instruction.offset);

		let key = (proc.id, offset as u16);
		let claims = self.breakpoints.entry(key).or_default();
		let had = claims.remove(&client).is_some();
		let success = if claims.is_empty() {
			// Nobody else wants it
			self.breakpoints.remove(&key);
			self.moved_breakpoints.remove(&(proc.id, instruction.offset));
			unhook_instruction(&proc, offset).is_ok()
		} else {
			had
		};
		self.send_or_disconnect(Response::BreakpointUnset { success });
	}

	fn handle_stacks(&mut self) {
		let stacks = match &self.state {
			Some(state) => {
				let mut ret = vec![];
				ret.push(Stack {
					id: 0,
					name: state.stacks.active[0].proc.path.clone()
				});

				for (idx, stack) in state.stacks.suspended.iter().enumerate() {
					ret.push(Stack {
						id: (idx + 1) as u32,
						name: stack[0].proc.path.clone()
					});
				}

				ret
			}

			None => vec![]
		};

		self.send_or_disconnect(Response::Stacks { stacks });
	}

	fn handle_stack_frames(&mut self, stack_id: u32, start_frame: Option<u32>, count: Option<u32>) {
		let response = match self.get_stack(stack_id) {
			Some(stack) => {
				let frame_base = self.get_stack_base_frame_id(stack_id);
				let start_frame = start_frame.unwrap_or(0);
				let end_frame = start_frame + count.unwrap_or(stack.len() as u32);

				let start_frame = start_frame as usize;
				let end_frame = end_frame as usize;

				let mut frames = vec![];

				for i in start_frame..end_frame {
					if i >= stack.len() {
						break;
					}

					let proc_ref = ProcRef {
						path: stack[i].proc.path.to_owned(),
						override_id: stack[i].proc.override_id()
					};

					frames.push(StackFrame {
						id: frame_base + (i as u32),
						instruction: InstructionRef {
							proc: proc_ref.clone(),
							offset: stack[i].offset as u32
						},
						line: self.frame_line_number(&stack[i])
					});
				}

				Response::StackFrames {
					frames,
					total_count: stack.len() as u32
				}
			}

			None => {
				self.notify("received StackFrames request when not paused");
				Response::StackFrames {
					frames: vec![],
					total_count: 0
				}
			}
		};

		self.send_or_disconnect(response);
	}

	fn handle_scopes(&mut self, frame_id: u32) {
		if self.state.is_none() {
			let response = Response::Scopes {
				arguments: None,
				locals: None,
				globals: None
			};

			self.send_or_disconnect(response);
			return;
		}

		let state = self.state.as_ref().unwrap();

		let arguments = Variables::Arguments { frame: frame_id };
		let locals = Variables::Locals { frame: frame_id };

		let globals = Variables::ObjectVars(Value::GLOBAL);

		let response = Response::Scopes {
			arguments: Some(state.get_ref(arguments)),
			locals: Some(state.get_ref(locals)),
			globals: Some(state.get_ref(globals))
		};

		self.send_or_disconnect(response);
	}

	fn handle_variables(&mut self, vars: VariablesRef) {
		let response = match &self.state {
			Some(state) => match state.get_variables(vars) {
				Some(vars) => match vars {
					Variables::Arguments { frame } => Response::Variables { vars: self.get_args(frame) },
					Variables::Locals { frame } => Response::Variables {
						vars: self.get_locals(frame)
					},
					Variables::ObjectVars(value) => match self.object_to_variables(&value) {
						Ok(vars) => Response::Variables { vars },

						Err(e) => {
							self.notify(format!("runtime occured while processing Variables request: {:?}", e));
							Response::Variables { vars: vec![] }
						}
					},
					Variables::ListContents(value) => match self.list_to_variables(&value) {
						Ok(vars) => Response::Variables { vars },

						Err(e) => {
							self.notify(format!("runtime occured while processing Variables request: {:?}", e));
							Response::Variables { vars: vec![] }
						}
					},

					Variables::ListPair { key, value } => Response::Variables {
						vars: vec![
							self.value_to_variable("key".to_owned(), &key),
							self.value_to_variable("value".to_owned(), &value),
						]
					}
				},

				None => {
					self.notify("received unknown VariableRef in Variables request");
					Response::Variables { vars: vec![] }
				}
			},

			None => {
				self.notify("recevied Variables request while not paused");
				Response::Variables { vars: vec![] }
			}
		};

		self.send_or_disconnect(response);
	}

	fn handle_command(&mut self, frame_id: Option<u32>, command: &str) -> String {
		// How many matches variables can you spot? It could be better...
		let response = match self.app.try_get_matches_from_mut(command.split_ascii_whitespace()) {
			Ok(matches) => {
				match matches.subcommand() {
					Some(("disassemble", matches)) => {
						if let Some(proc) = matches.value_of("proc") {
							// Default id to 0 in the worst way possible
							let id = matches.value_of("id").and_then(|x| x.parse::<u32>().ok()).unwrap_or(0);

							self.handle_disassemble(proc, id)
						} else if let Some(frame_id) = frame_id {
							if let Some(frame) = self.get_stack_frame(frame_id) {
								let proc = frame.proc.path.clone();
								let id = frame.proc.override_id();
								self.handle_disassemble(&proc, id)
							} else {
								"couldn't find stack frame (is execution not paused?)".to_owned()
							}
						} else {
							"no execution frame selected".to_owned()
						}
					}

					Some(("guest_override", matches)) => match matches.value_of("ckey") {
						Some(ckey) => match crate::ckey_override::override_guest_ckey(ckey) {
							Ok(()) => "Success".to_owned(),

							Err(e) => {
								format!("Failed: {:?}", e)
							}
						},

						None => "no ckey provided".to_owned()
					},

					Some(("mem_profiler", matches)) => match matches.subcommand() {
						Some(("begin", matches)) => match matches.value_of("path") {
							Some(path) => mem_profiler::begin(path)
								.map(|_| "Memory profiler enabled".to_owned())
								.unwrap_or_else(|e| format!("Failed: {}", e)),

							None => "no path provided".to_owned()
						},

						Some(("end", _)) => {
							mem_profiler::end();
							"Memory profiler disabled".to_owned()
						}

						_ => "unknown memory profiler sub-command".to_owned()
					},

					_ => "unknown command".to_owned()
				}
			}
			Err(e) => e.to_string()
		};

		response
	}

	fn eval_expr(&mut self, frame_id: Option<u32>, command: &str) -> Option<Value> {
		// Assembling the expression runs DM code of its own to look up
		// typepaths and resources, so it counts as eval code too. Otherwise the
		// instruction hook would handle the client's next request in the middle
		// of this one, and a runtime would pause inside the pause we're in.
		self.in_eval = true;
		self.eval_error = None;

		let result = self.compile_and_run_expr(frame_id, command);

		self.in_eval = false;

		if let Some(err) = self.eval_error.take() {
			self.notify(format!("runtime occured when executing expression: {}", err));
		}

		result
	}

	fn compile_and_run_expr(&mut self, frame_id: Option<u32>, command: &str) -> Option<Value> {
		enum ArgType {
			Dot,
			Usr,
			Src,
			Arg(u32),
			Local(u32)
		}

		let (ctx, instance, args) = match frame_id {
			// Global context
			None => (std::ptr::null_mut(), std::ptr::null_mut(), vec![]),

			Some(frame_id) => {
				let frame = match self.get_stack_frame(frame_id) {
					Some(x) => x,
					None => {
						self.notify(format!("tried to evaluate expression with invalid frame id: {}", frame_id));
						return None;
					}
				};

				let mut args = vec![
					(".".to_owned(), frame.dot.clone(), ArgType::Dot),
					("usr".to_owned(), frame.usr.clone(), ArgType::Usr),
					("src".to_owned(), frame.src.clone(), ArgType::Src),
				];

				for (idx, arg) in frame.args.iter().enumerate() {
					if let Some(name) = &arg.0 {
						args.push((name.into(), arg.1.clone(), ArgType::Arg(idx as u32)));
					}
				}

				for (idx, local) in frame.locals.iter().enumerate() {
					args.push(((&local.0).into(), local.1.clone(), ArgType::Local(idx as u32)));
				}

				(frame.context, frame.instance, args)
			}
		};

		let arg_names: Vec<&str> = args.iter().map(|(name, ..)| name.as_str()).collect();
		let arg_values: Vec<&Value> = args.iter().map(|(_, value, _)| value).collect();

		let expr = match dmasm::compiler::compile_expr(command, &arg_names) {
			Ok(expr) => expr,
			Err(err) => {
				self.notify(format!("{}", err));
				return None;
			}
		};

		let assembly = match dmasm::assembler::assemble(&expr, &mut crate::assemble_env::AssembleEnv) {
			Ok(assembly) => assembly,
			Err(err) => {
				self.notify(format!("expression {} failed to assemble: {:#?}", command, err));
				return None;
			}
		};

		let proc = match Proc::find("/proc/auxtools_expr_stub") {
			Some(proc) => proc,
			None => {
				self.notify("Couldn't find /proc/auxtools_expr_stub! DM evaluation not available.");
				return None;
			}
		};

		proc.set_bytecode(assembly);

		match proc.call(&arg_values) {
			Ok(res) => {
				if let Ok(list) = res.as_list() {
					// The rest are the potentially mutated parameters. We need
					// to commit them to the function that
					// called us. TODO: This sucks, obviously.
					let len = list.len();
					for i in 2..=len {
						let value = list.get(i).unwrap();
						let slot = &args[i as usize - 2].2;

						unsafe {
							match slot {
								ArgType::Dot => {
									let dot = (*ctx).dot_mut();
									let _ = Value::from_raw_owned(*dot);
									*dot = value.raw;
								}
								ArgType::Usr => {
									let _ = Value::from_raw_owned((*instance).usr);
									(*instance).usr = value.raw;
								}
								ArgType::Src => {
									let _ = Value::from_raw_owned((*instance).src);
									(*instance).src = value.raw;
								}
								ArgType::Arg(idx) => {
									let args = (*instance).args();
									let arg = args.add(*idx as usize);
									let _ = Value::from_raw_owned(*arg);
									(*arg) = value.raw;
								}
								ArgType::Local(idx) => {
									let locals = *(*ctx).locals();
									let local = locals.add(*idx as usize);
									let _ = Value::from_raw_owned(*local);
									(*local) = value.raw;
								}
							}
						}

						std::mem::forget(value);
					}

					Some(list.get(1).unwrap())
				} else {
					None
				}
			}

			Err(_) => {
				self.notify(format!("Value::call failed when evaluating expression {}", command));
				None
			}
		}
	}

	fn handle_eval(&mut self, frame_id: Option<u32>, command: &str, context: Option<String>) {
		if let Some(command) = command.strip_prefix('#') {
			let response = self.handle_command(frame_id, command);
			self.send_or_disconnect(Response::Eval(EvalResponse {
				value: response,
				variables: None
			}));
			return;
		}

		match self.eval_expr(frame_id, command) {
			Some(result) => {
				let variables = match context {
					Some(str) if str == "repl" => None,
					_ => self.value_to_variables_ref(&result)
				};

				self.send_or_disconnect(Response::Eval(EvalResponse {
					value: Self::stringify(&result),
					variables
				}));
			}

			None => {
				self.send_or_disconnect(Response::Eval(EvalResponse {
					value: "<no value>".to_owned(),
					variables: None
				}));
			}
		}
	}

	fn handle_hot_reload(&mut self, request: &HotReloadRequest) {
		let result = self.hot_reload(request);
		self.send_or_disconnect(Response::HotReload(result));
	}

	fn hot_reload(&mut self, request: &HotReloadRequest) -> HotReloadResult {
		// Assembling runs DM code to look up typepaths and resources, which
		// counts as eval code (see eval_expr)
		self.in_eval = true;
		self.eval_error = None;

		let mut warnings = vec![];
		// New types come first, so the rest can be prepared against them
		let prepared = hot_reload::add_types(&request.new_types).and_then(|added| {
			hot_reload::prepare(request, &mut warnings).map_err(|mut errors| {
				if added > 0 {
					errors.push(format!("the {} new types were added anyway; nothing uses them yet", added));
				}
				errors
			})
		});
		let result = match prepared {
			Ok(prepared) => {
				hot_reload::apply(prepared, &mut self.breakpoints, &mut self.moved_breakpoints, &mut warnings);
				HotReloadResult {
					applied: true,
					errors: vec![],
					warnings
				}
			}

			Err(errors) => HotReloadResult {
				applied: false,
				errors,
				warnings
			}
		};

		self.in_eval = false;
		if let Some(err) = self.eval_error.take() {
			self.notify(format!("runtime occured during a hot reload: {}", err));
		}

		// Every client hears what changed
		if result.applied && !request.is_empty() {
			self.notify(format!(
				"Hot reload: new code for {} procs, new settings for {}, {} new procs, {} unlinked, {} new vars, {} new var defaults, {} new init \
				 procs, {} moved",
				request.procs.len(),
				request.settings.len(),
				request.new_procs.len(),
				request.removed_procs.len(),
				request.new_vars.len(),
				request.var_defaults.len(),
				request.init_procs.len(),
				request.line_moves.len()
			));
		}

		result
	}

	fn handle_disassemble(&mut self, path: &str, id: u32) -> String {
		match auxtools::Proc::find_override(path, id) {
			Some(proc) => {
				// The live bytecode has our breakpoints patched into it, which
				// nobody wants to read
				let bytecode = original_bytecode(&proc);

				let mut env = crate::DisassembleEnv;
				let (nodes, error) = dmasm::disassembler::disassemble(&bytecode, &mut env);
				let dism = dmasm::format_disassembly(&nodes, None);

				match error {
					Some(error) => {
						format!("Dism for {:?}\n{}\n\tError: {:?}", proc, dism, error)
					}

					None => {
						format!("Dism for {:?}\n{}", proc, dism)
					}
				}
			}

			None => "Proc not found".to_owned()
		}
	}

	// returns true if we need to break
	fn handle_request(&mut self, request: Request) -> bool {
		match request {
			Request::Disconnect => unreachable!(),
			Request::CatchRuntimes { should_catch } => {
				if let Some(client) = self.current.and_then(|id| self.clients.get_mut(&id)) {
					client.catch_runtimes = should_catch;
				}
			}
			Request::BreakpointSet { instruction, condition } => self.handle_breakpoint_set(instruction, condition),
			Request::BreakpointUnset { instruction } => self.handle_breakpoint_unset(instruction),
			Request::Stacks => self.handle_stacks(),
			Request::Scopes { frame_id } => self.handle_scopes(frame_id),
			Request::Variables { vars } => self.handle_variables(vars),
			Request::Eval { frame_id, command, context } => self.handle_eval(frame_id, &command, context),

			Request::StackFrames {
				stack_id,
				start_frame,
				count
			} => self.handle_stack_frames(stack_id, start_frame, count),

			Request::LineNumber { proc, offset } => {
				self.send_or_disconnect(Response::LineNumber {
					line: self.get_line_number(proc, offset)
				});
			}

			Request::Offset { proc, line } => {
				self.send_or_disconnect(Response::Offset {
					offset: self.get_offset(proc, line)
				});
			}

			Request::Pause => {
				self.send_or_disconnect(Response::Ack);
				return true;
			}

			Request::ByondVersion => {
				let (version, build) = auxtools::version::get();
				self.send_or_disconnect(Response::ByondVersion { version, build });
			}

			Request::HotReload(request) => self.handle_hot_reload(&request),

			Request::WorldInfo => {
				let (args, cwd) = crate::launched::command_line();
				self.send_or_disconnect(Response::WorldInfo { args, cwd });
			}

			Request::StdDef => {
				let stddef = crate::stddef::get_stddef().map(|x| x.to_string());
				self.send_or_disconnect(Response::StdDef(stddef));
			}

			Request::CurrentInstruction { frame_id } => {
				let response = self.get_stack_frame(frame_id).map(|frame| InstructionRef {
					proc: ProcRef {
						path: frame.proc.path.to_owned(),
						override_id: frame.proc.override_id()
					},
					offset: frame.offset as u32
				});

				self.send_or_disconnect(Response::CurrentInstruction(response));
			}

			// The following requests are special cases and handled outside of
			// this function
			Request::Configured | Request::Continue { .. } => {
				self.send_or_disconnect(Response::Ack);
			}
		}

		false
	}

	/// Applies whatever the networking threads have reported and hands back the
	/// next request, with `current` set to the client that sent it. `None`
	/// means nothing came in time: for `Wait::WhileClients`, that the last
	/// client left.
	fn recv_request(&mut self, wait: Wait) -> Option<Request> {
		loop {
			let event = if wait == Wait::No {
				self.events.try_recv().ok()?
			} else {
				self.events.recv().ok()?
			};

			match event {
				Event::Connected(id, connection) => {
					self.clients.insert(id, Client {
						connection,
						catch_runtimes: false
					});
				}

				Event::Disconnected(id) => {
					self.remove_client(id);

					if wait == Wait::WhileClients && self.clients.is_empty() {
						return None;
					}
				}

				// Sent by a client we've since given up on
				Event::Request(id, _) if !self.clients.contains_key(&id) => {}

				Event::Request(id, request) => {
					self.current = Some(id);
					return Some(request);
				}
			}
		}
	}

	/// Tells every client
	pub fn notify<T: Into<String>>(&mut self, message: T) {
		let message = message.into();
		eprintln!("Debug Server: {:?}", message);

		self.broadcast(Response::Notification { message }, None);
	}

	/// Something in here panicked. We don't know what state that left the
	/// session in, so the debug-client gets dropped and can connect again.
	pub fn recover_from_panic(&mut self) {
		eprintln!("Debug server panicked, dropping the debug clients");
		self.in_eval = false;
		// A panic mid-pause skips the cleanup at the end of handle_breakpoint.
		// The frames in here point at procs that are about to carry on running.
		self.state = None;
		self.disconnect_all();
	}

	pub fn handle_breakpoint(&mut self, _ctx: *mut raw_types::procs::ExecutionContext, reason: BreakpointReason) -> ContinueKind {
		// Ignore all breakpoints unless someone's connected, and runtime errors
		// unless someone wants them
		let wanted = match reason {
			BreakpointReason::Runtime(_) => self.clients.values().any(|client| client.catch_runtimes),
			_ => !self.clients.is_empty()
		};
		if !wanted {
			return ContinueKind::Continue;
		}

		self.state = Some(State::new());

		// Exit now if this is a conditional breakpoint and no condition passes.
		// One set without a condition always stops.
		if reason == BreakpointReason::Breakpoint {
			let proc = unsafe { (*(*(*_ctx).proc_instance())).proc };
			let offset = unsafe { *(*_ctx).bytecode_offset() };
			let conditions: Option<Vec<String>> = self
				.breakpoints
				.get(&(proc, offset))
				.filter(|claims| claims.values().all(Option::is_some))
				.map(|claims| claims.values().flatten().cloned().collect());

			if let Some(conditions) = conditions {
				let mut pause = false;
				for condition in conditions {
					let passed = self.eval_expr(Some(0), &condition).is_none_or(|result| result.is_truthy());

					// We might have just executed some code so invalidate the
					// stacks we already fetched
					self.state.as_mut().unwrap().invalidate_stacks();
					if passed {
						pause = true;
						break;
					}
				}

				if !pause {
					self.state = None;
					return ContinueKind::Continue;
				}
			}
		}

		self.notify(format!("Pausing execution (reason: {:?})", reason));
		self.broadcast(Response::BreakpointHit { reason }, None);

		while !self.clients.is_empty() {
			let Some(request) = self.recv_request(Wait::WhileClients) else {
				break;
			};

			// Hijack and handle any Continue requests. The other clients are
			// told the world carries on.
			if let Request::Continue { kind } = request {
				self.send_or_disconnect(Response::Ack);
				self.broadcast(Response::Resumed, self.current);
				self.state = None;
				return kind;
			}

			// Hijack eval too so that we can refresh our state after it
			if let Request::Eval { frame_id, command, context } = request {
				self.handle_eval(frame_id, &command, context);
				self.state.as_mut().unwrap().invalidate_stacks();
				continue;
			}

			// if we get a pause request here we can ignore it
			self.handle_request(request);
		}

		// Every client left
		self.state = None;
		ContinueKind::Continue
	}

	// returns true if we need to pause
	pub fn process(&mut self) -> bool {
		let mut should_pause = false;

		while let Some(request) = self.recv_request(Wait::No) {
			// Not `||`, that would skip every request after a Pause
			should_pause |= self.handle_request(request);
		}

		should_pause
	}

	/// Block while processing all received requests normally until a debug
	/// client says it's configured, however long that takes to connect
	pub fn process_until_configured(&mut self) {
		while let Some(request) = self.recv_request(Wait::Forever) {
			if let Request::Configured = request {
				self.send_or_disconnect(Response::Ack);
				break;
			}

			self.handle_request(request);
		}
	}

	/// Answers the client whose request this is
	fn send_or_disconnect(&mut self, response: Response) {
		// Nobody to tell: the client left while we were busy
		let Some(id) = self.current else {
			return;
		};
		let Some(client) = self.clients.get_mut(&id) else {
			return;
		};

		if let Err(e) = Self::send(&mut client.connection, &response) {
			eprintln!("Debug server failed to send message: {}", e);
			self.remove_client(id);
		}
	}

	/// Tells every client but `except`
	fn broadcast(&mut self, response: Response, except: Option<ClientId>) {
		let mut gone = vec![];
		for (&id, client) in &mut self.clients {
			if Some(id) != except && Self::send(&mut client.connection, &response).is_err() {
				gone.push(id);
			}
		}
		for id in gone {
			self.remove_client(id);
		}
	}

	/// Says goodbye to a client and takes away its breakpoints
	fn remove_client(&mut self, id: ClientId) {
		let Some(mut client) = self.clients.remove(&id) else {
			return;
		};
		eprintln!("Debug server disconnecting a client");
		let _ = Self::send(&mut client.connection, &Response::Disconnect);
		if self.current == Some(id) {
			self.current = None;
		}

		let mut unclaimed = vec![];
		for (key, claims) in &mut self.breakpoints {
			claims.remove(&id);
			if claims.is_empty() {
				unclaimed.push(*key);
			}
		}
		for (proc, offset) in unclaimed {
			self.breakpoints.remove(&(proc, offset));
			if let Some(proc) = Proc::from_id(proc) {
				let _ = unhook_instruction(&proc, offset as u32);
			}
		}

		// Whoever connects next sets up their own breakpoints. Until then the
		// old ones would only slow down the procs they sit in.
		if self.clients.is_empty() {
			self.breakpoints.clear();
			self.moved_breakpoints.clear();
			unhook_all();
		}
	}

	fn disconnect_all(&mut self) {
		let ids: Vec<ClientId> = self.clients.keys().copied().collect();
		for id in ids {
			self.remove_client(id);
		}
	}

	pub(crate) fn send(stream: &mut impl Write, response: &Response) -> Result<(), Box<dyn Error>> {
		let data = bincode::serialize(response)?;
		stream.write_all(&(data.len() as u32).to_le_bytes())?;
		stream.write_all(&data[..])?;
		stream.flush()?;
		Ok(())
	}
}

impl Drop for Server {
	fn drop(&mut self) {
		self.disconnect_all();

		// Clients from now on wait for the next Server
		if let Some(acceptor) = ACCEPTOR.lock().unwrap().as_mut() {
			let (events, waiting) = mpsc::channel();
			acceptor.events = events;
			acceptor.waiting = Some(waiting);
		}

		// Clients that connected before that, which we never got told about,
		// are told to go
		while let Ok(event) = self.events.try_recv() {
			if let Event::Connected(_, mut connection) = event {
				let _ = Self::send(&mut connection, &Response::Disconnect);
			}
		}
	}
}

/// Reads one request off a connection: a u32 length, then the request in
/// bincode.
fn read_request(stream: &mut impl Read, buf: &mut Vec<u8>) -> Result<Request, Box<dyn Error>> {
	let mut len_bytes = [0u8; 4];
	stream.read_exact(&mut len_bytes)?;
	let len = u32::from_le_bytes(len_bytes);

	if len > MAX_REQUEST_LEN {
		return Err(format!("a {} byte request is too big to be real", len).into());
	}

	buf.resize(len as usize, 0);
	stream.read_exact(buf)?;
	Ok(bincode::deserialize::<Request>(buf)?)
}

impl ServerThread {
	fn run(&self, id: ClientId, mut stream: Connection) {
		let mut buf = vec![];

		loop {
			let request = match read_request(&mut stream, &mut buf) {
				Ok(request) => request,

				Err(e) => {
					eprintln!("Debug server thread read error: {}", e);
					break;
				}
			};

			if let Request::Disconnect = request {
				eprintln!("Debug client disconnected");
				break;
			}

			if let Err(e) = self.events.send(Event::Request(id, request)) {
				eprintln!("Debug server thread failed to handle request: {}", e);
				break;
			}
		}

		let _ = self.events.send(Event::Disconnected(id));

		eprintln!("Debug client connection closed");
	}
}

#[cfg(test)]
mod tests {
	use std::{io::Read, sync::mpsc, thread, time::Duration};

	use super::{Connection, Server};
	use crate::connection;

	#[test]
	fn drop_hangs_up_on_clients_and_the_next_server_takes_new_ones() {
		let (done, dropped) = mpsc::channel();

		thread::spawn(move || {
			let mut server = Server::listen().unwrap();
			let first = Connection::connect(&connection::world_name()).unwrap();
			let second = Connection::connect(&connection::world_name()).unwrap();
			thread::sleep(Duration::from_millis(200));
			// Both are taken in
			assert!(server.recv_request(super::Wait::No).is_none());
			assert_eq!(server.clients.len(), 2);

			drop(server);
			let _ = done.send((first, second));
		});

		let (mut first, mut second) = dropped
			.recv_timeout(Duration::from_secs(5))
			.expect("dropping the server hung on its clients");
		// Each was told to go: a Disconnect, then the end of the connection
		for client in [&mut first, &mut second] {
			let mut len = [0u8; 4];
			client.read_exact(&mut len).expect("no Disconnect");
		}

		// A client that connects while the world reboots waits for the next
		// server
		let _third = Connection::connect(&connection::world_name()).expect("nothing listens between servers");
		thread::sleep(Duration::from_millis(200));
		let mut server = Server::listen().expect("the next server couldn't listen");
		assert!(server.recv_request(super::Wait::No).is_none());
		assert_eq!(server.clients.len(), 1);
		assert!(Server::listen().is_err(), "two servers at once");
	}
}
