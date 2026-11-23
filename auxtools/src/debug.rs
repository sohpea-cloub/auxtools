use std::{cell::RefCell, collections::HashMap};

use crate::{
	raw_types::{funcs, procs},
	Proc, StringRef, Value
};

thread_local! {
	static LOCAL_NAMES: RefCell<HashMap<usize, Vec<String>>> = RefCell::new(HashMap::new());
}

pub fn set_local_names(bytecode: *const u32, names: Vec<String>) {
	LOCAL_NAMES.with(|map| map.borrow_mut().insert(bytecode as usize, names));
}

pub fn has_local_names(bytecode: *const u32) -> bool {
	LOCAL_NAMES.with(|map| map.borrow().contains_key(&(bytecode as usize)))
}

pub fn local_names_for(proc: &Proc, bytecode: *const u32) -> Vec<String> {
	LOCAL_NAMES
		.with(|map| map.borrow().get(&(bytecode as usize)).cloned())
		.unwrap_or_else(|| proc.local_names().into_iter().map(String::from).collect())
}

pub(crate) fn clear_local_names() {
	LOCAL_NAMES.with(|map| map.borrow_mut().clear());
}

pub struct StackFrame {
	pub context: *mut procs::ExecutionContext,
	pub instance: *mut procs::ProcInstance,
	pub proc: Proc,
	pub offset: u16,
	pub usr: Value,
	pub src: Value,
	pub dot: Value,
	pub args: Vec<(Option<StringRef>, Value)>,
	pub locals: Vec<(StringRef, Value)>,
	pub file_name: Option<StringRef>,
	pub line_number: Option<u32>
}

pub struct CallStacks {
	pub active: Vec<StackFrame>,
	pub suspended: Vec<Vec<StackFrame>>
}

impl StackFrame {
	unsafe fn from_context(context: *mut procs::ExecutionContext) -> StackFrame {
		let instance = *(*context).proc_instance();

		let proc = Proc::from_id((*instance).proc).unwrap();
		let offset = *(*context).bytecode_offset();
		let param_names = proc.parameter_names();
		let bytecode = *(*context).bytecode();
		let local_names = LOCAL_NAMES.with(|map| {
			map.borrow()
				.get(&(bytecode as usize))
				.map(|names| names.iter().map(|name| StringRef::new(name).ok()).collect::<Vec<_>>())
		});
		let local_names = local_names.unwrap_or_else(|| proc.local_names().into_iter().map(Some).collect());

		let usr = Value::from_raw((*instance).usr);
		let src = Value::from_raw((*instance).src);
		let dot = Value::from_raw(*(*context).dot());

		let args = (0..(*instance).args_count())
			.map(|i| {
				let name = param_names.get(i as usize).cloned();
				(name, Value::from_raw(*((*instance).args()).add(i as usize)))
			})
			.collect();

		let locals = (0..*(*context).locals_count())
			.map(|i| {
				let name = match local_names.get(i as usize) {
					Some(Some(name)) => name.clone(),
					_ => StringRef::new(&format!("<local {}>", i)).unwrap()
				};
				(name, Value::from_raw(*(*(*context).locals()).add(i as usize)))
			})
			.collect();

		let mut file_name = None;
		let mut line_number = None;
		if (*context).filename().valid() {
			file_name = Some(StringRef::from_id(*(*context).filename()));
			line_number = Some(*(*context).line());
		}

		StackFrame {
			context,
			instance,
			proc,
			offset,
			usr,
			src,
			dot,
			args,
			locals,
			file_name,
			line_number
		}
	}
}

enum CallStackKind {
	Active,
	Suspended
}

impl Default for CallStacks {
	fn default() -> Self {
		Self::new()
	}
}

impl CallStacks {
	pub fn new() -> CallStacks {
		CallStacks {
			active: Self::active(),
			suspended: Self::suspended()
		}
	}

	/// The running proc's stack, innermost frame first.
	pub fn active() -> Vec<StackFrame> {
		unsafe { CallStacks::from_context(*funcs::CURRENT_EXECUTION_CONTEXT, CallStackKind::Active) }
	}

	pub fn suspended() -> Vec<Vec<StackFrame>> {
		Self::suspended_first(usize::MAX)
	}

	pub fn suspended_first(limit: usize) -> Vec<Vec<StackFrame>> {
		let mut suspended = vec![];

		unsafe {
			let buffer = (*funcs::SUSPENDED_PROCS_BUFFER).buffer;
			for x in funcs::suspended_range().take(limit) {
				let instance = *buffer.add(x);
				let context = (*instance).context;
				suspended.push(CallStacks::from_context(context, CallStackKind::Suspended));
			}
		}

		suspended
	}

	pub fn suspended_instances() -> Vec<*mut procs::ProcInstance> {
		unsafe {
			let buffer = (*funcs::SUSPENDED_PROCS_BUFFER).buffer;
			funcs::suspended_range().map(|x| *buffer.add(x)).collect()
		}
	}

	fn from_context(mut context: *mut procs::ExecutionContext, kind: CallStackKind) -> Vec<StackFrame> {
		let mut frames = vec![];

		loop {
			if context.is_null() {
				break;
			}

			unsafe {
				frames.push(StackFrame::from_context(context));
				context = *(*context).parent_context();
			}
		}

		match kind {
			CallStackKind::Active => frames,
			CallStackKind::Suspended => frames.into_iter().rev().collect()
		}
	}
}
