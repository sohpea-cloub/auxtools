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

struct Entry {
	id: u32,
	words: Vec<u32>
}

static BEFORE_SHUTDOWN: Mutex<Vec<Entry>> = Mutex::new(vec![]);

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
		t.set_bytecode(code_of(&find("/proc/auxtest_code_r")?));
		runtime_tables::set_proc_entry_word(t.id, ProcEntryWords::BYTECODE, reloaded.id);
	}
	*BEFORE_SHUTDOWN.lock().unwrap() = vec![replaced, reloaded];
	Ok(Value::from(true))
}

byond_ffi_fn! { auxtest_code_after_shutdown() {
	let entries = std::mem::take(&mut *BEFORE_SHUTDOWN.lock().unwrap());
	let changed: Vec<String> = entries
		.iter()
		.filter_map(|entry| {
			let words = unsafe { runtime_tables::misc_words(entry.id) };
			(words.as_ref() != Some(&entry.words)).then(|| format!("code entry {} has {:?}, not {:?}", entry.id, words, entry.words))
		})
		.collect();
	if changed.is_empty() {
		Some(String::new())
	} else {
		Some(format!("shutdown didn't give back the code it replaced: {}", changed.join(", ")))
	}
} }
