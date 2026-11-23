use std::time::{Duration, Instant};

use auxtools_cli::{
	client::{ops::Ending, when_listening, NotListening},
	local
};
use auxtools_protocol::{Reconnected, Request, Response};
use serde_json::json;

use super::{args, seconds_text, Failure, Result, World, HUNG_UP, TIMED_OUT};

const INTERVAL: Duration = Duration::from_millis(50);

const RUNTIME_ERROR: &str = "runtime error: ";

fn ask(world: &mut World, ending: Ending, until: Instant) -> Result<bool> {
	Ok(world.send(&ending.request(), Some(until))?.is_some())
}

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

enum Ended {
	Exited,
	TimedOut { disconnected: bool }
}

fn ended_by(world: &mut World, until: Instant) -> Result<Ended> {
	loop {
		if !local::alive(world.pid) {
			return Ok(Ended::Exited);
		}
		let now = Instant::now();
		if now >= until {
			return Ok(Ended::TimedOut { disconnected: false });
		}
		match world.next(Some(until.min(now + INTERVAL))) {
			Ok(Some(Response::Eval(Err(error)))) if !error.message.starts_with(RUNTIME_ERROR) => {}
			Ok(Some(Response::Eval(_))) => {
				return Err(format!(
					"world.Del() failed with a runtime error. World process {} is still running. See the world's log for the error. Fix \
					 world/Del(), or run stop again with --kill.",
					world.pid
				)
				.into())
			}
			Ok(_) => {}
			Err(e) if world.unavailable.is_some() => return Err(e),
			Err(_) if gone_by(world.pid, until) => return Ok(Ended::Exited),
			Err(_) => return Ok(Ended::TimedOut { disconnected: true })
		}
	}
}

fn done(as_json: bool, document: serde_json::Value, line: String) {
	if as_json {
		say!("{}", document);
	} else {
		say!("{}", line);
	}
}

pub fn stop(world: &mut World, args: &args::Stop) -> Result<()> {
	let timeout = args.timeout.unwrap_or(args::DEFAULT_STOP_TIMEOUT);
	let kill_timeout = args.kill_timeout.unwrap_or(args::DEFAULT_KILL_TIMEOUT);
	let pid = world.pid;
	let until = Instant::now() + timeout;
	match ask(world, Ending::Stop, until) {
		Ok(_) => {}
		Err(Failure(HUNG_UP, _)) if !local::alive(pid) => {}
		Err(Failure(HUNG_UP, _)) => {
			return Err(format!("cannot send the shutdown request: the connection to the world with pid {} ended.", pid).into())
		}
		Err(e) => return Err(e)
	}
	let (failed, disconnected) = match ended_by(world, until) {
		Ok(Ended::Exited) => {
			done(
				args.json,
				json!({ "pid": pid, "stopped": true, "killed": false }),
				format!("pid {}: stopped", pid)
			);
			return Ok(());
		}
		Ok(Ended::TimedOut { disconnected }) => (None, disconnected),
		Err(_) if world.unavailable.is_some() => (None, false),
		Err(e) if args.kill => (Some(e), false),
		Err(e) => return Err(e)
	};
	if !args.kill && world.unavailable.is_none() {
		let lost = if disconnected {
			" The connection to the debugger ended before the process exited."
		} else {
			""
		};
		return Err(Failure(
			TIMED_OUT,
			format!(
				"timed out after {} waiting for world process {} to exit.{} To terminate the process, run stop again with --kill.",
				seconds_text(timeout),
				pid,
				lost
			)
		));
	}
	local::kill(pid);
	if gone_by(pid, Instant::now() + kill_timeout) {
		match (&world.unavailable, failed) {
			(Some(why), _) => eprintln!(
				"debugger initialization failed: {}. world.Del() could not run, so the process was terminated.",
				why
			),
			(None, Some(_)) => eprintln!("world.Del() failed with a runtime error, so the process was terminated."),
			(None, None) => eprintln!(
				"graceful shutdown timed out after {}, so the process was terminated.",
				seconds_text(timeout)
			)
		}
		done(
			args.json,
			json!({ "pid": pid, "stopped": true, "killed": true }),
			format!("pid {}: stopped (killed)", pid)
		);
		Ok(())
	} else {
		Err(Failure(
			TIMED_OUT,
			format!(
				"timed out after {} waiting for terminated process {} to exit.",
				seconds_text(kill_timeout),
				pid
			)
		))
	}
}

pub fn reconnect(world: &mut World, args: &args::Reconnect) -> Result<()> {
	match world.request(Request::Reconnect)? {
		Response::Reconnect(Ok(Reconnected::Clients(clients))) => {
			done(
				args.json,
				json!({ "pid": world.pid, "reconnected": clients }),
				format!(
					"pid {}: asked {} {} to reconnect",
					world.pid,
					clients,
					if clients == 1 { "client" } else { "clients" }
				)
			);
			Ok(())
		}
		Response::Reconnect(Ok(Reconnected::Hosted)) => {
			if !args.json {
				eprintln!("DreamSeeker hosts the world, so the world is rebooted instead.");
			}
			reboot(world, &args::Reboot {
				timeout: args.timeout,
				json: args.json
			})
		}
		Response::Reconnect(Err(e)) => Err(format!("cannot reconnect the clients: {}.", e).into()),
		other => Err(format!("unexpected response: {:?}.", other).into())
	}
}

pub fn reboot(world: &mut World, args: &args::Reboot) -> Result<()> {
	let timeout = args.timeout.unwrap_or(args::DEFAULT_REBOOT_TIMEOUT);
	let pid = world.pid;
	let until = Instant::now() + timeout;
	ask(world, Ending::Reboot, until)?;

	loop {
		match world.next(Some(until)) {
			Ok(Some(_)) => {}
			Ok(None) => {
				return Err(Failure(
					TIMED_OUT,
					format!("timed out after {} waiting for the world to reboot.", seconds_text(timeout))
				))
			}
			Err(Failure(HUNG_UP, _)) => break,
			Err(e) => return Err(e)
		}
	}

	let exited = || Failure(HUNG_UP, format!("world process {} exited instead of rebooting.", pid));
	let never_ran = || {
		Failure(
			TIMED_OUT,
			format!(
				"timed out after {} waiting for the rebooted world to start running.",
				seconds_text(timeout)
			)
		)
	};
	loop {
		let mut again = match when_listening(pid, Some(until), World::connect) {
			Ok(world) => world,
			Err(NotListening::Gone) => return Err(exited()),
			Err(NotListening::TimedOut) => return Err(never_ran())
		};
		loop {
			let lost = match again.started(Some(until)) {
				Ok(Some(true)) => {
					again.leave();
					done(args.json, json!({ "pid": pid, "rebooted": true }), format!("pid {}: rebooted", pid));
					return Ok(());
				}
				Ok(_) => false,
				Err(_) => true
			};
			if !local::alive(pid) {
				return Err(exited());
			}
			if Instant::now() >= until {
				return Err(never_ran());
			}
			std::thread::sleep(INTERVAL);
			if lost {
				break;
			}
		}
	}
}
