use std::{path::PathBuf, process::Command};

use test_support::paths::{self, ByondCommand};

fn find_dll() -> PathBuf {
	let mut path = std::env::current_exe().unwrap();
	path.pop();

	#[cfg(unix)]
	path.push("libauxtest.so");

	#[cfg(windows)]
	path.push("auxtest.dll");

	assert!(path.is_file(), "couldn't find auxtest");
	path
}

fn find_dme() -> PathBuf {
	let mut path = std::env::current_dir().unwrap();
	path.push("tests/auxtest_host/auxtest_host.dme");
	assert!(path.is_file(), "couldn't find auxtest_host.dme");
	path
}

fn find_dmb() -> PathBuf {
	let mut path = std::env::current_dir().unwrap();
	path.push("tests/auxtest_host/auxtest_host.dmb");
	assert!(path.is_file(), "couldn't find auxtest_host.dmb");
	path
}

fn main() {
	paths::compile(&find_dme());

	let output = Command::new(paths::find_dreamdaemon())
		.with_byond_paths()
		.env("AUXTEST_DLL", find_dll())
		.arg(find_dmb())
		.arg("-trusted")
		.arg("-close")
		.output()
		.unwrap();

	let res = std::str::from_utf8(&output.stderr).unwrap();

	let errors = res.lines().filter(|x| x.starts_with("FAILED: ")).collect::<Vec<&str>>();

	if !errors.is_empty() {
		panic!("TESTS FAILED\n{}", errors.join("\n"));
	}

	let successes = res.lines().filter(|x| x.starts_with("SUCCESS: ")).collect::<Vec<&str>>();
	assert_eq!(successes.len(), 1, "Tests didn't output success message");
	assert!(
		output.status.success(),
		"DreamDaemon exited with {} after the tests passed\n{}",
		output.status,
		res
	);

	println!("Tests Succeeded");
}
