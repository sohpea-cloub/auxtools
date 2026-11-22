use std::{
	path::{Path, PathBuf},
	process::Command
};

pub trait ByondCommand {
	fn with_byond_paths(&mut self) -> &mut Self;
}

#[cfg(unix)]
impl ByondCommand for Command {
	fn with_byond_paths(&mut self) -> &mut Command {
		let byond_system = find_byond();
		let byond_bin = find_byond_bin();

		let path = format!(
			"{}:{}",
			byond_bin.as_os_str().to_str().unwrap(),
			std::env::var_os("PATH").unwrap().to_str().unwrap()
		);

		let ld_library_path = format!(
			"{}:{}",
			byond_bin.as_os_str().to_str().unwrap(),
			std::env::var_os("LD_LIBRARY_PATH").unwrap().to_str().unwrap()
		);

		self.env("BYOND_SYSTEM", byond_system)
			.env("PATH", path)
			.env("LD_LIBRARY_PATH", ld_library_path)
	}
}

#[cfg(windows)]
impl ByondCommand for Command {
	fn with_byond_paths(&mut self) -> &mut Command {
		self
	}
}

pub fn find_byond() -> PathBuf {
	let path = PathBuf::from(std::env::var_os("BYOND_PATH").unwrap());
	assert!(path.is_dir(), "couldn't find byond");
	path
}

pub fn find_byond_bin() -> PathBuf {
	let mut path = find_byond();
	path.push("bin");
	assert!(path.is_dir(), "couldn't find byond/bin");
	path
}

pub fn find_dm() -> PathBuf {
	let mut path = find_byond();

	#[cfg(unix)]
	path.push("bin/DreamMaker");

	#[cfg(windows)]
	path.push("bin/dm.exe");

	assert!(path.is_file(), "couldn't find dreammaker");
	path
}

pub fn find_dreamdaemon() -> PathBuf {
	let mut path = find_byond();

	#[cfg(unix)]
	path.push("bin/DreamDaemon");

	#[cfg(windows)]
	path.push("bin/dd.exe");

	assert!(path.is_file(), "couldn't find dreamdaemon");
	path
}

#[track_caller]
pub fn next_to_us(name: &str) -> PathBuf {
	let mut path = std::env::current_exe().unwrap();
	path.pop();
	path.push(name);
	assert!(path.is_file(), "couldn't find {}; build it first", path.display());
	path
}

#[track_caller]
pub fn launcher_path() -> PathBuf {
	next_to_us(if cfg!(windows) { "auxtools.exe" } else { "auxtools" })
}

#[track_caller]
pub fn library_path() -> PathBuf {
	next_to_us(if cfg!(windows) { "debug_server.dll" } else { "libdebug_server.so" })
}

pub fn dreammaker() -> Command {
	let mut dm = Command::new(find_dm());
	dm.with_byond_paths();
	dm
}

#[track_caller]
pub fn compile(dme: &Path) {
	compile_with(dme, &[]);
}

#[track_caller]
pub fn compile_with(dme: &Path, flags: &[&str]) {
	let built = dreammaker().args(flags).arg(dme).status().unwrap();
	assert!(built.success(), "dreammaker build of {} failed", dme.display());
}
