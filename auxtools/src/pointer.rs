//! BYOND's pointers (`&x`): reading what one names, through the function
//! byondapi exports for it. That function is in every build by name, so
//! there's no pattern to find and no layout to know.

use std::sync::OnceLock;

use byond_scan::{Anchor, Extract, Recipe, VersionRange};

use crate::{
	raw_types::{
		funcs,
		values::{Value as RawValue, ValueData, ValueTag}
	},
	Value
};

const READ_POINTER: Recipe = Recipe {
	name: "Byond_ReadPointer",
	versions: VersionRange { min: 1637, max: 1688 },
	anchor: Anchor::Export(c"Byond_ReadPointer"),
	hops: &[],
	extract: Extract::Entry
};

/// Finds `bool Byond_ReadPointer(const CByondValue *ptr, CByondValue *result)`
/// (a CByondValue is a value as BYOND keeps them: a type byte and four
/// bytes) for the wrapper that calls it, which catches the runtime error
/// some builds make of a pointer whose object is gone
fn found() -> bool {
	static FOUND: OnceLock<bool> = OnceLock::new();
	*FOUND.get_or_init(|| match crate::find_recipe(&READ_POINTER) {
		Ok(address) => {
			unsafe { funcs::read_pointer_byond = address as *const std::ffi::c_void };
			true
		}
		Err(_) => false
	})
}

/// Whether pointers can be read in this world
pub fn readable() -> bool {
	found()
}

/// What a pointer names now, as BYOND has it: no reference is taken, so
/// it's for comparing, or for making a [`Value`] of straight away. `None`
/// when BYOND can't read through it: what it named is gone (an object that
/// was deleted, an index past a list's end), or `pointer` isn't one.
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

/// What a pointer names now
pub fn read(pointer: &Value) -> Option<Value> {
	read_raw(pointer).map(|raw| unsafe { Value::from_raw(raw) })
}
