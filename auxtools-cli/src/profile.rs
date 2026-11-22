use std::{collections::HashMap, path::PathBuf};

use auxtools_cli::local;
use auxtools_protocol::*;
use serde_json::json;

pub struct Report {
	pub seconds: f64,
	pub ended: &'static str,
	pub overhead_ns: f64,
	pub hits: u64,
	pub own_ns: u64,
	pub procs: Vec<Listed>
}

pub struct Listed {
	pub code: ProfiledCode,
	pub listing: Option<Vec<Row>>
}

pub struct Row {
	pub offset: u16,
	pub text: String,
	pub line: Option<u32>,
	pub source: Option<String>,
	pub hits: u64,
	pub own_ns: u64,
	pub callee_ns: u64
}

pub struct Line {
	pub place: String,
	pub hits: u64,
	pub own_ns: u64,
	pub callee_ns: u64
}

impl Report {
	pub fn new(totals: ProfileTotals, ended: &'static str) -> Self {
		let mut procs: Vec<Listed> = totals.codes.into_iter().map(|code| Listed { code, listing: None }).collect();
		procs.sort_by(|a, b| {
			(b.code.total.own_ns.cmp(&a.code.total.own_ns))
				.then_with(|| a.code.proc.path.cmp(&b.code.proc.path))
				.then_with(|| a.code.proc.override_id.cmp(&b.code.proc.override_id))
		});
		Self {
			seconds: totals.seconds,
			ended,
			overhead_ns: totals.overhead_ns,
			hits: procs.iter().map(|listed| listed.code.total.hits).sum(),
			own_ns: procs.iter().map(|listed| listed.code.total.own_ns).sum(),
			procs
		}
	}

	pub fn share(&self, ns: u64) -> f64 {
		match self.own_ns {
			0 => 0.0,
			own => ns as f64 * 100.0 / own as f64
		}
	}

	pub fn lines(&self) -> Vec<Line> {
		let mut lines: Vec<Line> = vec![];
		let mut known: HashMap<(usize, u32), usize> = HashMap::new();
		for (at, listed) in self.procs.iter().enumerate() {
			for row in listed.listing.iter().flatten().filter(|row| row.hits != 0) {
				let Some(number) = row.line else { continue };
				let line = *known.entry((at, number)).or_insert_with(|| {
					let file = listed.code.file.as_deref().unwrap_or("line");
					lines.push(Line {
						place: format!("{}:{}  {}", file, number, name(&listed.code)),
						hits: 0,
						own_ns: 0,
						callee_ns: 0
					});
					lines.len() - 1
				});
				lines[line].hits += row.hits;
				lines[line].own_ns += row.own_ns;
				lines[line].callee_ns += row.callee_ns;
			}
		}
		lines.sort_by(|a, b| b.own_ns.cmp(&a.own_ns).then_with(|| a.place.cmp(&b.place)));
		lines
	}

	pub fn json(&self) -> serde_json::Value {
		let row = |row: &Row| {
			json!({
				"offset": row.offset,
				"text": row.text,
				"line": row.line,
				"source": row.source,
				"hits": row.hits,
				"ns": row.own_ns,
				"callee_ns": row.callee_ns
			})
		};
		let procs: Vec<_> = self
			.procs
			.iter()
			.map(|listed| {
				let (code, total) = (&listed.code, listed.code.total);
				json!({
					"proc": code.proc.path,
					"override": code.proc.override_id,
					"old_code": code.code.old != 0,
					"file": code.file,
					"line": code.line,
					"calls": total.calls,
					"instructions": total.hits,
					"ns": total.own_ns,
					"callee_ns": total.callee_ns,
					"listing": listed.listing.as_ref().map(|rows| rows.iter().map(row).collect::<Vec<_>>())
				})
			})
			.collect();
		json!({
			"seconds": self.seconds,
			"ended": self.ended,
			"overhead_ns": self.overhead_ns,
			"instructions": self.hits,
			"ns": self.own_ns,
			"procs": procs
		})
	}
}

pub fn name(code: &ProfiledCode) -> String {
	let name = match code.proc.override_id {
		0 => code.proc.path.clone(),
		n => format!("{}#{}", code.proc.path, n)
	};
	if code.code.old != 0 {
		format!("{} (old code)", name)
	} else {
		name
	}
}

pub fn listing(disassembly: &Disassembly, measured: &[ProfiledInstruction], mut source: impl FnMut(u32) -> Option<String>) -> Vec<Row> {
	let measured: HashMap<u16, &ProfiledInstruction> = measured.iter().map(|instruction| (instruction.offset, instruction)).collect();
	let mut line = None;
	disassembly
		.instructions
		.iter()
		.map(|instruction| {
			let took = measured.get(&instruction.offset);
			let new_line = instruction.line != line;
			line = instruction.line;
			Row {
				offset: instruction.offset,
				text: match (&instruction.text, &disassembly.error) {
					(text, _) if !text.is_empty() => text.clone(),
					(_, Some(error)) => format!("(not read: {})", error),
					(_, None) => "(not read)".to_owned()
				},
				line,
				source: line.filter(|_| new_line).and_then(&mut source),
				hits: took.map_or(0, |took| took.hits),
				own_ns: took.map_or(0, |took| took.own_ns),
				callee_ns: took.map_or(0, |took| took.callee_ns)
			}
		})
		.collect()
}

pub fn stop<E: From<String>>(ended: &'static str, listed: Option<usize>, mut ask: impl FnMut(Request) -> Result<Response, E>) -> Result<Report, E> {
	let mut profile = |request| match ask(Request::Profile(request))? {
		Response::Profile(reply) => Ok(reply.map_err(E::from)),
		other => Err(E::from(format!("unexpected response from the world: {:?}", other)))
	};
	let ProfileReply::Totals(totals) = profile(ProfileRequest::Stop)?? else {
		return Err(E::from("unexpected response from the world: no totals".to_owned()));
	};
	let mut report = Report::new(totals, ended);
	let listed = listed.map_or(report.procs.len(), |listed| listed.min(report.procs.len()));
	let mut measured = vec![];
	for at in &report.procs[..listed] {
		let ProfileReply::Code(instructions) = profile(ProfileRequest::Code { code: at.code.code })?? else {
			return Err(E::from("unexpected response from the world: no instructions".to_owned()));
		};
		measured.push(instructions);
	}
	let mut sources = Sources::of(match ask(Request::WorldInfo)? {
		Response::WorldInfo { args, cwd, .. } => local::world_dmb(&args, &cwd).and_then(|dmb| dmb.parent().map(PathBuf::from)),
		_ => None
	});
	for (at, measured) in measured.iter().enumerate() {
		let disassembly = match ask(Request::Disassemble {
			code: report.procs[at].code.code
		})? {
			Response::Disassemble(disassembly) => disassembly.map_err(E::from)?,
			other => return Err(E::from(format!("unexpected response from the world: {:?}", other)))
		};
		let file = disassembly.file.clone().or_else(|| report.procs[at].code.file.clone());
		report.procs[at].listing = Some(listing(&disassembly, measured, |line| sources.line(file.as_deref(), line)));
	}
	Ok(report)
}

struct Sources {
	directory: Option<PathBuf>,
	files: HashMap<String, Option<Vec<String>>>
}

impl Sources {
	fn of(directory: Option<PathBuf>) -> Self {
		Self {
			directory,
			files: HashMap::new()
		}
	}

	fn line(&mut self, file: Option<&str>, line: u32) -> Option<String> {
		let (file, directory) = (file?, self.directory.as_ref()?);
		let lines = self
			.files
			.entry(file.to_owned())
			.or_insert_with(|| Some(std::fs::read_to_string(directory.join(file)).ok()?.lines().map(str::to_owned).collect()));
		let text = lines.as_ref()?.get(line.checked_sub(1)? as usize)?.trim();
		(!text.is_empty()).then(|| text.to_owned())
	}
}

pub fn time(ns: u64) -> String {
	match ns {
		0..1_000 => format!("{} ns", ns),
		1_000..1_000_000 => format!("{:.1} us", ns as f64 / 1e3),
		1_000_000..1_000_000_000 => format!("{:.1} ms", ns as f64 / 1e6),
		_ => format!("{:.2} s", ns as f64 / 1e9)
	}
}

pub fn html(report: &Report) -> String {
	let data = report.json().to_string().replace('<', "\\u003c");
	include_str!("profile.html").replacen("/*DATA*/", &data, 1)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn code(path: &str, own_ns: u64) -> ProfiledCode {
		ProfiledCode {
			proc: ProcRef {
				path: path.to_owned(),
				override_id: 0
			},
			code: CodeRef { proc: 1, old: 0 },
			file: Some("code.dm".to_owned()),
			line: Some(3),
			total: ProfiledTotal {
				calls: 1,
				hits: 4,
				own_ns,
				callee_ns: 0
			}
		}
	}

	fn instruction(offset: u16, text: &str, line: Option<u32>) -> BytecodeInstruction {
		BytecodeInstruction {
			offset,
			words: vec![],
			text: text.to_owned(),
			line
		}
	}

	fn report() -> Report {
		let totals = ProfileTotals {
			seconds: 2.0,
			running: false,
			overhead_ns: 12.5,
			codes: vec![code("/b", 100), code("/a", 300), code("/c", 100)]
		};
		let mut report = Report::new(totals, "time");
		let disassembly = Disassembly {
			proc: report.procs[0].code.proc.clone(),
			file: Some("code.dm".to_owned()),
			instructions: vec![
				instruction(0, "DbgLine 3", Some(3)),
				instruction(2, "PushInt 1", Some(3)),
				instruction(4, "DbgLine 4", Some(4)),
				instruction(6, "CallGlob 0 /b", Some(4)),
				instruction(9, "End", Some(4)),
			],
			error: None
		};
		let took = |offset, hits, own_ns, callee_ns| ProfiledInstruction {
			offset,
			hits,
			own_ns,
			callee_ns
		};
		let measured = [took(0, 1, 10, 0), took(2, 1, 40, 0), took(4, 1, 50, 0), took(6, 1, 200, 100)];
		report.procs[0].listing = Some(listing(&disassembly, &measured, |line| Some(format!("source of {}", line))));
		report
	}

	#[test]
	fn procs_are_listed_longest_first_then_by_name() {
		let report = report();
		let names: Vec<&str> = report.procs.iter().map(|listed| listed.code.proc.path.as_str()).collect();
		assert_eq!(names, ["/a", "/b", "/c"]);
		assert_eq!((report.hits, report.own_ns), (12, 500));
		assert_eq!(report.share(100), 20.0);
	}

	#[test]
	fn a_listing_has_every_instruction_and_each_line_once() {
		let report = report();
		let rows = report.procs[0].listing.as_ref().unwrap();
		assert_eq!(rows.len(), 5);
		let sources: Vec<Option<&str>> = rows.iter().map(|row| row.source.as_deref()).collect();
		assert_eq!(sources, [Some("source of 3"), None, Some("source of 4"), None, None]);
		assert_eq!((rows[4].text.as_str(), rows[4].hits, rows[4].own_ns), ("End", 0, 0));
		assert_eq!((rows[3].hits, rows[3].own_ns, rows[3].callee_ns), (1, 200, 100));
	}

	#[test]
	fn lines_add_up_their_instructions() {
		let lines = report().lines();
		let said: Vec<(&str, u64, u64, u64)> = lines
			.iter()
			.map(|line| (line.place.as_str(), line.hits, line.own_ns, line.callee_ns))
			.collect();
		assert_eq!(said, [("code.dm:4  /a", 2, 250, 100), ("code.dm:3  /a", 2, 50, 0)]);
	}

	#[test]
	fn times_are_in_the_unit_that_reads_best() {
		let said: Vec<String> = [0, 999, 1_000, 41_250, 2_500_000, 1_500_000_000].into_iter().map(time).collect();
		assert_eq!(said, ["0 ns", "999 ns", "1.0 us", "41.2 us", "2.5 ms", "1.50 s"]);
	}

	#[test]
	fn the_page_holds_the_report_and_nothing_ends_its_script_early() {
		let mut report = report();
		report.procs[0].listing.as_mut().unwrap()[1].text = "PushVal \"<!-- <script></script>\"".to_owned();
		let page = html(&report);
		assert!(page.contains(r#""proc":"/a""#) && !page.contains("/*DATA*/"), "{}", page);
		assert!(page.contains(r#"\u003c!-- \u003cscript>\u003c/script>"#), "{}", page);
		assert!(!page.contains("<!--"), "{}", page);
		assert_eq!(page.matches("<script").count(), 2);
		assert_eq!(page.matches("</script>").count(), 2);
	}
}
