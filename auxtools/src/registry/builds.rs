use byond_scan::VersionRange;

pub const FIRST_515: u32 = 1637;

/// Entries store the typepath value (8 bytes more), and new types must be
/// registered for text2path.
pub const TYPEPATHS: u32 = 1641;

/// Linux without frame pointers: paired globals swap order, `get_length`
/// returns a value, misc interning copies words, and the frame is in ESI.
pub const LINUX_FRAMELESS: u32 = 1644;

/// Adds alists, and `icon_w` and `icon_z` before an appearance's mouse
/// pointers.
pub const FIRST_516: u32 = 1648;

/// Earlier Linux builds take `remove_from_list`'s list in `eax:edx`.
pub const LINUX_REMOVE_FROM_LIST_ON_STACK: u32 = 1674;

/// Typepath registration checks for no type, changing its prologue.
pub const TYPEPATH_NONE_CHECK: u32 = 1686;

/// Newer builds still try recipes but refuse resource reload and the
/// text2path cache.
pub const NEWEST_CHECKED: u32 = 1689;

pub const SUPPORTED: &[(u32, u32)] = &[(515, FIRST_515), (516, FIRST_516)];

/// A recipe with this range is the one tried on builds newer than any checked.
pub const fn since(first: u32) -> VersionRange {
	VersionRange::from(first)
}

/// The builds from `first` up to, but not including, `next`.
pub const fn until(first: u32, next: u32) -> VersionRange {
	VersionRange { min: first, max: next - 1 }
}
