use std::sync::Mutex;

use auxtools::{
	runtime_tables::{self, ProcEntryWords},
	*
};

fn find(path: &str) -> Result<Proc, Runtime> {
	Proc::find(path).ok_or_else(|| runtime!("{} is not defined", path))
}

fn code_of(proc: &Proc) -> Vec<u32> {
	unsafe { proc.bytecode() }.to_vec()
}

fn entry_of(proc: &Proc) -> u32 {
	unsafe { runtime_tables::proc_entry(proc.id) }.unwrap().0[ProcEntryWords::BYTECODE]
}

#[hook("/proc/auxtest_shared_code")]
fn test_shared_code() {
	let mut failures = vec![];

	let (big_a, big_b) = (find("/proc/auxtest_big_a")?, find("/proc/auxtest_big_b")?);
	let big_code = code_of(&big_a);
	if big_code.len() != u16::MAX as usize {
		return Err(runtime!("test_shared_code: /proc/auxtest_big_a has {} words, not 65535", big_code.len()));
	}
	if entry_of(&big_a) != entry_of(&big_b) {
		return Err(runtime!("test_shared_code: /proc/auxtest_big_a and /proc/auxtest_big_b don't share code"));
	}
	let small_code = code_of(&find("/proc/auxtest_code_r")?);
	if unsafe { big_a.set_bytecode(small_code.clone()) }.is_ok() {
		failures.push("replacing the code of a 65,535-word proc that shares it succeeded");
	}
	if code_of(&big_b) != big_code {
		failures.push("replacing the code of a 65,535-word proc changed the code of a proc that shared it");
	}
	if code_of(&big_a) != big_code {
		failures.push("a 65,535-word proc that couldn't get code of its own got new code");
	}

	let (p, q) = (find("/proc/auxtest_code_p")?, find("/proc/auxtest_code_q")?);
	let q_code = code_of(&q);
	unsafe {
		p.set_bytecode(small_code.clone()).map_err(|e| runtime!("test_shared_code: {}", e))?;
		runtime_tables::set_proc_entry_word(p.id, ProcEntryWords::BYTECODE, entry_of(&q));
	}
	let other_code = code_of(&find("/proc/auxtest_code_s")?);
	unsafe { p.set_bytecode(other_code.clone()) }.map_err(|e| runtime!("test_shared_code: {}", e))?;
	if code_of(&q) != q_code {
		failures.push("replacing the code of a proc that shares its entry since an earlier replacement changed the other proc's code");
	}
	if code_of(&p) != other_code {
		failures.push("a proc that shares its entry since an earlier replacement didn't get its new code");
	}

	if !failures.is_empty() {
		return Err(runtime!("test_shared_code: {}", failures.join("; ")));
	}
	Ok(Value::from(true))
}

struct Entry {
	id: u32,
	words: Vec<u32>
}

struct BeforeShutdown {
	replaced: Entry,
	reloaded: Entry
}

static BEFORE_SHUTDOWN: Mutex<Option<BeforeShutdown>> = Mutex::new(None);

#[hook("/proc/auxtest_code_before_shutdown")]
fn code_before_shutdown() {
	let (t, u) = (find("/proc/auxtest_code_t")?, find("/proc/auxtest_code_u")?);
	let replaced = Entry {
		id: entry_of(&t),
		words: code_of(&t)
	};
	let reloaded = Entry {
		id: entry_of(&u),
		words: code_of(&u)
	};
	unsafe {
		t.set_bytecode(code_of(&find("/proc/auxtest_code_r")?))
			.map_err(|e| runtime!("code_before_shutdown: {}", e))?;
		runtime_tables::set_proc_entry_word(t.id, ProcEntryWords::BYTECODE, reloaded.id);
	}
	*BEFORE_SHUTDOWN.lock().unwrap() = Some(BeforeShutdown { replaced, reloaded });
	Ok(Value::from(true))
}

byond_ffi_fn! { auxtest_code_after_shutdown() {
	let Some(saved) = BEFORE_SHUTDOWN.lock().unwrap().take() else {
		return Some("nothing was saved before shutdown".to_owned());
	};
	let changed: Vec<String> = [saved.replaced, saved.reloaded]
		.iter()
		.filter_map(|entry| {
			let words = unsafe { runtime_tables::misc_words(entry.id) };
			(words.as_ref() != Some(&entry.words)).then(|| format!("code entry {} has {:?}, not {:?}", entry.id, words, entry.words))
		})
		.collect();
	if changed.is_empty() {
		Some("SUCCESS".to_owned())
	} else {
		Some(format!("shutdown didn't give back the code it replaced: {}", changed.join(", ")))
	}
} }
