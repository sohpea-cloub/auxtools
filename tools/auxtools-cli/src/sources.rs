//! Where the world's procs are in its source, from a parse of its .dme. The
//! debug server knows procs by path and override id (the order of their
//! definitions), and the client knows files and lines.

use std::{
	collections::HashMap,
	path::{Path, PathBuf}
};

use dreammaker::{objtree::ObjectTree, Context, FileId};

/// stddef.dm's procs. SpacemanDMM counts them as built in, but BYOND compiles
/// them like any other, so each is a proc's first definition.
const STDDEF_PROCS: &[&str] = &[
	"/sound/New",
	"/sound/RscFile",
	"/icon/New",
	"/icon/Icon",
	"/icon/RscFile",
	"/icon/IconStates",
	"/icon/Turn",
	"/icon/Flip",
	"/icon/Shift",
	"/icon/SetIntensity",
	"/icon/Blend",
	"/icon/SwapColor",
	"/icon/DrawBox",
	"/icon/Insert",
	"/icon/MapColors",
	"/icon/Scale",
	"/icon/Crop",
	"/icon/GetPixel",
	"/icon/Width",
	"/icon/Height",
	"/matrix/New",
	"/matrix/Multiply",
	"/matrix/Add",
	"/matrix/Subtract",
	"/matrix/Invert",
	"/matrix/Turn",
	"/matrix/Scale",
	"/matrix/Translate",
	"/matrix/Interpolate",
	"/database/New",
	"/database/Open",
	"/database/Close",
	"/database/Error",
	"/database/ErrorMsg",
	"/database/query/New",
	"/database/query/Open",
	"/database/query/Clear",
	"/database/query/Add",
	"/database/query/Execute",
	"/database/query/NextRow",
	"/database/query/RowsAffected",
	"/database/query/Reset",
	"/database/query/Columns",
	"/database/query/GetColumn",
	"/database/query/GetRowData",
	"/database/query/Close",
	"/exception/New",
	"/regex/New",
	"/regex/Find",
	"/regex/Replace"
];

/// Where a proc's definition starts
#[derive(Clone, Debug)]
pub enum Place {
	File {
		path: PathBuf,
		line: u32
	},
	/// In stddef.dm, which the debug server has the text of
	Stddef
}

/// A proc as the debug server knows it
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct ProcId {
	/// `/type/name`, `/name` for a global proc
	pub path: String,
	pub override_id: u32
}

pub struct Sources {
	/// The .dme's directory, which the files are named relative to
	root: PathBuf,
	/// Each file's procs by the line they start on, sorted, by `file_key`
	by_file: HashMap<String, Vec<(u32, ProcId)>>,
	procs: HashMap<ProcId, Place>
}

impl Sources {
	/// The procs in a parse of the codebase whose .dme is in `root`. Errors in
	/// the code only leave procs out.
	pub fn from_tree(ctx: &Context, tree: &ObjectTree, root: PathBuf) -> Self {
		let mut sources = Self {
			root,
			by_file: HashMap::new(),
			procs: HashMap::new()
		};

		let mut paths: HashMap<FileId, PathBuf> = HashMap::new();
		for ty in tree.iter_types() {
			for (name, type_proc) in ty.get().procs.iter() {
				let path = format!("{}/{}", ty.get().path, name);
				let mut override_id = 0;
				for value in &type_proc.value {
					let location = value.location;
					let place = if location.is_builtins() {
						if !STDDEF_PROCS.contains(&path.as_str()) {
							continue;
						}
						Place::Stddef
					} else {
						let file = paths
							.entry(location.file)
							.or_insert_with(|| sources.root.join(&*ctx.file_path(location.file)))
							.clone();
						Place::File {
							path: file,
							line: location.line
						}
					};

					let id = ProcId {
						path: path.clone(),
						override_id
					};
					override_id += 1;
					if let Place::File { path, line } = &place {
						sources.by_file.entry(file_key(path)).or_default().push((*line, id.clone()));
					}
					sources.procs.insert(id, place);
				}
			}
		}

		for procs in sources.by_file.values_mut() {
			procs.sort_by_key(|(line, _)| *line);
		}
		sources
	}

	/// Whether `path` is one of the codebase's files
	pub fn has_file(&self, path: &str) -> bool {
		self.by_file.contains_key(&file_key(Path::new(path)))
	}

	/// The proc a line of a file is in: the last one to start at or before it
	pub fn proc_at(&self, path: &str, line: u32) -> Option<&ProcId> {
		let procs = self.by_file.get(&file_key(Path::new(path)))?;
		procs.iter().rev().find(|(start, _)| *start <= line).map(|(_, id)| id)
	}

	/// Where a proc is, by the path the debug server gives it (which can have
	/// `proc/` or `verb/` in it)
	pub fn place(&self, path: &str, override_id: u32) -> Option<&Place> {
		self.procs.get(&ProcId {
			path: strip_declaration(path),
			override_id
		})
	}
}

/// A file's path in a form two spellings of it share: absolute, with `/`
/// between its parts, and on Windows in lower case
fn file_key(path: &Path) -> String {
	let path = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
	let key = path.to_string_lossy().replace('\\', "/");
	if cfg!(windows) {
		key.to_lowercase()
	} else {
		key
	}
}

/// `/mob/proc/Life` as `/mob/Life`
pub fn strip_declaration(path: &str) -> String {
	let mut parts: Vec<&str> = path.split('/').collect();
	if parts.len() >= 3 && matches!(parts[parts.len() - 2], "proc" | "verb") {
		parts.remove(parts.len() - 2);
	}
	parts.join("/")
}

/// stddef.dm's procs, from its text, as the debug server hands it over
pub struct Stddef {
	pub text: String,
	lines: HashMap<ProcId, u32>
}

impl Stddef {
	pub fn new(text: String) -> Self {
		let ctx = Context::default();
		let pp = dreammaker::Preprocessor::from_buffer(&ctx, "stddef.dm".into(), text.as_str());
		let tree = dreammaker::Parser::new(&ctx, pp).parse_object_tree_without_builtins();

		let mut lines = HashMap::new();
		for ty in tree.iter_types() {
			for (name, type_proc) in ty.get().procs.iter() {
				if let Some(value) = type_proc.value.first() {
					lines.insert(
						ProcId {
							path: format!("{}/{}", ty.get().path, name),
							override_id: 0
						},
						value.location.line
					);
				}
			}
		}
		Self { text, lines }
	}

	pub fn line(&self, path: &str, override_id: u32) -> Option<u32> {
		self.lines
			.get(&ProcId {
				path: strip_declaration(path),
				override_id
			})
			.copied()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn lines_find_procs() {
		let dir = std::env::temp_dir().join(format!("launcher-sources-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("w.dme"), "#include \"code.dm\"\n").unwrap();
		std::fs::write(
			dir.join("code.dm"),
			"/proc/first()\n\treturn 1\n\n/mob/proc/second()\n\treturn 2\n\n/mob/second()\n\t..()\n\n/icon/Width()\n\treturn 3\n"
		)
		.unwrap();

		let snapshot = dmasm::reload::Snapshot::parse(&dir.join("w.dme"), &Default::default()).unwrap();
		let sources = Sources::from_tree(snapshot.context(), snapshot.tree(), dir.clone());
		let code = dir.join("code.dm");
		let code = code.to_str().unwrap();
		assert!(sources.has_file(code));

		let at = |line| sources.proc_at(code, line).map(|id| (id.path.clone(), id.override_id));
		assert_eq!(at(2), Some(("/first".to_owned(), 0)));
		assert_eq!(at(5), Some(("/mob/second".to_owned(), 0)));
		assert_eq!(at(8), Some(("/mob/second".to_owned(), 1)));
		// stddef.dm's is the first
		assert_eq!(at(11), Some(("/icon/Width".to_owned(), 1)));

		assert!(matches!(sources.place("/mob/proc/second", 1), Some(Place::File { line: 7, .. })));
		assert!(matches!(sources.place("/proc/first", 0), Some(Place::File { line: 1, .. })));
		assert!(matches!(sources.place("/icon/proc/Width", 0), Some(Place::Stddef)));

		std::fs::remove_dir_all(&dir).unwrap();
	}

	#[test]
	fn declarations_are_stripped() {
		assert_eq!(strip_declaration("/mob/proc/Life"), "/mob/Life");
		assert_eq!(strip_declaration("/proc/global"), "/global");
		assert_eq!(strip_declaration("/mob/verb/say"), "/mob/say");
		assert_eq!(strip_declaration("/mob/Login"), "/mob/Login");
	}
}
