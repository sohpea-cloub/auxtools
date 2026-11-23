use std::cell::UnsafeCell;

use crate::{inventory, StringRef};

#[macro_export]
macro_rules! byond_string {
	($s:literal) => {{
		static STORE: $crate::InternedString = $crate::InternedString::new($s);
		$crate::inventory::submit!(&STORE);
		STORE.get()
	}};
}

#[doc(hidden)]
pub struct InternedString {
	text: &'static str,
	string: UnsafeCell<Option<StringRef>>
}

unsafe impl Sync for InternedString {}

impl InternedString {
	pub const fn new(text: &'static str) -> Self {
		Self {
			text,
			string: UnsafeCell::new(None)
		}
	}

	#[track_caller]
	pub fn get(&self) -> StringRef {
		crate::expect_live("byond_string!");
		unsafe { &*self.string.get() }
			.clone()
			.unwrap_or_else(|| panic!("byond_string!({:?}) wasn't made at init", self.text))
	}
}

inventory::collect!(&'static InternedString);

pub fn setup_interned_strings() {
	for info in inventory::iter::<&'static InternedString> {
		let string = StringRef::new(info.text).expect("failed to create interned string");

		unsafe {
			*info.string.get() = Some(string);
		}
	}
}

pub fn destroy_interned_strings() {
	for info in inventory::iter::<&'static InternedString> {
		unsafe {
			*info.string.get() = None;
		}
	}
}
