//! `stop` and `reboot`: the world's own ending, and its restart, asked for
//! as an editor's terminate asks: by evaluating `world.Del()` or
//! `world.Reboot()` in it. Neither is answered when it works (the world is
//! gone, or lets its clients go), so what's waited for is the process going,
//! or the world answering again.

use std::time::{Duration, Instant};

use auxtools_cli::{
	local,
	server_types::Request
};

use super::{Failure, Flags, Result, World, HUNG_UP, TIMED_OUT};

/// How often to look again
const INTERVAL: Duration = Duration::from_millis(50);

/// `--timeout` in seconds, or `default`
fn timeout(flags: &Flags, default: f64) -> Result<Duration> {
	let seconds = flags.number::<f64>("--timeout")?.unwrap_or(default);
	if !(seconds.is_finite() && seconds >= 0.0) {
		return Err(format!("--timeout {} isn't a number of seconds", seconds).into());
	}
	Ok(Duration::from_secs_f64(seconds))
}

/// Has the world evaluate `expression`, without waiting for an answer
fn ask(world: &mut World, expression: &str) -> Result<()> {
	let request = Request::Eval {
		frame_id: None,
		command: expression.to_owned(),
		context: None
	};
	local::send(&mut world.stream, &request).map_err(|_| super::hung_up())
}

/// Whether process `pid` is gone by `until`
fn gone_by(pid: u32, until: Instant) -> bool {
	loop {
		if !local::alive(pid) {
			return true;
		}
		if Instant::now() >= until {
			return false;
		}
		std::thread::sleep(INTERVAL);
	}
}

pub fn stop(world: &mut World, args: &[String]) -> Result<()> {
	let flags = Flags::parse(args, &["--timeout"], &["--kill"])?;
	flags.nothing_else("stop")?;
	let timeout = timeout(&flags, 10.0)?;
	let pid = world.pid;
	ask(world, "world.Del()")?;
	if gone_by(pid, Instant::now() + timeout) {
		println!("The world has ended (process {})", pid);
		return Ok(());
	}
	if !flags.has("--kill") {
		return Err(format!(
			"the world (process {}) is still running {} s on: its world/Del() didn't end it. The debugger shuts down \
			 in world/Del() all the same, so other commands may not reach it now (`stop --kill` ends the process)",
			pid,
			timeout.as_secs_f64()
		)
		.into());
	}
	local::kill(pid);
	if gone_by(pid, Instant::now() + Duration::from_secs(5)) {
		println!(
			"The world didn't end in {} s, so its process ({}) was ended",
			timeout.as_secs_f64(),
			pid
		);
		Ok(())
	} else {
		Err(format!("couldn't end process {}", pid).into())
	}
}

pub fn reboot(world: &mut World, args: &[String]) -> Result<()> {
	let flags = Flags::parse(args, &["--timeout"], &[])?;
	flags.nothing_else("reboot")?;
	let timeout = timeout(&flags, 60.0)?;
	let pid = world.pid;
	let started = Instant::now();
	let until = started + timeout;
	ask(world, "world.Reboot()")?;

	// The world lets its clients go as it reboots
	loop {
		match world.next(Some(until)) {
			Ok(Some(_)) => {}
			Ok(None) => {
				return Err(Failure(
					TIMED_OUT,
					format!("the world didn't reboot in {} s", timeout.as_secs_f64())
				))
			}
			Err(Failure(HUNG_UP, _)) => break,
			Err(e) => return Err(e)
		}
	}

	// One that connects while the world loads is answered once it has
	loop {
		if !local::alive(pid) {
			return Err(format!("the world (process {}) ended rather than rebooting", pid).into());
		}
		if Instant::now() >= until {
			return Err(Failure(
				TIMED_OUT,
				format!("the world rebooted, but wasn't answering {} s on", timeout.as_secs_f64())
			));
		}
		if let Ok(mut again) = World::connect(pid) {
			// Answering, and let run by its launcher: a world that has only
			// loaded isn't back yet
			let answered = again.started().unwrap_or(false);
			again.leave();
			if answered {
				println!(
					"The world rebooted and is answering again, {:.1} s on (process {})",
					started.elapsed().as_secs_f64(),
					pid
				);
				return Ok(());
			}
		}
		std::thread::sleep(INTERVAL);
	}
}
