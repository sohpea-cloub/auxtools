use std::sync::atomic::Ordering;

use crate::profile::{self, time, Listed};

use super::{
	trace::{interruptible, INTERRUPTED, TICK},
	*
};

pub fn profile(world: &mut World, args: &args::Profile) -> Result<()> {
	let procs = args.procs.iter().map(|proc| proc_ref(proc)).collect::<Result<Vec<_>>>()?;
	let named = !procs.is_empty();
	world
		.profile(ProfileRequest::Start { procs })
		.map_err(|Failure(code, why)| Failure(code, format!("cannot start a profile: {}.", why.trim_end_matches('.'))))?;
	if args.duration.is_none() {
		eprintln!("profiling until interrupted");
	}

	interruptible();
	let until = args.duration.map(|time| Instant::now() + time);
	let ended = loop {
		if INTERRUPTED.load(Ordering::SeqCst) {
			break "interrupted";
		}
		if until.is_some_and(|until| Instant::now() >= until) {
			break "time";
		}
		let tick = Instant::now() + TICK;
		world.next(Some(until.map_or(tick, |until| until.min(tick))))?;
	};

	let listed = (!named).then_some(args.top as usize);
	let report = profile::stop(ended, listed, |request| world.request(request))?;

	if let Some(file) = &args.html {
		std::fs::write(file, profile::html(&report)).map_err(|e| format!("cannot write {}: {}.", file.display(), e))?;
	}
	if args.json {
		say!("{}", report.json());
		return Ok(());
	}

	say!(
		"profiled {:.1} s: {} instructions took {} in {} procs",
		report.seconds,
		report.hits,
		time(report.own_ns),
		report.procs.len()
	);
	if report.procs.is_empty() {
		return Ok(());
	}
	say!("");
	say!(
		"{:>9}  {:>5}  {:>10}  {:>9}  {:>12}  proc",
		"time",
		"%",
		"with calls",
		"calls",
		"instructions"
	);
	for listed in report.procs.iter().take(if named { usize::MAX } else { args.top as usize }) {
		let total = listed.code.total;
		say!(
			"{:>9}  {:>5.1}  {:>10}  {:>9}  {:>12}  {}",
			time(total.own_ns),
			report.share(total.own_ns),
			time(total.own_ns + total.callee_ns),
			total.calls,
			total.hits,
			spot(
				&listed.code.proc,
				listed.code.code.old != 0,
				listed.code.file.as_deref(),
				listed.code.line
			)
		);
	}

	if named {
		for listed in &report.procs {
			say!("");
			instructions(listed);
		}
		return Ok(());
	}
	let lines = report.lines();
	if !lines.is_empty() {
		say!("");
		say!("{:>9}  {:>5}  {:>10}  {:>12}  line", "time", "%", "with calls", "instructions");
		for line in lines.iter().take(args.top as usize) {
			say!(
				"{:>9}  {:>5.1}  {:>10}  {:>12}  {}",
				time(line.own_ns),
				report.share(line.own_ns),
				time(line.own_ns + line.callee_ns),
				line.hits,
				line.place
			);
		}
	}
	Ok(())
}

fn instructions(listed: &Listed) {
	let code = &listed.code;
	say!("{}", spot(&code.proc, code.code.old != 0, code.file.as_deref(), None));
	say!(
		"{:>6}  {:>10}  {:>5}  {:>8}  {:>10}  instruction",
		"offset",
		"hits",
		"%",
		"ns/hit",
		"calls/hit"
	);
	let mut line = None;
	for row in listed.listing.iter().flatten() {
		if row.line != line && row.line.is_some() {
			line = row.line;
			match &row.source {
				Some(source) => say!("line {}:  {}", line.unwrap(), source),
				None => say!("line {}:", line.unwrap())
			}
		}
		if row.hits == 0 {
			say!(
				"{:>6}  {:>10}  {:>5}  {:>8}  {:>10}  {}",
				format!("{:04X}", row.offset),
				"",
				"",
				"",
				"",
				row.text
			);
			continue;
		}
		let share = match code.total.own_ns {
			0 => 0.0,
			own => row.own_ns as f64 * 100.0 / own as f64
		};
		let calls = match row.callee_ns {
			0 => String::new(),
			callee => time(callee / row.hits)
		};
		say!(
			"{:>6}  {:>10}  {:>5.1}  {:>8.1}  {:>10}  {}",
			format!("{:04X}", row.offset),
			row.hits,
			share,
			row.own_ns as f64 / row.hits as f64,
			calls,
			row.text
		);
	}
}

impl World {
	fn profile(&mut self, request: ProfileRequest) -> Result<ProfileReply> {
		match self.request(Request::Profile(request))? {
			Response::Profile(reply) => Ok(reply?),
			other => Err(unexpected(&other))
		}
	}
}
