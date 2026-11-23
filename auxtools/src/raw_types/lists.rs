use super::values;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct ListId(pub u32);

/// 32 bytes: BYOND reads the key at `+0` and the children at `+16` and `+20`.
#[repr(C)]
pub struct AssociativeListEntry {
	/// Holds no reference; the same value also sits in a `vector_part` slot.
	pub key: values::Value,
	/// Holds a reference, released when the tree is freed.
	pub value: values::Value,
	pub left: *mut AssociativeListEntry,
	pub right: *mut AssociativeListEntry,
	pub parent: *mut AssociativeListEntry,
	/// 0 is red, 1 is black.
	pub color: u8
}

/// The 8-byte record an `/alist` id points to in the alist table.
#[repr(C)]
pub struct Alist {
	pub root: *mut AssociativeListEntry,
	/// Starts at 1.
	pub refcount: u32
}

#[repr(C)]
pub struct List {
	pub vector_part: *mut values::Value,
	pub assoc_part: *mut AssociativeListEntry,
	pub allocated: u32,
	pub length: u32,
	pub refcount: u32,
	unknown: u32
}
