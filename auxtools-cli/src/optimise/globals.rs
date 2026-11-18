//! Make procs that never use `src` global procs.
//!
//! A proc on a type has an entry in the proc table of every type below it;
//! a global proc has none, and is called without a lookup. A proc becomes
//! global when nothing can tell the difference: it is declared once and
//! never overridden, its body never refers to `src`, it cannot sleep (a
//! sleeping proc ends when its `src` is deleted, a global proc goes on),
//! and every call of it is a call on the caller's own `src` from a proc of
//! the same type or one below. The proc keeps its path; a runtime error in
//! it reports no `src`.

use std::collections::{HashMap, HashSet};

use dmasm::{
	disassembler::{disassemble, DisassembleEnv},
	dmb::{Dmb, Proc, NONE},
	Instruction, Node, ReferenceKind, Variable
};

use super::{lists_not_code, ProcPath, ProcTables, Strings, Uses};

/// Words of `GetVar src`, `SetVar cache`: what `cache = src` in a call's
/// operand did before the call, for the code after it that reads the cache.
const CACHE_SRC: [u32; 4] = [0x33, 0xFFCE, 0x34, 0xFFD8];
const CALL_GLOBAL: u32 = 0x30;
const CALL_GLOBAL_ARGLIST: u32 = 0xCD;
/// The argument count of a call that takes its arguments from a list.
const ARGLIST: u32 = 0xFFFF;

/// Built-in procs of lists, which run no code of the world's and return at
/// once.
const LIST_PROCS: [&str; 10] = ["Add", "Copy", "Cut", "Find", "Insert", "Join", "Remove", "RemoveAll", "Splice", "Swap"];

/// Built-in procs of `world` and `client` that wait for an answer. A proc of
/// the world's with the same name does not make a call by that name safe.
const WAITING_PROCS: [&str; 26] = [
	"AddCredits",
	"AllowUpload",
	"CheckPassport",
	"ClearMedal",
	"Export",
	"GetAPI",
	"GetConfig",
	"GetCredits",
	"GetMedal",
	"GetScores",
	"Import",
	"IsBanned",
	"IsByondMember",
	"IsSubscribed",
	"MeasureText",
	"OpenPort",
	"PayCredits",
	"Reboot",
	"RenderIcon",
	"Repop",
	"SendPage",
	"SetAPI",
	"SetConfig",
	"SetMedal",
	"SetScores",
	"SoundQuery"
];

/// The procs BYOND itself compiles into every world, by type. A call by
/// name reaches them where their names are those of list procs (`Add`,
/// `Find`). The objects they make are of their own types.
const OWN_PROCS: [(&str, &[&str]); 8] = [
	("/sound", &["New", "operator:=", "RscFile"]),
	("/icon", &[
		"New",
		"operator:=",
		"Icon",
		"RscFile",
		"IconStates",
		"Turn",
		"Flip",
		"Shift",
		"SetIntensity",
		"Blend",
		"SwapColor",
		"DrawBox",
		"Insert",
		"MapColors",
		"Scale",
		"Crop",
		"GetPixel",
		"Width",
		"Height"
	]),
	("/matrix", &[
		"New",
		"operator:=",
		"Multiply",
		"Add",
		"Subtract",
		"Invert",
		"Turn",
		"Scale",
		"Translate",
		"Interpolate"
	]),
	("/database", &["New", "Open", "Close", "Error", "ErrorMsg"]),
	("/database/query", &[
		"New",
		"Open",
		"Clear",
		"Add",
		"Execute",
		"NextRow",
		"RowsAffected",
		"Reset",
		"Columns",
		"GetColumn",
		"GetRowData",
		"Close"
	]),
	("/exception", &["New"]),
	("/regex", &["New", "Find", "Replace", "Find_char", "Replace_char"]),
	("/generator", &[
		"New",
		"Rand",
		"Turn",
		"operator+",
		"operator-",
		"operator*",
		"operator/",
		"operator+=",
		"operator-=",
		"operator*=",
		"operator/=",
		"operator_turn"
	])
];

/// Whether a type is one of BYOND's own that `OWN_PROCS` lists.
fn own_type(owner: &[u8]) -> bool {
	OWN_PROCS.iter().any(|(ty, _)| ty.as_bytes() == owner)
}

#[derive(Debug, Default)]
pub struct Report {
	/// The paths of the procs made global, and the calls changed.
	pub procs: Vec<String>,
	pub calls: usize,
	/// The proc tables before and after.
	pub before: ProcTables,
	pub after: ProcTables,
	/// Why nothing was done, when the world as a whole rules it out.
	pub refused: Option<&'static str>
}

/// Disassembles with ids for names, so that operands can be compared with
/// the tables.
struct Ids;

impl DisassembleEnv for Ids {
	fn get_string_data(&mut self, index: u32) -> Option<Vec<u8>> {
		Some(index.to_string().into_bytes())
	}

	fn get_variable_name(&mut self, index: u32) -> Option<Vec<u8>> {
		Some(index.to_string().into_bytes())
	}

	fn get_proc_name(&mut self, index: u32) -> Option<String> {
		Some(index.to_string())
	}

	fn value_to_string_data(&mut self, _tag: u32, data: u32) -> Option<Vec<u8>> {
		Some(data.to_string().into_bytes())
	}
}

fn id(text: &[u8]) -> u32 {
	std::str::from_utf8(text).ok().and_then(|text| text.parse().ok()).unwrap_or(NONE)
}

/// What a call names: procs by the string they are looked up by, or one
/// proc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Target {
	Name(u32),
	Proc(u32)
}

fn target(variable: &Variable) -> Option<Target> {
	match variable {
		Variable::DynamicProc(name) | Variable::DynamicVerb(name) => Some(Target::Name(id(&name.0))),
		Variable::StaticProc(proc) | Variable::StaticVerb(proc) => Some(Target::Proc(id(proc.0.as_bytes()))),
		_ => None
	}
}

fn refers_to_src(variable: &Variable) -> bool {
	match variable {
		// `caller` and `callee` show the frame, src included
		Variable::Src | Variable::Caller | Variable::Callee => true,
		Variable::SetCache(object, rest) => refers_to_src(object) || refers_to_src(rest),
		Variable::Initial(inner) | Variable::IsSaved(inner) | Variable::PtrRef(inner) | Variable::PtrDeref(inner) => refers_to_src(inner),
		_ => false
	}
}

/// What the cache register holds after an instruction, as far as its
/// operands say.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cache {
	/// Untouched
	Kept,
	Src,
	Other
}

/// What an operand leaves in the cache register.
fn cache_from(variable: &Variable) -> Cache {
	match variable {
		Variable::SetCache(object, rest) if **object == Variable::Src && !matches!(**rest, Variable::SetCache(..)) => Cache::Src,
		Variable::SetCache(..) | Variable::Cache | Variable::CacheKey | Variable::CacheIndex => Cache::Other,
		Variable::Initial(inner) | Variable::IsSaved(inner) | Variable::PtrRef(inner) | Variable::PtrDeref(inner) => cache_from(inner),
		_ => Cache::Kept
	}
}

fn cache_after(instruction: &Instruction) -> Cache {
	use Instruction::*;
	match instruction {
		PopCache | PopCacheKey | SetCacheJmpIfNull(_) | SetCachePopJmpIfNull(_) => return Cache::Other,
		GetVar(Variable::Cache) => return Cache::Kept,
		_ => {}
	}
	let mut cache = Cache::Kept;
	for variable in instruction.variables() {
		match cache_from(variable) {
			Cache::Kept => {}
			Cache::Other => return Cache::Other,
			Cache::Src => cache = Cache::Src
		}
	}
	cache
}

/// What running an instruction can do besides computing.
enum Effect {
	None,
	/// It can wait, or runs code this pass does not follow.
	Waits,
	/// It runs the world's procs of these names, if there are any.
	Runs(&'static [&'static str]),
	/// It runs the proc this one overrides.
	Parent
}

const fn effect(instruction: &Instruction) -> Effect {
	use Instruction::*;
	const MOVES: &[&str] = &[
		"Move",
		"Enter",
		"Exit",
		"Entered",
		"Exited",
		"Cross",
		"Uncross",
		"Crossed",
		"Uncrossed",
		"Bump"
	];
	match instruction {
		Sleep | Spawn(_) | Input(..) | InputColor(..) | Alert | PromptCheck | WinGet | WinExists | Shell => Effect::Waits,
		// Calls of procs named at run time and of libraries
		CallParent | CallParentArgs(_) | CallParentArgList => Effect::Parent,
		CallName(_) | CallNameArgList | CallPath(_) | CallPathArgList | CallLib(_) | CallLibArgList | LoadExt | CallExtLoaded(_)
		| CallExtLoadedArgList | Eval | DmsNewKernel | DmsParse | DmsExportText | DmsPrepare | Startup(_) | Shutdown | ShutdownArgs => Effect::Waits,
		New(_) | NewArgList | NewImage | NewImageArgList | NewImageArgs(_) | IconNew(_) | MatrixNew(_) | RegexNew(_) | MakeGenerator(_)
		| FilterNewArgList => Effect::Runs(&["New"]),
		Del => Effect::Runs(&["Del"]),
		Output => Effect::Runs(&["Write"]),
		// Text made of values, and indexing, run these operators where a
		// type has them
		OutputFormat(..) => Effect::Runs(&["Write", "operator\"\""]),
		Format(..) => Effect::Runs(&["operator\"\""]),
		ListGet => Effect::Runs(&["operator[]"]),
		ListSet => Effect::Runs(&["operator[]="]),
		Read => Effect::Runs(&["Read"]),
		Step | StepTo | StepAway | StepTowards | StepRand | Walk | WalkTo | WalkAway | WalkTowards | WalkRand | StepSpeed | StepToSpeed
		| StepAwaySpeed | StepTowardsSpeed | StepRandSpeed | WalkSpeed | WalkToSpeed | WalkAwaySpeed | WalkTowardsSpeed | WalkRandSpeed => {
			Effect::Runs(MOVES)
		}
		_ => Effect::None
	}
}

/// A call of a proc by name or id, as it stands in the bytecode.
struct Site {
	target: Target,
	/// The instruction's first word and its length.
	offset: u32,
	len: u32,
	arguments: u32,
	/// The call is on the caller's `src`.
	on_src: bool,
	/// The operand sets the cache to `src` itself, rather than finding it
	/// there.
	sets_cache: bool
}

/// What a proc's bytecode does.
#[derive(Default)]
struct Body {
	read: bool,
	uses_src: bool,
	waits: bool,
	/// It calls the proc it overrides.
	parent: bool,
	/// Names of procs it runs without calling them.
	runs: Vec<&'static str>,
	/// The names of the object vars it sets: setting a mob's key runs
	/// `Login` and `Logout`.
	sets: Vec<u32>,
	sites: Vec<Site>,
	/// Every call target, for following what the proc can reach.
	calls: Vec<Target>
}

fn body(words: &[u32]) -> Body {
	let (nodes, error) = disassemble(words, &mut Ids);
	let mut body = Body {
		read: error.is_none(),
		..Body::default()
	};
	// What the cache holds as DreamMaker knew it: nothing across a label
	let mut cache = Cache::Other;
	for node in &nodes {
		let (instruction, at) = match node {
			Node::Instruction(instruction, at) => (instruction, at),
			_ => {
				cache = Cache::Other;
				continue;
			}
		};
		use Instruction::*;
		if matches!(
			instruction,
			CallSelf | CallSelfArgs(_) | CallSelfArgList | CallParent | CallParentArgs(_) | CallParentArgList
		) {
			body.uses_src = true;
		}
		match effect(instruction) {
			Effect::None => {}
			Effect::Waits => body.waits = true,
			Effect::Runs(names) => body.runs.extend(names),
			Effect::Parent => body.parent = true
		}
		match instruction {
			SetVar(variable) | SetVarExpr(variable) | AssignInto(variable) => {
				let mut last = variable;
				while let Variable::SetCache(_, rest) = last {
					last = rest;
				}
				if let Variable::Field(name) = last {
					body.sets.push(id(&name.0));
				}
			}
			CallGlob(_, proc) | CallGlobalArgList(proc) => body.calls.push(Target::Proc(id(proc.0.as_bytes()))),
			Call(variable, arguments) | CallStatement(variable, arguments) => {
				let (receiver, called) = match variable {
					Variable::SetCache(object, rest) => (Some(&**object), &**rest),
					other => (None, other)
				};
				match target(called) {
					Some(target) => {
						body.calls.push(target);
						let on_src = match receiver {
							Some(Variable::Src) => true,
							Some(_) => false,
							None => cache == Cache::Src
						};
						body.sites.push(Site {
							target,
							offset: at.offset,
							len: at.bytecode.len() as u32,
							arguments: *arguments,
							on_src,
							sets_cache: receiver.is_some()
						});
					}
					// A chain of objects before the proc: followed for what
					// it can reach
					None => {
						let mut last = called;
						while let Variable::SetCache(_, rest) = last {
							last = rest;
						}
						match target(last) {
							Some(target) => {
								body.calls.push(target);
								body.sites.push(Site {
									target,
									offset: at.offset,
									len: at.bytecode.len() as u32,
									arguments: *arguments,
									on_src: false,
									sets_cache: true
								});
							}
							None => body.waits = true
						}
					}
				}
			}
			_ => {}
		}
		if instruction.variables().into_iter().any(refers_to_src) {
			body.uses_src = true;
		}
		match cache_after(instruction) {
			Cache::Kept => {}
			after => cache = after
		}
		// Nothing runs on from a jump: what follows is reached from elsewhere
		if matches!(instruction, Jmp(_) | JmpLoop(_) | Ret | End | Throw) {
			cache = Cache::Other;
		}
	}
	body
}

/// Whether the proc has the settings DreamMaker gives a proc with none.
fn plain(proc: &Proc) -> bool {
	const WAITFOR: u8 = 0x04;
	proc.flags & !WAITFOR == 0
		&& proc.flags_ex.is_none()
		&& proc.desc == NONE
		&& proc.category == NONE
		&& (proc.src_range, proc.src_kind) == (0xFF, 0)
}

/// `words` with each `(offset, length)` replaced by new words, and every
/// jump moved to where its target went. `None` if a jump lands inside a
/// replaced instruction or the result does not read as it should.
fn splice(words: &[u32], edits: &[(u32, u32, Vec<u32>)]) -> Option<Vec<u32>> {
	let scanned = dmasm::scan(words).ok()?;
	let moved = |offset: u32| -> Option<u32> {
		let mut shift = 0i64;
		for (at, len, new) in edits {
			if offset >= at + len {
				shift += new.len() as i64 - i64::from(*len);
			} else if offset > *at {
				return None;
			}
		}
		u32::try_from(i64::from(offset) + shift).ok()
	};
	let mut out = Vec::with_capacity(words.len() + edits.len());
	let mut edits_left = edits.iter().peekable();
	let mut at = 0u32;
	while (at as usize) < words.len() {
		match edits_left.peek() {
			Some((offset, len, new)) if *offset == at => {
				out.extend_from_slice(new);
				at += len;
				edits_left.next();
			}
			_ => {
				out.push(words[at as usize]);
				at += 1;
			}
		}
	}
	if edits_left.next().is_some() {
		return None;
	}
	for label in &scanned.label_operands {
		let operand = moved(label.word_offset)? as usize;
		out[operand] = moved(label.target)?;
	}
	// Each jump of the result lands on an instruction, and nothing but the
	// replaced instructions changed
	let rescanned = dmasm::scan(&out).ok()?;
	let starts: HashSet<u32> = rescanned.instructions.iter().map(|instruction| instruction.offset).collect();
	let added: usize = edits
		.iter()
		.map(|(_, _, new)| dmasm::scan(new).map_or(usize::MAX / 2, |new| new.instructions.len()))
		.sum();
	let lands = rescanned
		.label_operands
		.iter()
		.all(|label| starts.contains(&label.target) || label.target as usize == out.len());
	(lands && rescanned.instructions.len() + edits.len() == scanned.instructions.len() + added).then_some(out)
}

pub fn run(dmb: &mut Dmb) -> Report {
	let mut report = Report {
		before: ProcTables::count(dmb),
		..Report::default()
	};
	report.after = report.before;
	let strings = Strings::new(dmb);
	let uses = Uses::new(dmb);

	let bodies: HashMap<u32, Body> = uses.code.iter().map(|&list| (list, body(&dmb.lists[list as usize]))).collect();
	let body_of = |proc: u32| bodies.get(&dmb.procs[proc as usize].code);

	// Every proc's path, and the procs a name or an identifier stands for
	let paths: Vec<Option<ProcPath>> = dmb
		.procs
		.iter()
		.map(|proc| {
			dmb.strings
				.get(proc.path as usize)
				.filter(|_| proc.path != NONE)
				.and_then(|path| ProcPath::parse(path))
		})
		.collect();
	let mut by_identifier: HashMap<&[u8], Vec<u32>> = HashMap::new();
	let mut by_string: HashMap<u32, Vec<u32>> = HashMap::new();
	let mut declared: HashSet<&[u8]> = HashSet::new();
	let mut by_path: HashMap<u32, usize> = HashMap::new();
	for (proc, path) in paths.iter().enumerate() {
		let Some(path) = path else { continue };
		*by_path.entry(dmb.procs[proc].path).or_default() += 1;
		by_identifier.entry(path.identifier).or_default().push(proc as u32);
		let name = dmb.procs[proc].name;
		if name != NONE {
			by_string.entry(name).or_default().push(proc as u32);
		}
		if let Some(identifier) = strings.get(path.identifier).filter(|&identifier| identifier != name) {
			by_string.entry(identifier).or_default().push(proc as u32);
		}
		let path_text = &dmb.strings[dmb.procs[proc].path as usize];
		if path_text.len() > path.owner.len() + 1 + path.identifier.len() {
			declared.insert(path.identifier);
		}
	}

	// Which procs can wait, or run something that can: to a fixed point
	let text = |string: u32| dmb.strings.get(string as usize).map_or(&[][..], Vec::as_slice);
	let named = |names: &[&str], text: &[u8]| names.iter().any(|name| name.as_bytes() == text);
	let mut waits: Vec<bool> = Vec::with_capacity(dmb.procs.len());
	let mut reaches: Vec<Vec<u32>> = Vec::with_capacity(dmb.procs.len());
	for proc in 0..dmb.procs.len() as u32 {
		let Some(body) = body_of(proc).filter(|body| body.read) else {
			waits.push(true);
			reaches.push(Vec::new());
			continue;
		};
		let mut wait = body.waits || dmb.procs[proc as usize].flags_ex.is_some();
		let mut reach = Vec::new();
		let logs_in = body.sets.iter().any(|&var| named(&["key", "ckey", "mob"], text(var)));
		// One of BYOND's own procs makes objects of BYOND's own types
		let own = paths[proc as usize].as_ref().is_some_and(|path| {
			by_path.get(&dmb.procs[proc as usize].path).copied() == Some(1)
				&& OWN_PROCS
					.iter()
					.any(|(ty, procs)| ty.as_bytes() == path.owner && named(procs, path.identifier))
		});
		// The proc a `..()` runs is one of the same identifier, or the
		// engine's: followed for `New`, which gives a client its mob
		let mut parents: &[&str] = &[];
		if body.parent {
			match paths[proc as usize].as_ref().map(|path| path.identifier) {
				Some(b"New") => parents = &["New", "Login"],
				Some(identifier) if declared.contains(identifier) => reach.extend(by_identifier.get(identifier).into_iter().flatten().copied()),
				_ => wait = true
			}
		}
		let runs = body.runs.iter().chain(parents).copied();
		for name in runs.chain(logs_in.then_some(["Login", "Logout"]).into_iter().flatten()) {
			let run = by_identifier.get(name.as_bytes()).into_iter().flatten().copied();
			reach.extend(run.filter(|&run| !(own && name == "New") || paths[run as usize].as_ref().is_some_and(|path| own_type(path.owner))));
		}
		for call in &body.calls {
			match *call {
				Target::Proc(callee) if (callee as usize) < dmb.procs.len() => reach.push(callee),
				Target::Proc(_) => wait = true,
				Target::Name(name) => {
					let procs = by_string.get(&name).map_or(&[][..], Vec::as_slice);
					// A name the world declares is the world's procs; any
					// other is the engine's proc, safe only where known
					let engine = !procs
						.iter()
						.any(|&proc| paths[proc as usize].as_ref().is_some_and(|path| declared.contains(path.identifier)));
					let name = text(name);
					if named(&WAITING_PROCS, name) || (engine && !named(&LIST_PROCS, name)) {
						wait = true;
					}
					reach.extend_from_slice(procs);
				}
			}
		}
		waits.push(wait);
		reaches.push(reach);
	}
	loop {
		let mut changed = false;
		for proc in 0..waits.len() {
			if !waits[proc] && reaches[proc].iter().any(|&reached| waits[reached as usize]) {
				waits[proc] = true;
				changed = true;
			}
		}
		if !changed {
			break;
		}
	}
	// The other operators run wherever a value of their type is computed
	// with, which is not followed: none of them may wait. BYOND's own are
	// left out: they make values of their own types and nothing more.
	const FOLLOWED: [&[u8]; 3] = [b"operator\"\"", b"operator[]", b"operator[]="];
	let operator_waits = paths.iter().enumerate().any(|(proc, path)| {
		path.as_ref().is_some_and(|path| {
			path.identifier.starts_with(b"operator") && !FOLLOWED.contains(&path.identifier) && !own_type(path.owner) && waits[proc]
		})
	});
	if operator_waits {
		report.refused = Some("an operator proc may sleep");
		return report;
	}

	// The type each proc is on, and each type's ancestors
	let mut owner: HashMap<u32, u32> = HashMap::new();
	let mut verbs: HashSet<u32> = HashSet::new();
	for (index, ty) in dmb.types.iter().enumerate() {
		if let Some(procs) = dmb.lists.get(ty.procs as usize).filter(|_| ty.procs != NONE) {
			owner.extend(procs.iter().map(|&proc| (proc, index as u32)));
		}
		if let Some(list) = dmb.lists.get(ty.verbs as usize).filter(|_| ty.verbs != NONE) {
			verbs.extend(list.iter().copied());
		}
	}
	let at_or_below = |mut ty: u32, ancestor: u32| {
		for _ in 0..dmb.types.len() {
			if ty == ancestor {
				return true;
			}
			match dmb.types.get(ty as usize) {
				Some(found) if found.parent != NONE && found.parent != ty => ty = found.parent,
				_ => break
			}
		}
		false
	};

	// Every reference to a proc by id or by a name, to see that each is a
	// call this pass can change
	let mut referred: HashMap<Target, usize> = HashMap::new();
	for &list in &uses.code {
		let Some(references) = &uses.references[&list] else {
			continue;
		};
		for reference in references {
			let word = dmb.lists[list as usize][reference.word_offset as usize];
			match reference.kind {
				ReferenceKind::ProcName => *referred.entry(Target::Name(word)).or_default() += 1,
				ReferenceKind::Proc => *referred.entry(Target::Proc(word)).or_default() += 1,
				_ => {}
			}
		}
	}
	// The calls of each target, by the list they are in
	let mut sites: HashMap<Target, Vec<(u32, &Site)>> = HashMap::new();
	for (&list, body) in &bodies {
		for site in &body.sites {
			sites.entry(site.target).or_default().push((list, site));
		}
	}
	// The procs that run each list
	let mut runners: HashMap<u32, Vec<u32>> = HashMap::new();
	for (proc, record) in dmb.procs.iter().enumerate() {
		runners.entry(record.code).or_default().push(proc as u32);
	}

	let mut edits: HashMap<u32, Vec<(u32, u32, Vec<u32>)>> = HashMap::new();
	let mut chosen: Vec<(u32, u32)> = Vec::new();
	for (proc, path) in paths.iter().enumerate() {
		let proc = proc as u32;
		let record = &dmb.procs[proc as usize];
		let Some(path) = path else { continue };
		let Some(&ty) = owner.get(&proc) else { continue };
		let path_text = &dmb.strings[record.path as usize];
		// A declaration on a type (`/type/proc/name`), of an identifier and
		// a name nothing else has
		if path.global() || path.verb || !path_text[path.owner.len()..].starts_with(b"/proc/") || path.identifier.starts_with(b"operator") {
			continue;
		}
		let identifier = strings.get(path.identifier);
		let alone = |procs: Option<&Vec<u32>>| procs.is_some_and(|procs| procs.len() == 1);
		if record.name == NONE || !alone(by_identifier.get(path.identifier)) || !alone(by_string.get(&record.name)) {
			continue;
		}
		if identifier.is_some_and(|identifier| !alone(by_string.get(&identifier))) {
			continue;
		}
		if !plain(record) || verbs.contains(&proc) || uses.proc_values.contains(&proc) {
			continue;
		}
		if !body_of(proc).is_some_and(|body| body.read && !body.uses_src) {
			continue;
		}
		// (a list two procs run would need two bodies)
		if waits[proc as usize] || runners[&record.code].len() != 1 {
			continue;
		}
		// Neither name is text, which `call()` could look it up by, or is
		// in bytecode that could not be read
		let names: Vec<u32> = [Some(record.name), identifier].into_iter().flatten().collect();
		if names.iter().any(|name| uses.text.contains(name) || uses.unread_words.contains(name)) || uses.unread_words.contains(&proc) {
			continue;
		}
		let mut targets: Vec<Target> = names.iter().map(|&name| Target::Name(name)).collect();
		targets.dedup();
		targets.push(Target::Proc(proc));
		let calls: Vec<&(u32, &Site)> = targets.iter().filter_map(|target| sites.get(target)).flatten().collect();
		let references: usize = targets.iter().filter_map(|target| referred.get(target)).sum();
		let changeable = calls.iter().all(|(list, site)| {
			site.on_src
				&& runners[list]
					.iter()
					.all(|caller| owner.get(caller).is_some_and(|&caller_type| at_or_below(caller_type, ty)))
		});
		if !changeable || calls.len() != references {
			continue;
		}
		for (list, site) in calls {
			let mut new = Vec::with_capacity(7);
			if site.sets_cache {
				new.extend_from_slice(&CACHE_SRC);
			}
			if site.arguments == ARGLIST {
				new.extend([CALL_GLOBAL_ARGLIST, proc]);
			} else {
				new.extend([CALL_GLOBAL, site.arguments, proc]);
			}
			edits.entry(*list).or_default().push((site.offset, site.len, new));
		}
		chosen.push((proc, ty));
	}

	// Rewrite the callers. A list that does not come out right is left, and
	// the procs it calls stay where they are.
	let not_code = lists_not_code(dmb);
	let mut rewritten: HashMap<u32, Vec<u32>> = HashMap::new();
	let mut failed: HashSet<u32> = HashSet::new();
	for (&list, list_edits) in &mut edits {
		list_edits.sort_by_key(|(offset, ..)| *offset);
		match splice(&dmb.lists[list as usize], list_edits) {
			Some(words) if words.len() <= usize::from(u16::MAX) => {
				rewritten.insert(list, words);
			}
			_ => failed.extend(list_edits.iter().map(|(_, _, new)| *new.last().unwrap()))
		}
	}
	if !failed.is_empty() {
		// Take nothing in part: a proc is global with all its calls or not
		// at all. Lists that also call a proc left behind are redone.
		chosen.retain(|(proc, _)| !failed.contains(proc));
		rewritten.clear();
		for (&list, list_edits) in &mut edits {
			list_edits.retain(|(_, _, new)| !failed.contains(new.last().unwrap()));
			if list_edits.is_empty() {
				continue;
			}
			match splice(&dmb.lists[list as usize], list_edits) {
				Some(words) if words.len() <= usize::from(u16::MAX) => {
					rewritten.insert(list, words);
				}
				_ => {
					report.refused = Some("a caller's bytecode could not be rewritten");
					return report;
				}
			}
		}
	}

	let kept: HashSet<u32> = chosen.iter().map(|(proc, _)| *proc).collect();
	for (list, words) in rewritten {
		report.calls += edits[&list].iter().filter(|(_, _, new)| kept.contains(new.last().unwrap())).count();
		if not_code.contains(&list) {
			dmb.lists.push(words);
			let copy = dmb.lists.len() as u32 - 1;
			for &proc in &runners[&list] {
				dmb.procs[proc as usize].code = copy;
			}
		} else {
			dmb.lists[list as usize] = words;
		}
	}
	// Take each proc out of its type's list. The list is the type's alone
	// unless another table has the same words.
	let mut list_users: HashMap<u32, usize> = HashMap::new();
	for ty in &dmb.types {
		for list in [ty.procs, ty.verbs, ty.set_vars, ty.declared_vars, ty.builtin_vars, ty.unknown] {
			*list_users.entry(list).or_default() += 1;
		}
	}
	let code: HashSet<u32> = dmb.procs.iter().flat_map(|proc| [proc.code, proc.locals, proc.params]).collect();
	let mut removed: HashMap<u32, Vec<u32>> = HashMap::new();
	for &(proc, ty) in &chosen {
		removed.entry(ty).or_default().push(proc);
		report
			.procs
			.push(String::from_utf8_lossy(&dmb.strings[dmb.procs[proc as usize].path as usize]).into_owned());
	}
	for (ty, procs) in removed {
		let list = dmb.types[ty as usize].procs;
		let left: Vec<u32> = dmb.lists[list as usize].iter().copied().filter(|proc| !procs.contains(proc)).collect();
		if left.is_empty() {
			dmb.types[ty as usize].procs = NONE;
		} else if list_users[&list] > 1 || code.contains(&list) || list == dmb.world.procs {
			dmb.lists.push(left);
			dmb.types[ty as usize].procs = dmb.lists.len() as u32 - 1;
		} else {
			dmb.lists[list as usize] = left;
		}
	}
	report.procs.sort();
	report.after = ProcTables::count(dmb);
	report
}

#[cfg(test)]
mod tests {
	use dmasm::dmb::{Dmb, Proc, Type, NONE};

	use super::{body, run, splice};

	const END: u32 = 0x00;
	const JMP: u32 = 0x0F;
	const JZ: u32 = 0x11;
	const SLEEP: u32 = 0x24;
	const CALL: u32 = 0x29;
	const GET: u32 = 0x33;
	const POP: u32 = 0x51;
	const SRC: u32 = 0xFFCE;
	const LOCAL: u32 = 0xFFDA;
	const SET_CACHE: u32 = 0xFFDC;
	const BY_NAME: u32 = 0xFFDD;

	#[test]
	fn jumps_follow_the_code_around_a_replaced_call() {
		// if(..) { src.<7>(1 argument) }; then back to the start
		let words = [JZ, 8, CALL, SET_CACHE, SRC, BY_NAME, 7, 1, JMP, 0, END];
		assert_eq!(splice(&words, &[(2, 6, vec![0x30, 1, 3])]), Some(vec![JZ, 5, 0x30, 1, 3, JMP, 0, END]));
		// A longer replacement, and a jump to the call itself
		let words = [JMP, 2, CALL, BY_NAME, 7, 0, JZ, 2, END];
		assert_eq!(
			splice(&words, &[(2, 4, vec![GET, SRC, 0x34, 0xFFD8, 0x30, 0, 3])]),
			Some(vec![JMP, 2, GET, SRC, 0x34, 0xFFD8, 0x30, 0, 3, JZ, 2, END])
		);
		// Two calls
		let words = [CALL, BY_NAME, 7, 0, JZ, 10, CALL, BY_NAME, 7, 1, JMP, 4, END];
		assert_eq!(
			splice(&words, &[(0, 4, vec![0x30, 0, 3]), (6, 4, vec![0xCD, 3])]),
			Some(vec![0x30, 0, 3, JZ, 7, 0xCD, 3, JMP, 3, END])
		);
		// A jump into the replaced instruction has nowhere to go
		let words = [JMP, 4, CALL, BY_NAME, 7, 0, END];
		assert_eq!(splice(&words, &[(2, 4, vec![0x30, 0, 3])]), None);
	}

	#[test]
	fn a_call_is_on_src_where_the_cache_is_known_to_hold_it() {
		let sites = |words: &[u32]| -> Vec<(u32, bool, bool)> {
			let body = body(words);
			assert!(body.read);
			body.sites.iter().map(|site| (site.offset, site.on_src, site.sets_cache)).collect()
		};
		// src.<7>(); then <7>() again, the cache still src
		assert_eq!(sites(&[CALL, SET_CACHE, SRC, BY_NAME, 7, 0, POP, CALL, BY_NAME, 7, 0, END]), [
			(0, true, true),
			(7, true, false)
		]);
		// On a local var; then with the cache still that var
		assert_eq!(sites(&[CALL, SET_CACHE, LOCAL, 0, BY_NAME, 7, 0, POP, CALL, BY_NAME, 7, 0, END]), [
			(0, false, true),
			(8, false, false)
		]);
		// src.<var 9>.<7>() leaves the var's value in the cache
		assert_eq!(
			sites(&[CALL, SET_CACHE, SRC, SET_CACHE, 9, BY_NAME, 7, 0, POP, CALL, BY_NAME, 7, 0, END]),
			[(0, false, true), (9, false, false)]
		);
		// Past a jump's target nothing is known of the cache
		assert_eq!(sites(&[CALL, SET_CACHE, SRC, BY_NAME, 7, 0, JZ, 8, CALL, BY_NAME, 7, 0, END]), [
			(0, true, true),
			(8, false, false)
		]);
		// Reading a var of src leaves src in the cache
		assert_eq!(sites(&[GET, SET_CACHE, SRC, 9, POP, CALL, BY_NAME, 7, 0, END]), [(5, true, false)]);

		assert!(body(&[GET, SRC, END]).uses_src);
		assert!(body(&[CALL, SET_CACHE, SRC, BY_NAME, 7, 0, END]).uses_src);
		assert!(!body(&[GET, LOCAL, 0, END]).uses_src);
		assert!(body(&[SLEEP, END]).waits);
		assert!(!body(&[0xFFFF_0000]).read);
	}

	fn string(dmb: &mut Dmb, text: &str) -> u32 {
		if let Some(id) = dmb.strings.iter().position(|string| string == text.as_bytes()) {
			return id as u32;
		}
		dmb.strings.push(text.as_bytes().to_vec());
		dmb.strings.len() as u32 - 1
	}

	fn proc(dmb: &mut Dmb, path: &str, code: Vec<u32>) -> u32 {
		let identifier = path.rsplit('/').next().unwrap().to_owned();
		let (path, name) = (string(dmb, path), string(dmb, &identifier));
		dmb.lists.push(code);
		dmb.procs.push(Proc {
			path,
			name,
			desc: NONE,
			category: NONE,
			src_range: 0xFF,
			src_kind: 0,
			flags: 0x04,
			flags_ex: None,
			code: dmb.lists.len() as u32 - 1,
			locals: NONE,
			params: NONE
		});
		dmb.procs.len() as u32 - 1
	}

	fn ty(dmb: &mut Dmb, path: &str, parent: u32, procs: Vec<u32>) -> u32 {
		let path = string(dmb, path);
		dmb.lists.push(procs);
		dmb.types.push(Type {
			path,
			parent,
			procs: dmb.lists.len() as u32 - 1,
			verbs: NONE,
			set_vars: NONE,
			declared_vars: NONE,
			builtin_vars: NONE,
			unknown: NONE,
			..Type::default()
		});
		dmb.types.len() as u32 - 1
	}

	/// `/datum/a/proc/helper`, which returns its argument, and
	/// `/datum/a/sub/proc/caller`, which calls it on src.
	fn world(helper_code: Vec<u32>) -> (Dmb, u32, u32) {
		let mut dmb = Dmb::default();
		dmb.world.procs = NONE;
		let name = string(&mut dmb, "helper");
		let helper = proc(&mut dmb, "/datum/a/proc/helper", helper_code);
		let caller = proc(&mut dmb, "/datum/a/sub/proc/caller", vec![
			CALL, SET_CACHE, SRC, BY_NAME, name, 1, 0x12, END,
		]);
		let a = ty(&mut dmb, "/datum/a", NONE, vec![helper]);
		ty(&mut dmb, "/datum/a/sub", a, vec![caller]);
		(dmb, helper, caller)
	}

	fn returns_argument() -> Vec<u32> {
		vec![GET, 0xFFD9, 0, 0x12, END]
	}

	#[test]
	fn a_proc_that_never_uses_src_is_called_as_a_global_proc() {
		let (mut dmb, helper, caller) = world(returns_argument());
		let report = run(&mut dmb);
		assert_eq!(
			(report.procs, report.calls, report.refused),
			(vec!["/datum/a/proc/helper".to_owned()], 1, None)
		);
		assert_eq!(dmb.lists[dmb.procs[caller as usize].code as usize], [
			GET, SRC, 0x34, 0xFFD8, 0x30, 1, helper, 0x12, END
		]);
		// Off its type, and with the path it had
		assert_eq!(dmb.types[0].procs, NONE);
		assert_eq!(dmb.strings[dmb.procs[helper as usize].path as usize], b"/datum/a/proc/helper");
		assert_eq!((report.before.names, report.after.names), (3, 1));

		let again = run(&mut dmb.clone());
		assert!(again.procs.is_empty() && again.calls == 0);
	}

	#[test]
	fn a_proc_stays_on_its_type_when_a_rule_is_broken() {
		let kept = |dmb: &mut Dmb| {
			let before = dmb.clone();
			let report = run(dmb);
			assert!(report.procs.is_empty() && report.calls == 0 && report.refused.is_none());
			assert!(*dmb == before, "a world with no proc to make global was changed");
		};
		// It uses src, or can sleep
		kept(&mut world(vec![GET, SRC, 0x12, END]).0);
		kept(&mut world(vec![SLEEP, END]).0);
		// It calls something that can sleep
		let (mut dmb, ..) = world(vec![0x30, 0, 2, END]);
		proc(&mut dmb, "/proc/sleeper", vec![SLEEP, END]);
		kept(&mut dmb);
		// It is overridden, or another type has the identifier
		let (mut dmb, ..) = world(returns_argument());
		let over = proc(&mut dmb, "/datum/a/sub/helper", vec![END]);
		let procs = dmb.types[1].procs as usize;
		dmb.lists[procs].push(over);
		kept(&mut dmb);
		let (mut dmb, ..) = world(returns_argument());
		let other = proc(&mut dmb, "/datum/b/proc/helper", vec![END]);
		ty(&mut dmb, "/datum/b", NONE, vec![other]);
		kept(&mut dmb);
		// It is called on something other than src, or from a global proc
		// or an unrelated type
		let (mut dmb, ..) = world(returns_argument());
		let name = string(&mut dmb, "helper");
		proc(&mut dmb, "/proc/calls", vec![CALL, SET_CACHE, LOCAL, 0, BY_NAME, name, 1, END]);
		kept(&mut dmb);
		let (mut dmb, ..) = world(returns_argument());
		proc(&mut dmb, "/proc/calls", vec![CALL, SET_CACHE, SRC, BY_NAME, name, 1, END]);
		kept(&mut dmb);
		let (mut dmb, ..) = world(returns_argument());
		let stranger = proc(&mut dmb, "/datum/b/proc/calls", vec![CALL, SET_CACHE, SRC, BY_NAME, name, 1, END]);
		ty(&mut dmb, "/datum/b", NONE, vec![stranger]);
		kept(&mut dmb);
		// Its name is text, or its path a value
		let (mut dmb, ..) = world(returns_argument());
		proc(&mut dmb, "/proc/says", vec![0x60, 0x06, name, END]);
		kept(&mut dmb);
		let (mut dmb, helper, _) = world(returns_argument());
		proc(&mut dmb, "/proc/takes", vec![0x60, 0x26, helper, END]);
		kept(&mut dmb);
		// It has settings, or is an operator
		let (mut dmb, helper, _) = world(returns_argument());
		dmb.procs[helper as usize].flags = 0x80;
		dmb.procs[helper as usize].flags_ex = Some((0x184, 0));
		kept(&mut dmb);
		// A caller's bytecode cannot be read
		let (mut dmb, ..) = world(returns_argument());
		proc(&mut dmb, "/proc/unknown", vec![0xFFFF_0000, name]);
		kept(&mut dmb);
	}
}
