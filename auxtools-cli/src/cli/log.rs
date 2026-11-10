//! The world's record of what happened in it (the debug server's journal):
//! `log` prints it, and how its entries read, as text and as JSON.

use std::{
	collections::VecDeque,
	io::Write,
	time::{Duration, Instant}
};

use auxtools_cli::{local, server_types::*};
use serde_json::json;

use super::{hung_up, unexpected, Failure, Flags, Result, World, HUNG_UP, TIMED_OUT};

/// `auxtools log`
pub fn log(world: &mut World, args: &[String]) -> Result<()> {
	let flags = Flags::parse(
		args,
		&["--since", "--last"],
		&["--follow", "--runtimes", "--reloads", "--output", "--world", "--json"]
	)?;
	flags.nothing_else("log")?;
	let kinds = kinds(&flags);
	let json = flags.has("--json");
	let since = flags.number::<u64>("--since")?;
	let last = flags.number::<u32>("--last")?;

	let (entries, first, newest) = match world.request(Request::Journal {
		after: since.unwrap_or(0),
		kinds: kinds.clone(),
		limit: last,
		follow: false
	})? {
		Response::Journal { entries, first, last } => (entries, first, last),
		other => return Err(unexpected(&other))
	};
	if let Some(since) = since.filter(|since| first > since + 1) {
		eprintln!("entries {} to {} are gone: the world keeps the last few thousand", since + 1, first - 1);
	}
	for entry in &entries {
		if !print(entry, json) {
			return Ok(());
		}
	}
	if !flags.has("--follow") {
		return Ok(());
	}

	// From where that left off, across reboots, until the world ends
	let mut follower = Follower::new(newest.max(since.unwrap_or(0)), kinds);
	loop {
		let Some(entry) = follower.next(world, None)? else {
			continue;
		};
		if !print(&entry, json) {
			return Ok(());
		}
	}
}

/// `wait --reload` and `wait --reboot`, which wait on the world's record
/// rather than for it to pause. `None` if neither is asked for.
pub fn wait_on_record(world: &mut World, args: &[String]) -> Option<Result<()>> {
	let reload = args.iter().any(|arg| arg == "--reload");
	let reboot = args.iter().any(|arg| arg == "--reboot");
	if !reload && !reboot {
		return None;
	}
	Some((|| {
		let what = if reload { "--reload" } else { "--reboot" };
		let ours = ["--reload", "--reboot", "--timeout", "--since", "--json"];
		if let Some(other) = args.iter().find(|arg| arg.starts_with("--") && !ours.contains(&arg.as_str())) {
			return Err(format!("{} doesn't go with {}", other, what).into());
		}
		if reload && reboot {
			return Err("wait for --reload or --reboot, not both".into());
		}
		let flags = Flags::parse(args, &["--timeout", "--since"], &["--reload", "--reboot", "--json"])?;
		flags.nothing_else("wait")?;
		let until = flags.number::<f64>("--timeout")?.map(|t| Instant::now() + Duration::from_secs_f64(t));
		let since = flags.number::<u64>("--since")?;
		if reload {
			wait_reload(world, since, until, flags.has("--json"))
		} else {
			wait_reboot(world, since, until, flags.has("--json"))
		}
	})())
}

/// Waits for a hot reload to be tried (the next, or the first after entry
/// `since`), and prints what came of it: exit code 1 if it wasn't applied
fn wait_reload(world: &mut World, since: Option<u64>, until: Option<Instant>, json: bool) -> Result<()> {
	let mut follower = Follower::new(since.unwrap_or(u64::MAX), vec![JournalKind::Reload]);
	let Some(entry) = follower.next(world, until)? else {
		return Err(Failure(TIMED_OUT, "timed out: no hot reload was tried".to_owned()));
	};
	print(&entry, json);
	match entry.event {
		JournalEvent::Reload { applied: true, .. } => Ok(()),
		_ => Err("the hot reload wasn't applied".into())
	}
}

/// Waits for the world to reboot (or for the first load after entry
/// `since`), and for the new world to answer
fn wait_reboot(world: &mut World, since: Option<u64>, until: Option<Instant>, json: bool) -> Result<()> {
	let mut follower = Follower::new(since.unwrap_or(u64::MAX), vec![JournalKind::World]);
	loop {
		let Some(entry) = follower.next(world, until)? else {
			return Err(Failure(TIMED_OUT, "timed out: the world didn't reboot".to_owned()));
		};
		if let JournalEvent::Loaded { .. } = entry.event {
			print(&entry, json);
			return Ok(());
		}
	}
}

/// The kinds of entries the flags ask for: all of them, if none are named
pub fn kinds(flags: &Flags) -> Vec<JournalKind> {
	[
		("--runtimes", JournalKind::Runtime),
		("--reloads", JournalKind::Reload),
		("--output", JournalKind::Output),
		("--world", JournalKind::World)
	]
	.into_iter()
	.filter(|(flag, _)| flags.has(flag))
	.map(|(_, kind)| kind)
	.collect()
}

/// Prints an entry. False once nothing reads what's printed (a closed pipe),
/// which ends the command quietly.
pub fn print(entry: &JournalEntry, json: bool) -> bool {
	let text = if json { to_json(entry).to_string() } else { block(entry) };
	let mut out = std::io::stdout().lock();
	writeln!(out, "{}", text).and_then(|()| out.flush()).is_ok()
}

/// The entries of the record as they're made, from a number on, of some
/// kinds. A reboot hangs up on every client: this connects again, and goes
/// on from the last entry it had.
pub struct Follower {
	/// The number of the last entry had, `u64::MAX` for "from now"
	after: u64,
	kinds: Vec<JournalKind>,
	/// Entries read, and not handed out yet
	waiting: VecDeque<JournalEntry>,
	/// Whether this connection has asked for them
	asked: bool
}

impl Follower {
	pub const fn new(after: u64, kinds: Vec<JournalKind>) -> Self {
		Self {
			after,
			kinds,
			waiting: VecDeque::new(),
			asked: false
		}
	}

	/// The next entry. `None` when `until` passes first. A world that ends
	/// is `HUNG_UP`.
	pub fn next(&mut self, world: &mut World, until: Option<Instant>) -> Result<Option<JournalEntry>> {
		loop {
			if let Some(entry) = self.waiting.pop_front() {
				self.after = entry.number;
				return Ok(Some(entry));
			}
			if !self.asked {
				let asked = Request::Journal {
					after: self.after,
					kinds: self.kinds.clone(),
					limit: None,
					follow: true
				};
				if local::send(&mut world.stream, &asked).is_err() {
					world.reconnect(until)?;
					continue;
				}
				self.asked = true;
			}
			match world.next(until) {
				Ok(None) => return Ok(None),
				Ok(Some(Response::Journal { entries, last, .. })) => {
					if self.after == u64::MAX {
						self.after = last;
					}
					self.waiting.extend(entries);
				}
				Ok(Some(Response::Journaled(entry))) if entry.number > self.after => self.waiting.push_back(entry),
				Ok(Some(_)) => {}
				Err(Failure(HUNG_UP, _)) => {
					self.asked = false;
					world.reconnect(until)?;
				}
				Err(e) => return Err(e)
			}
		}
	}
}

/// How often to try a world's socket while it reboots
const RECONNECT_INTERVAL: Duration = Duration::from_millis(100);

impl World {
	/// Connects again to the world, which hung up: it rebooted, or ended
	/// (`HUNG_UP`). One that connects while the world reboots is taken by the
	/// next world.
	pub fn reconnect(&mut self, until: Option<Instant>) -> Result<()> {
		loop {
			if let Ok(world) = World::connect(self.pid) {
				*self = world;
				return Ok(());
			}
			if !local::alive(self.pid) {
				return Err(hung_up());
			}
			if until.is_some_and(|until| Instant::now() >= until) {
				return Err(Failure(TIMED_OUT, "timed out: the world hung up, and didn't come back".to_owned()));
			}
			std::thread::sleep(RECONNECT_INTERVAL);
		}
	}
}

/// An entry as text: a line that starts with its number, then what there is
/// to say about it, each line of that indented
pub fn block(entry: &JournalEntry) -> String {
	let (what, more) = describe(&entry.event);
	let mut text = format!(
		"#{} {} (world.time {}) {}",
		entry.number,
		clock(entry.time),
		number(entry.world_time),
		what
	);
	for line in more {
		text += "\n  ";
		text += &line;
	}
	text
}

/// What happened, in a line, and the lines that go under it
pub fn describe(event: &JournalEvent) -> (String, Vec<String>) {
	match event {
		JournalEvent::Runtime { message, caught, stack } => {
			let kind = if *caught { "caught runtime error" } else { "runtime error" };
			(format!("{}: {}", kind, message), stack.iter().map(frame).collect())
		}
		JournalEvent::Reload {
			applied,
			summary,
			errors,
			warnings
		} => (
			format!("hot reload {}: {}", if *applied { "applied" } else { "not applied" }, summary),
			errors
				.iter()
				.map(|e| format!("error: {}", e))
				.chain(warnings.iter().map(|w| format!("warning: {}", w)))
				.collect()
		),
		JournalEvent::Loaded { load: 1 } => ("the world loaded".to_owned(), vec![]),
		JournalEvent::Loaded { load } => (format!("the world rebooted (world {} in this process)", load), vec![]),
		JournalEvent::Output { lines } => ("output".to_owned(), lines.clone()),
		JournalEvent::HoldEnded { seconds } => (
			format!(
				"the world carried on by itself: its pause was held, and nobody debugging came back to it in {} s (--hold-max)",
				number(*seconds)
			),
			vec![]
		)
	}
}

fn frame(frame: &JournalFrame) -> String {
	match (&frame.file, frame.line) {
		(Some(file), Some(line)) => format!("{}  {}:{}", frame.proc, file, line),
		(None, Some(line)) => format!("{}  line {}", frame.proc, line),
		(Some(file), None) => format!("{}  {}", frame.proc, file),
		(None, None) => frame.proc.clone()
	}
}

/// A number without a fraction if it has none
fn number(n: f64) -> String {
	if n.fract() == 0.0 && n.abs() < 1e15 {
		format!("{}", n as i64)
	} else {
		format!("{:.1}", n)
	}
}

/// The time of day of a time in milliseconds since 1970, in UTC
pub fn clock(ms: u64) -> String {
	let of_day = ms % 86_400_000;
	format!(
		"{:02}:{:02}:{:02}.{:03} UTC",
		of_day / 3_600_000,
		of_day / 60_000 % 60,
		of_day / 1000 % 60,
		of_day % 1000
	)
}

/// An entry as one JSON object: `number`, `time` (milliseconds since 1970),
/// `world_time` (deciseconds), `kind`, and what that kind has
pub fn to_json(entry: &JournalEntry) -> serde_json::Value {
	let mut value = match &entry.event {
		JournalEvent::Runtime { message, caught, stack } => {
			let top = stack.first();
			json!({
				"kind": "runtime",
				"message": message,
				"caught": caught,
				"proc": top.map(|frame| &frame.proc),
				"file": top.and_then(|frame| frame.file.as_ref()),
				"line": top.and_then(|frame| frame.line),
				"stack": stack.iter().map(|frame| json!({ "proc": frame.proc, "file": frame.file, "line": frame.line })).collect::<Vec<_>>()
			})
		}
		JournalEvent::Reload {
			applied,
			summary,
			errors,
			warnings
		} => json!({ "kind": "reload", "applied": applied, "summary": summary, "errors": errors, "warnings": warnings }),
		JournalEvent::Loaded { load } => json!({ "kind": "world", "event": "loaded", "load": load }),
		JournalEvent::Output { lines } => json!({ "kind": "output", "lines": lines }),
		JournalEvent::HoldEnded { seconds } => json!({ "kind": "world", "event": "hold_ended", "seconds": seconds })
	};
	value["number"] = entry.number.into();
	value["time"] = entry.time.into();
	value["world_time"] = entry.world_time.into();
	value
}

#[cfg(test)]
mod tests {
	use auxtools_cli::server_types::{JournalEntry, JournalEvent, JournalFrame};

	use super::{block, clock, to_json};

	fn runtime() -> JournalEntry {
		JournalEntry {
			number: 7,
			time: 86_400_000 + 3_723_004,
			world_time: 45.0,
			event: JournalEvent::Runtime {
				message: "cannot read from list".to_owned(),
				caught: false,
				stack: vec![
					JournalFrame {
						proc: "/fail".to_owned(),
						file: Some("code.dm".to_owned()),
						line: Some(30)
					},
					JournalFrame {
						proc: "/run".to_owned(),
						file: None,
						line: None
					},
				]
			}
		}
	}

	#[test]
	fn the_time_is_of_the_day_in_utc() {
		assert_eq!(clock(86_400_000 + 3_723_004), "01:02:03.004 UTC");
	}

	#[test]
	fn a_runtime_is_a_line_and_its_stack() {
		assert_eq!(
			block(&runtime()),
			"#7 01:02:03.004 UTC (world.time 45) runtime error: cannot read from list\n  /fail  code.dm:30\n  /run"
		);
	}

	#[test]
	fn a_runtime_in_json_says_where_it_was() {
		let value = to_json(&runtime());
		assert_eq!(value["kind"], "runtime");
		assert_eq!(value["number"], 7);
		assert_eq!((&value["proc"], &value["file"], &value["line"]), (&"/fail".into(), &"code.dm".into(), &30.into()));
		assert_eq!(value["stack"][1]["proc"], "/run");
	}

	#[test]
	fn a_reload_lists_its_errors_and_warnings() {
		let entry = JournalEntry {
			event: JournalEvent::Reload {
				applied: false,
				summary: "couldn't read the code".to_owned(),
				errors: vec!["a.dm, line 3: bad".to_owned()],
				warnings: vec!["careful".to_owned()]
			},
			..runtime()
		};
		assert!(block(&entry).ends_with("hot reload not applied: couldn't read the code\n  error: a.dm, line 3: bad\n  warning: careful"));
	}
}
