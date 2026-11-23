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
	unsafe { runtime_tables::proc_entry(proc.id()) }.unwrap().0[ProcEntryWords::BYTECODE]
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
		runtime_tables::set_proc_entry_word(p.id(), ProcEntryWords::BYTECODE, entry_of(&q));
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

#[hook("/proc/auxtest_entry_users")]
fn test_entry_users() {
	let mut failures = vec![];
	let r_code = code_of(&find("/proc/auxtest_code_r")?);
	let s_code = code_of(&find("/proc/auxtest_code_s")?);
	let fail = |e: String| runtime!("test_entry_users: {}", e);

	let (v, w) = (find("/proc/auxtest_code_v")?, find("/proc/auxtest_code_w")?);
	unsafe {
		v.set_bytecode(r_code.clone()).map_err(fail)?;
		let entry = entry_of(&v);
		raw_types::misc::add_ref(&raw_types::misc::MiscId::from_raw(entry));
		runtime_tables::set_proc_entry_word(w.id(), ProcEntryWords::BYTECODE, entry);
		v.set_bytecode(s_code.clone()).map_err(fail)?;
	}
	if code_of(&w) != r_code {
		failures.push(format!(
			"replacing a proc's code again changed the code of a proc pointed at its entry: {:?}, not {:?}",
			code_of(&w),
			r_code
		));
	}
	if code_of(&v) != s_code {
		failures.push("a proc whose entry another proc was pointed at didn't get its new code".to_owned());
	}

	let (x, locals_user) = (find("/proc/auxtest_code_x")?, find("/proc/auxtest_locals_user")?);
	let x_code = code_of(&x);
	let entry = unsafe { runtime_tables::intern_misc(&x_code) }.map_err(fail)?;
	unsafe { runtime_tables::set_proc_entry_word(x.id(), ProcEntryWords::BYTECODE, entry) };
	let locals = unsafe { runtime_tables::intern_misc(&x_code) }.map_err(fail)?;
	if locals != entry {
		return Err(runtime!(
			"test_entry_users: interning the same words again gave entry {}, not {}",
			locals,
			entry
		));
	}
	let user_locals = unsafe { runtime_tables::proc_entry(locals_user.id()) }.unwrap().0[ProcEntryWords::LOCALS];
	unsafe {
		runtime_tables::set_proc_entry_word(locals_user.id(), ProcEntryWords::LOCALS, locals);
		let replaced = x.set_bytecode(r_code.clone());
		runtime_tables::set_proc_entry_word(locals_user.id(), ProcEntryWords::LOCALS, user_locals);
		replaced.map_err(fail)?;
	}
	let words = unsafe { runtime_tables::misc_words(entry) };
	if words.as_ref() != Some(&x_code) {
		failures.push(format!(
			"replacing a proc's code changed an entry that is also a locals table: {:?}, not {:?}",
			words, x_code
		));
	}
	if code_of(&x) != r_code {
		failures.push("a proc whose code entry is also a locals table didn't get its new code".to_owned());
	}

	let (y, z) = (find("/proc/auxtest_code_y")?, find("/proc/auxtest_code_z")?);
	let y_code = code_of(&y);
	let shared = unsafe { runtime_tables::intern_misc(&y_code) }.map_err(fail)?;
	unsafe {
		runtime_tables::set_proc_entry_word(y.id(), ProcEntryWords::BYTECODE, shared);
		runtime_tables::set_proc_entry_word(z.id(), ProcEntryWords::BYTECODE, shared);
		runtime_tables::intern_misc(&y_code).map_err(fail)?;
	}
	let (at, _) = unsafe { y.bytecode_mut_ptr() };
	unsafe { own_code(&y) }.map_err(fail)?;
	let own = entry_of(&y);
	if own == shared || entry_of(&z) != shared {
		failures.push(format!(
			"own_code gave the procs entries {} and {}, not a new one and {}",
			own,
			entry_of(&z),
			shared
		));
	}
	if unsafe { y.bytecode_mut_ptr() }.0 != at {
		failures.push("own_code moved the proc's code".to_owned());
	}
	let patched = unsafe { std::mem::replace(&mut *at, u32::MAX) };
	let words = unsafe { runtime_tables::misc_words(shared) };
	unsafe { *at = patched };
	if words.as_ref() != Some(&y_code) || code_of(&z) != y_code {
		failures.push(format!("patching code after own_code changed the shared entry: {:?}", words));
	}
	unsafe { own_code(&y) }.map_err(fail)?;
	if entry_of(&y) != own {
		failures.push("own_code gave a proc a new entry although it had one of its own".to_owned());
	}

	if !failures.is_empty() {
		return Err(runtime!("test_entry_users: {}", failures.join("; ")));
	}
	Ok(Value::from(true))
}

struct Entry {
	id: u32,
	words: Vec<u32>
}

struct BeforeShutdown {
	replaced: Entry,
	reloaded: Entry,
	foreign: (raw_types::procs::ProcId, Vec<u32>)
}

static BEFORE_SHUTDOWN: Mutex<Option<BeforeShutdown>> = Mutex::new(None);

#[hook("/proc/auxtest_code_before_shutdown")]
fn code_before_shutdown() {
	let fail = |e: String| runtime!("code_before_shutdown: {}", e);
	let (t, u) = (find("/proc/auxtest_code_t")?, find("/proc/auxtest_code_u")?);
	let r_code = code_of(&find("/proc/auxtest_code_r")?);
	let replaced = Entry {
		id: entry_of(&t),
		words: code_of(&t)
	};
	let reloaded = Entry {
		id: entry_of(&u),
		words: code_of(&u)
	};
	unsafe {
		t.set_bytecode(r_code.clone()).map_err(fail)?;
		runtime_tables::set_proc_entry_word(t.id(), ProcEntryWords::BYTECODE, reloaded.id);
	}

	let f = find("/proc/auxtest_code_f")?;
	let theirs: &'static mut [u32] = Box::leak(code_of(&find("/proc/auxtest_code_s")?).into_boxed_slice());
	let foreign = (f.id(), theirs.to_vec());
	unsafe {
		let id = (*f.entry()).metadata.bytecode;
		let (own, len) = raw_types::misc::get_bytecode(id);
		raw_types::misc::set_bytecode(id, theirs.as_mut_ptr(), theirs.len() as u16);
		f.set_bytecode(r_code).map_err(fail)?;
		raw_types::misc::set_bytecode(id, own, len);
	}
	theirs.fill(0xDEAD_DEAD);

	*BEFORE_SHUTDOWN.lock().unwrap() = Some(BeforeShutdown { replaced, reloaded, foreign });
	Ok(Value::from(true))
}

byond_ffi_fn! { auxtest_code_after_shutdown() {
	let Some(saved) = BEFORE_SHUTDOWN.lock().unwrap().take() else {
		return Some("nothing was saved before shutdown".to_owned());
	};
	let mut changed: Vec<String> = [saved.replaced, saved.reloaded]
		.iter()
		.filter_map(|entry| {
			let words = unsafe { runtime_tables::misc_words(entry.id) };
			(words.as_ref() != Some(&entry.words)).then(|| format!("code entry {} has {:?}, not {:?}", entry.id, words, entry.words))
		})
		.collect();
	let (proc, words) = saved.foreign;
	let now = unsafe { runtime_tables::proc_entry(proc) }.and_then(|entry| unsafe { runtime_tables::misc_words(entry.0[ProcEntryWords::BYTECODE]) });
	if now.as_ref() != Some(&words) {
		changed.push(format!("a proc whose code another library had replaced has {:?}, not {:?}", now, words));
	}
	if changed.is_empty() {
		Some("SUCCESS".to_owned())
	} else {
		Some(format!("shutdown didn't give back the code it replaced: {}", changed.join(", ")))
	}
} }
