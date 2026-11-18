//! Rewrite a compiled world to use less memory.

use std::path::{Path, PathBuf};

use dmasm::dmb::Dmb;
use serde_json::{json, Map, Value};

use super::{args, Result};
use crate::optimise::{names, PROC_ENTRY_BYTES};

fn megabytes(bytes: u64) -> String {
	format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

fn counted(count: usize, one: &str, many: &str) -> String {
	match count {
		1 => format!("1 {}", one),
		n => format!("{} {}", n, many)
	}
}

/// Write beside the target and rename, so a failed write leaves the world
/// as it was.
fn write(path: &Path, data: &[u8]) -> std::result::Result<(), String> {
	let mut name = path.file_name().unwrap_or_default().to_owned();
	name.push(format!(".{}.tmp", std::process::id()));
	let temporary = path.with_file_name(name);
	std::fs::write(&temporary, data)
		.and_then(|()| std::fs::rename(&temporary, path))
		.map_err(|e| {
			let _ = std::fs::remove_file(&temporary);
			format!("cannot write {}: {}.", path.display(), e)
		})
}

fn names_text(report: &names::Report) {
	say!(
		"names: renamed {} of {}, changed {}",
		counted(report.procs, "proc", "procs"),
		counted(report.identifiers, "identifier", "identifiers"),
		counted(report.calls, "call", "calls")
	);
	say!(
		"  proc tables: {} to {} ({} second names to {})",
		megabytes(report.before.bytes()),
		megabytes(report.after.bytes()),
		report.before.second_names,
		report.after.second_names
	);
	for (kept, identifiers) in &report.kept {
		say!(
			"  kept: {} with {}",
			counted(identifiers.len(), "identifier", "identifiers"),
			kept.reason()
		);
	}
}

fn names_json(report: &names::Report) -> Value {
	let kept: Map<String, Value> = names::Kept::ALL
		.iter()
		.map(|kept| (kept.key().to_owned(), json!(report.kept.get(kept).map_or(&[][..], Vec::as_slice))))
		.collect();
	json!({
		"identifiers": report.identifiers,
		"procs": report.procs,
		"calls": report.calls,
		"kept": kept,
		"proc_table_bytes": { "before": report.before.bytes(), "after": report.after.bytes() },
		"second_names": {
			"before": report.before.second_names,
			"after": report.after.second_names,
			"bytes_each": PROC_ENTRY_BYTES
		}
	})
}

pub fn optimise(args: &args::Optimise) -> Result<()> {
	let data = std::fs::read(&args.dmb).map_err(|e| format!("cannot read {}: {}.", args.dmb.display(), e))?;
	let mut dmb = Dmb::read(&data).map_err(|e| format!("cannot read {}: {}.", args.dmb.display(), e))?;

	let names = names::run(&mut dmb);

	let output: &PathBuf = args.output.as_ref().unwrap_or(&args.dmb);
	let written = dmb.write().map_err(|e| format!("cannot write {}: {}.", output.display(), e))?;
	if !args.dry_run {
		write(output, &written)?;
	}

	if args.json {
		say!(
			"{}",
			json!({
				"dmb": args.dmb,
				"output": if args.dry_run { Value::Null } else { json!(output) },
				"bytes": { "before": data.len(), "after": written.len() },
				"names": names_json(&names)
			})
		);
	} else {
		names_text(&names);
		if args.dry_run {
			say!(
				"{} not written: {} to {}",
				output.display(),
				megabytes(data.len() as u64),
				megabytes(written.len() as u64)
			);
		} else {
			say!(
				"wrote {}: {} to {}",
				output.display(),
				megabytes(data.len() as u64),
				megabytes(written.len() as u64)
			);
		}
	}
	Ok(())
}
