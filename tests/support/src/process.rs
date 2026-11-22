use std::process::Command;

pub fn kill(pid: u64) -> bool {
	let killed = if cfg!(windows) {
		Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output()
	} else {
		Command::new("kill").args(["-9", &pid.to_string()]).output()
	};
	killed.is_ok_and(|out| out.status.success())
}

#[track_caller]
pub fn kill_process(pid: u64) {
	assert!(kill(pid), "couldn't kill process {}", pid);
}

#[cfg(test)]
mod tests {
	use std::process::Stdio;

	use super::*;

	#[test]
	fn a_running_process_is_ended_and_one_that_has_gone_is_not() {
		let mut waiting = Command::new(if cfg!(windows) { "cmd" } else { "sh" })
			.stdin(Stdio::piped())
			.stdout(Stdio::null())
			.spawn()
			.unwrap();
		let pid = u64::from(waiting.id());
		assert!(waiting.try_wait().unwrap().is_none(), "the shell didn't wait");
		assert!(kill(pid), "couldn't kill process {}", pid);
		assert!(!waiting.wait().unwrap().success(), "the shell ended by itself");
		assert!(!kill(pid), "process {} was ended twice", pid);
	}
}
