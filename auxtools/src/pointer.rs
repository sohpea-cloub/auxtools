use std::sync::OnceLock;

use crate::{
	raw_types::{
		funcs,
		values::{Value as RawValue, ValueData, ValueTag}
	},
	Value
};

fn found() -> bool {
	static FOUND: OnceLock<bool> = OnceLock::new();
	*FOUND.get_or_init(|| match crate::registry::find!("Byond_ReadPointer") {
		Ok(address) => {
			unsafe { funcs::read_pointer_byond = address as *const std::ffi::c_void };
			true
		}
		Err(_) => false
	})
}

/// Whether this BYOND build exports `Byond_ReadPointer`.
pub fn readable() -> bool {
	found()
}

/// The result holds no reference. Compare or convert it at once.
pub fn read_raw(pointer: &Value) -> Option<RawValue> {
	if pointer.raw.tag != ValueTag::Pointer {
		return None;
	}
	if !found() {
		return None;
	}
	let mut result = RawValue {
		tag: ValueTag::Null,
		data: ValueData { id: 0 }
	};
	(unsafe { funcs::read_pointer(&mut result, &pointer.raw) } == 1).then_some(result)
}

pub fn read(pointer: &Value) -> Option<Value> {
	read_raw(pointer).map(|raw| unsafe { Value::from_raw(raw) })
}
