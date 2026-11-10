//! `--hold-max`: a limit on how long a pause a command leaves behind lasts
//! with nobody debugging. A paused world stops every player, and an agent
//! that dies in the middle of debugging would leave it stopped for good.

use auxtools_cli::server_types::{Request, Response};

use super::{unexpected, Failure, Result, World};

/// The commands that leave the world paused after they've gone
const HOLDING: [&str; 5] = ["pause", "wait", "next", "step", "out"];

/// What `--hold-max` is when it isn't given
pub const HOLD_MAX_VAR: &str = "AUXTOOLS_HOLD_MAX";

/// Takes `--hold-max SECONDS` out of a command's arguments, for a command
/// that holds a pause; else `AUXTOOLS_HOLD_MAX`, if it's set
pub fn take(command: &str, args: &[String]) -> Result<(Vec<String>, Option<f64>)> {
	if !HOLDING.contains(&command) {
		return Ok((args.to_vec(), None));
	}
	let mut rest = vec![];
	let mut given = None;
	let mut args = args.iter();
	while let Some(arg) = args.next() {
		if arg == "--hold-max" {
			given = Some(args.next().ok_or("--hold-max needs a value")?.clone());
		} else {
			rest.push(arg.clone());
		}
	}
	let seconds = match given {
		Some(text) => Some(seconds(&text, "--hold-max")?),
		None => match std::env::var(HOLD_MAX_VAR) {
			Ok(text) if !text.trim().is_empty() => Some(seconds(&text, HOLD_MAX_VAR)?),
			_ => None
		}
	};
	Ok((rest, seconds))
}

fn seconds(text: &str, name: &str) -> Result<f64> {
	match text.trim().parse::<f64>() {
		Ok(seconds) if seconds.is_finite() && seconds >= 0.0 => Ok(seconds),
		_ => Err(Failure(1, format!("{} {} isn't a number of seconds", name, text)))
	}
}

impl World {
	/// Has the pause this command holds last at most `seconds` with nobody
	/// debugging
	pub fn hold_max(&mut self, seconds: f64) -> Result<()> {
		match self.request(Request::HoldMax { seconds: Some(seconds) })? {
			Response::Ack => Ok(()),
			other => Err(unexpected(&other))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::take;

	fn args(args: &[&str]) -> Vec<String> {
		args.iter().map(|arg| (*arg).to_owned()).collect()
	}

	#[test]
	fn hold_max_is_taken_out_for_commands_that_hold() {
		let (rest, seconds) = take("wait", &args(&["--break", "/proc/x", "--hold-max", "2.5", "--timeout", "3"])).ok().unwrap();
		assert_eq!((rest, seconds), (args(&["--break", "/proc/x", "--timeout", "3"]), Some(2.5)));
		assert!(take("pause", &args(&["--hold-max", "soon"])).is_err());
		assert!(take("pause", &args(&["--hold-max"])).is_err());
		// Others don't take it
		let (rest, seconds) = take("status", &args(&["--hold-max", "2"])).ok().unwrap();
		assert_eq!((rest, seconds), (args(&["--hold-max", "2"]), None));
	}
}
