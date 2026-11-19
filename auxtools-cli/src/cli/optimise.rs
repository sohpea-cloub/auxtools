//! Rewrite a compiled world to use less memory.

use std::path::{Path, PathBuf};

use dmasm::dmb::Dmb;
use serde_json::{json, Map, Value};

use super::{args, Result};
use crate::optimise::{globals, names, refs, report, PROC_ENTRY_BYTES};

fn megabytes(bytes: u64) -> String {
	format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

/// A size in the report's lists, which small worlds fill too.
fn size(bytes: u64) -> String {
	if bytes < 1_048_576 {
		format!("{:.1} KB", bytes as f64 / 1024.0)
	} else {
		megabytes(bytes)
	}
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

fn costs_text(costs: &[report::Cost], what: &str) {
	for cost in costs {
		say!(
			"  {:>9}  {} {} in {} of {}",
			size(cost.bytes),
			cost.entries,
			what,
			counted(cost.types as usize, "table", "tables"),
			cost.path
		);
	}
}

fn report_text(report: &report::Report) {
	say!("world: {} types, {} procs, {} strings", report.types, report.procs, report.strings);
	say!(
		"proc tables: {} ({} entries under names, {} under second names). The types whose procs cost most:",
		megabytes(report.proc_tables.bytes()),
		report.proc_tables.names,
		report.proc_tables.second_names
	);
	costs_text(&report.proc_costs, "names");
	let unused: u64 = report.unused.iter().map(|(_, bytes)| bytes).sum();
	say!(
		"  {} on types that no call, text or value names: {}{}",
		counted(report.unused.len(), "proc", "procs"),
		size(unused),
		if report.unused.is_empty() { "" } else { ". The costliest:" }
	);
	for (path, bytes) in report.unused.iter().take(10) {
		say!("  {:>9}  {}", size(*bytes), path);
	}
	say!(
		"var tables: {} ({} entries in {}). The types whose vars cost most:",
		megabytes(report.var_entries * report::VAR_ENTRY_BYTES),
		report.var_entries,
		counted(report.var_tables as usize, "table", "tables")
	);
	costs_text(&report.var_costs, "vars");
}

fn report_json(report: &report::Report) -> Value {
	let costs = |costs: &[report::Cost]| -> Vec<Value> {
		costs
			.iter()
			.map(|cost| json!({ "type": cost.path, "entries": cost.entries, "tables": cost.types, "bytes": cost.bytes }))
			.collect()
	};
	json!({
		"types": report.types,
		"procs": report.procs,
		"strings": report.strings,
		"proc_tables": {
			"bytes": report.proc_tables.bytes(),
			"names": report.proc_tables.names,
			"second_names": report.proc_tables.second_names,
			"costliest": costs(&report.proc_costs)
		},
		"var_tables": {
			"bytes": report.var_entries * report::VAR_ENTRY_BYTES,
			"entries": report.var_entries,
			"tables": report.var_tables,
			"costliest": costs(&report.var_costs)
		},
		"unused_procs": report.unused.iter().map(|(path, bytes)| json!({ "proc": path, "bytes": bytes })).collect::<Vec<_>>()
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

fn globals_text(report: &globals::Report) {
	say!(
		"globals: made {} global, changed {}",
		counted(report.procs.len(), "proc", "procs"),
		counted(report.calls, "call", "calls")
	);
	say!(
		"  proc tables: {} to {}",
		megabytes(report.before.bytes()),
		megabytes(report.after.bytes())
	);
	if let Some(why) = report.refused {
		say!("  stopped: {}", why);
	}
}

fn globals_json(report: &globals::Report) -> Value {
	json!({
		"procs": report.procs,
		"calls": report.calls,
		"proc_table_bytes": { "before": report.before.bytes(), "after": report.after.bytes() },
		"refused": report.refused
	})
}

fn refs_text(report: &refs::Report) {
	say!(
		"refs: added {} spaced {} refs apart{}",
		counted(report.added, "string", "strings"),
		refs::SPACING,
		match report.present {
			0 => String::new(),
			present => format!(", {} already there", present)
		}
	);
	if report.full {
		say!("  stopped: no string id left");
	}
}

fn refs_json(report: &refs::Report) -> Value {
	json!({
		"added": report.added,
		"present": report.present,
		"wanted": report.wanted,
		"spacing": refs::SPACING,
		"full": report.full
	})
}

pub fn optimise(args: &args::Optimise) -> Result<()> {
	let data = std::fs::read(&args.dmb).map_err(|e| format!("cannot read {}: {}.", args.dmb.display(), e))?;
	let mut dmb = Dmb::read(&data).map_err(|e| format!("cannot read {}: {}.", args.dmb.display(), e))?;

	// What the world costs as it was built
	let report = args.report.then(|| report::run(&dmb));
	let dry_run = args.dry_run || args.report;
	let skipped = |pass: &str| args.skip.iter().any(|skip| skip == pass);
	let names = (!skipped("names")).then(|| names::run(&mut dmb));
	let globals = args.globals.then(|| globals::run(&mut dmb));
	let refs = (!skipped("refs")).then(|| refs::run(&mut dmb, args.ref_seeds.unwrap_or(refs::DEFAULT)));

	let output: &PathBuf = args.output.as_ref().unwrap_or(&args.dmb);
	let written = dmb.write().map_err(|e| format!("cannot write {}: {}.", output.display(), e))?;
	if !dry_run {
		write(output, &written)?;
	}

	if args.json {
		say!(
			"{}",
			json!({
				"dmb": args.dmb,
				"output": if dry_run { Value::Null } else { json!(output) },
				"bytes": { "before": data.len(), "after": written.len() },
				"report": report.as_ref().map(report_json),
				"names": names.as_ref().map(names_json),
				"globals": globals.as_ref().map(globals_json),
				"refs": refs.as_ref().map(refs_json)
			})
		);
	} else {
		if let Some(report) = &report {
			report_text(report);
		}
		if let Some(names) = &names {
			names_text(names);
		}
		if let Some(globals) = &globals {
			globals_text(globals);
		}
		if let Some(refs) = &refs {
			refs_text(refs);
		}
		if dry_run {
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
