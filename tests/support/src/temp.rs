use std::{
	ops::Deref,
	path::{Path, PathBuf}
};

#[derive(Debug)]
pub struct TempDir(PathBuf);

impl TempDir {
	pub fn new(name: &str) -> Self {
		let dir = std::env::temp_dir().join(format!("{}-{}", name, std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		Self(dir)
	}

	pub fn copy_of(name: &str, from: &Path, extensions: &[&str]) -> Self {
		fn copy(from: &Path, to: &Path, extensions: &[&str]) {
			std::fs::create_dir_all(to).unwrap();
			for entry in std::fs::read_dir(from).unwrap() {
				let path = entry.unwrap().path();
				let copy_to = to.join(path.file_name().unwrap());
				if path.is_dir() {
					copy(&path, &copy_to, extensions);
				} else if path.extension().is_some_and(|e| extensions.contains(&e.to_str().unwrap_or_default())) {
					std::fs::copy(&path, copy_to).unwrap();
				}
			}
		}
		let dir = Self::new(name);
		copy(from, &dir, extensions);
		dir
	}
}

impl Deref for TempDir {
	type Target = Path;

	fn deref(&self) -> &Path {
		&self.0
	}
}

impl AsRef<Path> for TempDir {
	fn as_ref(&self) -> &Path {
		&self.0
	}
}

impl Drop for TempDir {
	fn drop(&mut self) {
		if !std::thread::panicking() {
			let _ = std::fs::remove_dir_all(&self.0);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn it_is_named_for_the_process_and_removed_when_dropped() {
		let dir = TempDir::new("test_support_dropped");
		let path = dir.to_path_buf();
		assert_eq!(path.parent(), Some(std::env::temp_dir().as_path()));
		assert_eq!(
			path.file_name().unwrap().to_str(),
			Some(format!("test_support_dropped-{}", std::process::id()).as_str())
		);
		std::fs::create_dir(dir.join("inside")).unwrap();
		std::fs::write(dir.join("inside/file"), "text").unwrap();
		drop(dir);
		assert!(!path.exists(), "{} is still there", path.display());
	}

	#[test]
	fn it_is_left_when_a_check_fails() {
		let made = std::sync::Mutex::new(None);
		let failed = std::panic::catch_unwind(|| {
			let dir = TempDir::new("test_support_failed");
			std::fs::write(dir.join("file"), "text").unwrap();
			*made.lock().unwrap() = Some(dir.to_path_buf());
			panic!("a check failed (this test's own panic)");
		});
		assert!(failed.is_err());
		let path = made.lock().unwrap().take().unwrap();
		assert!(path.join("file").exists(), "{} is gone", path.display());
		std::fs::remove_dir_all(path).unwrap();
	}

	#[test]
	fn what_another_run_left_is_removed_first() {
		let left = std::env::temp_dir().join(format!("test_support_left-{}", std::process::id()));
		std::fs::create_dir_all(&left).unwrap();
		std::fs::write(left.join("stop"), "yes").unwrap();
		let dir = TempDir::new("test_support_left");
		assert_eq!(&*dir, left.as_path());
		assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "the directory isn't empty");
	}

	#[test]
	fn a_copy_has_the_files_asked_for_and_the_directories() {
		let from = TempDir::new("test_support_copied_from");
		std::fs::create_dir(from.join("sub")).unwrap();
		for file in ["world.dme", "world.dmb", "notes.txt", "sub/code.dm", "sub/left.log"] {
			std::fs::write(from.join(file), file).unwrap();
		}
		let copy = TempDir::copy_of("test_support_copied", &from, &["dme", "dm"]);
		let has = |file: &str| copy.join(file).is_file();
		assert!(has("world.dme") && has("sub/code.dm"), "the copy lacks a file");
		assert!(
			!has("world.dmb") && !has("notes.txt") && !has("sub/left.log"),
			"the copy has a file not asked for"
		);
		assert_eq!(std::fs::read_to_string(copy.join("sub/code.dm")).unwrap(), "sub/code.dm");
	}
}
