use std::path::{Path, PathBuf};

use dmasm::{
	reload::{ParseOptions, ProcKey, Snapshot},
	ByondVersion
};
use serde_json::json;

use super::{args, Failure, Result};
use crate::code::parse_define;

pub const FOUND: i32 = 5;

fn proc_name(proc: &ProcKey) -> String {
	match proc.override_id {
		0 => proc.path.clone(),
		n => format!("{}#{}", proc.path, n)
	}
}

fn only_dme() -> Result<PathBuf> {
	let mut found: Vec<PathBuf> = std::fs::read_dir(".")
		.map_err(|e| format!("cannot read the current directory: {}.", e))?
		.filter_map(|entry| Some(entry.ok()?.path()))
		.filter(|path| path.extension().is_some_and(|extension| extension == "dme"))
		.collect();
	match found.len() {
		1 => Ok(found.remove(0)),
		0 => Err("no .dme in the current directory: specify DME.".into()),
		_ => Err("more than one .dme in the current directory: specify DME.".into())
	}
}

fn counted(count: usize, one: &str, many: &str) -> String {
	match count {
		1 => format!("1 {}", one),
		n => format!("{} {}", n, many)
	}
}

pub fn check(args: &args::Check) -> Result<()> {
	let dme = match &args.dme {
		Some(dme) => dme.clone(),
		None => only_dme()?
	};
	let dme = std::path::absolute(&dme).map_err(|e| format!("invalid path {}: {}.", dme.display(), e))?;
	if !dme.is_file() {
		return Err(format!("{} does not exist.", dme.display()).into());
	}
	let dir = dme.parent().unwrap_or(Path::new("."));
	std::env::set_current_dir(dir).map_err(|e| format!("cannot enter {}: {}.", dir.display(), e))?;
	let name = PathBuf::from(dme.file_name().unwrap_or_default());

	let latest = ByondVersion::LATEST;
	let (major, build) = args.byond_version.unwrap_or((latest.major, latest.build));
	let options = ParseOptions {
		defines: args.defines.iter().map(|define| parse_define(define)).collect(),
		byond: Some((major, build))
	};
	std::panic::set_hook(Box::new(|_| {}));
	let mut snapshot = Snapshot::parse(&name, &options).map_err(|e| format!("{}.", e))?;
	snapshot.set_byond(ByondVersion::new(major, build));

	let unread: Vec<String> = snapshot.parse_errors().into_iter().map(|(_, error)| error).collect();
	let found = snapshot.known_bugs();
	let procs = snapshot.procs().count() - found.not_checked.len();

	if args.json {
		say!(
			"{}",
			json!({
				"byond": format!("{}.{}", major, build),
				"procs": procs,
				"found": found.bugs.iter().map(|bug| json!({
					"file": bug.file,
					"line": bug.line,
					"proc": bug.proc.path,
					"override": bug.proc.override_id,
					"text": bug.text
				})).collect::<Vec<_>>(),
				"not_checked": found.not_checked.iter().map(|(proc, file, error)| json!({
					"file": file,
					"proc": proc.path,
					"override": proc.override_id,
					"error": error
				})).collect::<Vec<_>>(),
				"parse_errors": unread
			})
		);
	} else {
		for error in &unread {
			eprintln!("not checked: {}", error);
		}
		for (proc, file, error) in &found.not_checked {
			eprintln!("not checked: {}: {}: {}", file, proc_name(proc), error);
		}
		for bug in &found.bugs {
			match bug.line {
				Some(line) => say!("{}:{}: {}: {}", bug.file, line, proc_name(&bug.proc), bug.text),
				None => say!("{}: {}: {}", bug.file, proc_name(&bug.proc), bug.text)
			}
		}
	}

	let mut summary = format!(
		"{} found in {} for BYOND {}.{}",
		match found.bugs.len() {
			0 => "nothing".to_owned(),
			n => counted(n, "issue", "issues")
		},
		counted(procs, "proc", "procs"),
		major,
		build
	);
	if !found.not_checked.is_empty() {
		summary += &format!("; {} not checked", counted(found.not_checked.len(), "proc", "procs"));
	}
	if !unread.is_empty() {
		summary += &format!("; {}", counted(unread.len(), "parse error", "parse errors"));
	}
	if !found.bugs.is_empty() {
		return Err(Failure(FOUND, format!("{}.", summary)));
	}
	if !args.json {
		say!("{}", summary);
	}
	Ok(())
}
