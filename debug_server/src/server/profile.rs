use auxtools::*;

use super::{Codes, Server};
use auxtools_protocol::*;

use crate::{profiler, unnamed};

impl Server {
	pub(super) fn profile(&mut self, request: ProfileRequest) -> Result<ProfileReply, String> {
		match request {
			ProfileRequest::Start { procs } => {
				let only: Vec<u32> = procs
					.iter()
					.map(|proc| Codes::proc_info(proc).map(|info| info.code.proc))
					.collect::<Result<_, _>>()?;
				if profiler::running() {
					return Err("another profile is running".to_owned());
				}
				profiler::start((!only.is_empty()).then_some(only));
				self.sessions.profiling = self.sessions.requester();
				Ok(ProfileReply::Started)
			}
			ProfileRequest::Stop => {
				self.own_profile()?;
				profiler::stop();
				self.profile_totals()
			}
			ProfileRequest::Totals => {
				self.own_profile()?;
				self.profile_totals()
			}
			ProfileRequest::Code { code } => {
				self.own_profile()?;
				let proc = Proc::from_id(raw_types::procs::ProcId(code.proc)).ok_or_else(|| format!("the world has no proc {:#x}", code.proc))?;
				let bytecode = match code.old {
					0 => unsafe { proc.bytecode_mut_ptr() }.0 as usize,
					old => (self.codes.old_codes.borrow().get(old as usize - 1).copied())
						.ok_or_else(|| format!("{} has no old code {}", proc.path, old))?
				};
				if !profiler::exists() {
					return Err(NO_PROFILE.to_owned());
				}
				profiler::instructions(bytecode)
					.map(ProfileReply::Code)
					.ok_or_else(|| format!("{} wasn't profiled", proc.path))
			}
		}
	}

	fn own_profile(&self) -> Result<(), String> {
		if profiler::exists() && self.sessions.profiling != self.sessions.requester() {
			return Err("the profile belongs to another client".to_owned());
		}
		Ok(())
	}

	fn profile_totals(&self) -> Result<ProfileReply, String> {
		let totals = profiler::totals().ok_or(NO_PROFILE)?;
		let codes = totals
			.codes
			.into_iter()
			.filter_map(|(proc, bytecode, total)| {
				let proc = Proc::from_id(raw_types::procs::ProcId(proc))?;
				let current = unsafe { proc.bytecode_mut_ptr() }.0 as usize;
				let old = if bytecode == current { 0 } else { self.codes.old_code_number(bytecode) };
				let (file, line) = start_of(bytecode, &proc);
				Some(ProfiledCode {
					proc: unnamed::proc_ref(&proc),
					code: CodeRef { proc: proc.id.0, old },
					file,
					line,
					total
				})
			})
			.collect();
		Ok(ProfileReply::Totals(ProfileTotals {
			seconds: totals.seconds,
			running: totals.running,
			overhead_ns: totals.overhead,
			codes
		}))
	}
}

const NO_PROFILE: &str = "no profile has been started";

fn start_of(bytecode: usize, proc: &Proc) -> (Option<String>, Option<u32>) {
	let (dbg_file, dbg_line) = (dmasm::Opcode::DbgFile.word(), dmasm::Opcode::DbgLine.word());
	let (current, len) = unsafe { proc.bytecode_mut_ptr() };
	let words = if current as usize == bytecode {
		unsafe { std::slice::from_raw_parts(current, len as usize) }.to_vec()
	} else {
		unsafe { auxtools::bytecode_at(bytecode as *const u32) }.unwrap_or_default()
	};
	match words.as_slice() {
		[file_op, file, rest @ ..] if *file_op == dbg_file => {
			let name = unsafe { StringRef::from_id(raw_types::strings::StringId(*file)) };
			let line = match rest {
				[line_op, line, ..] if *line_op == dbg_line => Some(*line),
				_ => None
			};
			(Some(String::from_utf8_lossy(name.data()).into_owned()), line)
		}
		_ => (None, None)
	}
}
