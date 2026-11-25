use std::{ffi::CString, os::raw::c_char};

#[cfg(windows)]
use auxtools::byond_scan::{Anchor, Extract, Recipe, SignatureTreatment, VersionRange};
use auxtools::*;
use region::Protection;

static mut STRING_PTR: *mut *const c_char = std::ptr::null_mut();

#[cfg(windows)]
const GUEST_NAME_FORMAT: Recipe = Recipe {
	name: "guest_name_format",
	versions: VersionRange { min: 1637, max: 1688 },
	anchor: Anchor::Signature(
		SignatureTreatment::NoOffset,
		"68 ?? ?? ?? ?? 50 E8 ?? ?? ?? ?? 83 C4 0C 8D 8D ?? ?? ?? ?? E8 ?? ?? ?? ?? 8B 85 ?? ?? ?? ??"
	),
	hops: &[],
	extract: Extract::Offset(1)
};

#[init(full)]
fn ckey_override_init() -> Result<(), String> {
	#[cfg(windows)]
	if let Ok(address) = find_recipe(&GUEST_NAME_FORMAT) {
		unsafe {
			STRING_PTR = address as *mut *const c_char;
		}
	}

	Ok(())
}

#[derive(Debug)]
pub enum Error {
	UnsupportedByondVersion,
	InvalidString
}

pub fn override_guest_ckey(name: &str) -> Result<(), Error> {
	unsafe {
		if STRING_PTR.is_null() {
			return Err(Error::UnsupportedByondVersion);
		}
	}

	let name = name.replace('%', "%%");

	let new_ptr = CString::new(name).map_err(|_| Error::InvalidString)?.into_raw();

	unsafe {
		region::protect(STRING_PTR as *const u8, 4, Protection::READ_WRITE_EXECUTE).unwrap();

		*STRING_PTR = new_ptr;
	}

	Ok(())
}
