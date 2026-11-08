use auxtools::*;

pub struct AssembleEnv;

impl AssembleEnv {
	// Runs `code` in the expression stub with `name` set to `arg` and returns
	// what it evaluates to
	fn eval_with_name(code: &str, arg: &str) -> Option<Value> {
		let arg = Value::from_string(arg).ok()?;
		let expr = dmasm::compiler::compile_expr(code, &["name"]).unwrap();
		let assembly = dmasm::assembler::assemble(&expr, &mut Self).unwrap();

		let proc = Proc::find("/proc/auxtools_expr_stub")?;
		proc.set_bytecode(assembly);

		let res = proc.call(&[&arg]).ok()?.as_list().ok()?.get(1).ok()?;

		if res == Value::NULL {
			return None;
		}

		Some(res)
	}
}

impl dmasm::assembler::AssembleEnv for AssembleEnv {
	fn get_string_index(&mut self, string: &[u8]) -> Option<u32> {
		let string = StringRef::from_raw(string).ok()?;
		let id = string.get_id();

		// We leak here because the assembled code now holds a reference to this
		// string
		std::mem::forget(string);

		Some(id.0)
	}

	fn get_variable_name_index(&mut self, name: &[u8]) -> Option<u32> {
		let id = self.get_string_index(name)?;

		unsafe {
			let mut names = (*raw_types::funcs::VARIABLE_NAMES).entries;

			for i in 0..(*raw_types::funcs::VARIABLE_NAMES).count {
				if (*names).0 == id {
					return Some(i);
				}

				names = names.add(1);
			}

			None
		}
	}

	fn get_proc_index(&mut self, path: &str) -> Option<u32> {
		Proc::find(path).map(|p| p.id.0)
	}

	// This is... pretty crazy
	fn get_type(&mut self, path: &str) -> Option<(u8, u32)> {
		match path {
			// text2path() returns null for these two
			"/list" => return Some((raw_types::values::ValueTag::ListTypepath as u8, 0)),
			"/savefile" => return Some((raw_types::values::ValueTag::SaveFileTypepath as u8, 0)),
			// text2path() gives /client's path id as the data, but the compiler
			// always writes 0
			"/client" => return Some((raw_types::values::ValueTag::ClientTypepath as u8, 0)),
			_ => {}
		}

		if let Some(res) = Self::eval_with_name("text2path(name)", path) {
			return Some((res.raw.tag as u8, unsafe { res.raw.data.id }));
		}

		// A reference to a proc override, like /obj/foo/bar, has no /proc/ in
		// its path, and text2path() doesn't know it
		Proc::find(path).map(|p| (raw_types::values::ValueTag::ProcRef as u8, p.id.0))
	}

	// fcopy_rsc() hands back the cache entry a file already has, and adds the
	// file to the cache if it isn't there yet
	fn get_resource_index(&mut self, path: &str) -> Option<u32> {
		let res = Self::eval_with_name("fcopy_rsc(name)", path)?;

		if res.raw.tag != raw_types::values::ValueTag::Resource {
			return None;
		}

		Some(unsafe { res.raw.data.id })
	}
}
