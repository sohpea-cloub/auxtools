use byond_scan::VersionRange;

pub const FIRST_515: u32 = 1637;

/// The first build that keeps a type's typepath value in its entry and
/// indexes them for text2path. Entries are 8 bytes longer from here, and a
/// new type has to be registered. Earlier builds work the value out on
/// demand.
pub const TYPEPATHS: u32 = 1641;

/// The first Linux build compiled without frame pointers, much as 516 is.
/// From here the pairs of globals sit the other way round (a queue's back
/// before its front, a table's count before its pointer), `get_length`
/// returns a value and not a count, misc entries are interned by a function
/// that copies the words, and the interpreter keeps the running proc's frame
/// in ESI, not EDI. Windows did not change.
pub const LINUX_FRAMELESS: u32 = 1644;

/// The first 516 build. With it came alists, and `icon_w` and `icon_z` in an
/// appearance, before its mouse pointers.
pub const FIRST_516: u32 = 1648;

/// The first Linux build where `remove_from_list` takes its arguments on the
/// stack. Every build before it wants the list in `eax:edx`. Read off the
/// prologue of every build from 1669 to 1675.
pub const LINUX_REMOVE_FROM_LIST_ON_STACK: u32 = 1674;

/// The first build whose typepath registration checks for no type, which
/// changed its first instructions.
pub const TYPEPATH_NONE_CHECK: u32 = 1686;

/// The newest build the recipes and the layouts were checked on. A recipe
/// is tried on a newer build all the same. What rests on a layout that no
/// recipe confirms is refused there until the build is checked: resource
/// reload, which changes BYOND's appearance tree, and the text2path cache.
pub const NEWEST_CHECKED: u32 = 1689;

pub const SUPPORTED: &[(u32, u32)] = &[(515, FIRST_515), (516, FIRST_516)];

/// Every build from `first` on. A recipe with this range is the one tried
/// on builds newer than any checked.
pub const fn since(first: u32) -> VersionRange {
	VersionRange::from(first)
}

/// The builds from `first` up to the one before `next`
pub const fn until(first: u32, next: u32) -> VersionRange {
	VersionRange { min: first, max: next - 1 }
}
