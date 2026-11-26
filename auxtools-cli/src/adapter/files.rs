use std::{
	collections::HashMap,
	path::{Path, PathBuf},
	time::{Duration, Instant}
};

use dmasm::debug_files;

const STALE: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct WorldFiles {
	dme: Option<PathBuf>,
	names: Option<(HashMap<String, PathBuf>, HashMap<PathBuf, String>, Instant)>
}

impl WorldFiles {
	pub const fn new(dme: Option<PathBuf>) -> Self {
		WorldFiles { dme, names: None }
	}

	pub fn dme(&self) -> Option<&Path> {
		self.dme.as_deref()
	}

	fn root(&self) -> Option<&Path> {
		self.dme.as_deref()?.parent()
	}

	fn read(&mut self) {
		let Some(dme) = &self.dme else { return };
		let mut by_name = HashMap::new();
		let mut by_path = HashMap::new();
		for (name, path) in debug_files::names(dme, &|_| true) {
			if Path::new(&name) != path {
				by_name.entry(name.clone()).or_insert_with(|| path.clone());
				by_path.entry(path).or_insert(name);
			}
		}
		self.names = Some((by_name, by_path, Instant::now()));
	}

	pub fn path(&mut self, name: &str) -> PathBuf {
		let name = name.replace('\\', "/");
		let Some(root) = self.root().map(Path::to_owned) else {
			return PathBuf::from(name);
		};
		if is_absolute(&name) {
			let built = Path::new(&name);
			return match &self.dme {
				Some(dme) if !built.is_file() && name.rsplit('/').next() == dme.file_name().and_then(|file| file.to_str()) => dme.clone(),
				_ => built.to_owned()
			};
		}
		if self.names.is_none() {
			self.read();
		}
		if let Some(path) = self.names.as_ref().and_then(|(by_name, ..)| by_name.get(&name)) {
			return root.join(path);
		}
		let path = root.join(debug_files::normalize(Path::new(&name)));
		if !path.is_file() && self.names.as_ref().is_some_and(|(.., read)| read.elapsed() >= STALE) {
			self.read();
			if let Some(found) = self.names.as_ref().and_then(|(by_name, ..)| by_name.get(&name)) {
				return root.join(found);
			}
		}
		path
	}

	pub fn name(&mut self, path: &str) -> String {
		let Some(relative) = self.root().and_then(|root| Path::new(path).strip_prefix(root).ok()).map(Path::to_owned) else {
			return path.to_owned();
		};
		if self.names.is_none() {
			self.read();
		}
		match self
			.names
			.as_ref()
			.and_then(|(_, by_path, _)| by_path.get(&debug_files::normalize(&relative)))
		{
			Some(name) => name.clone(),
			None => relative.to_string_lossy().replace('\\', "/")
		}
	}
}

fn is_absolute(name: &str) -> bool {
	let bytes = name.as_bytes();
	name.starts_with('/') || (bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/')
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn names_and_paths() {
		let dir = std::env::temp_dir().join(format!("auxtools-world-files-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		let write = |path: &str, text: &str| {
			let path = dir.join(path);
			std::fs::create_dir_all(path.parent().unwrap()).unwrap();
			std::fs::write(path, text).unwrap();
		};
		write("world.dme", "#include \"code\\map.dm\"\n#include \"_std\\__std.dme\"\n");
		write("code/map.dm", "");
		write("_std/__std.dme", "#include \"map.dm\"\n#include \"macros\\dir.dm\"\n");
		write("_std/map.dm", "");
		write("_std/macros/dir.dm", "");
		let mut files = WorldFiles::new(Some(dir.join("world.dme")));

		assert_eq!(files.path("code/map.dm"), dir.join("code/map.dm"));
		assert_eq!(files.path("map.dm"), dir.join("_std/map.dm"));
		assert_eq!(files.path("macros/dir.dm"), dir.join("_std/macros/dir.dm"));
		assert_eq!(files.path("_std/__std.dme"), dir.join("_std/__std.dme"));
		assert_eq!(files.path("code\\gone.dm"), dir.join("code/gone.dm"));
		assert_eq!(files.path("code/sub/../map.dm"), dir.join("code/map.dm"));
		assert_eq!(files.path("/build/somewhere/world.dme"), dir.join("world.dme"));
		assert_eq!(files.path("C:\\build\\world.dme"), dir.join("world.dme"));
		assert_eq!(files.path("/build/somewhere/other.dme"), Path::new("/build/somewhere/other.dme"));

		let name = |files: &mut WorldFiles, path: &str| files.name(&dir.join(path).to_string_lossy());
		assert_eq!(name(&mut files, "code/map.dm"), "code/map.dm");
		assert_eq!(name(&mut files, "_std/map.dm"), "map.dm");
		assert_eq!(name(&mut files, "_std/macros/dir.dm"), "macros/dir.dm");
		assert_eq!(name(&mut files, "_std/__std.dme"), "_std/__std.dme");
		assert_eq!(files.name("/elsewhere/code/map.dm"), "/elsewhere/code/map.dm");

		let mut none = WorldFiles::default();
		assert_eq!(none.path("code\\map.dm"), Path::new("code/map.dm"));
		assert_eq!(none.name("/elsewhere/code/map.dm"), "/elsewhere/code/map.dm");
		let _ = std::fs::remove_dir_all(&dir);
	}
}
