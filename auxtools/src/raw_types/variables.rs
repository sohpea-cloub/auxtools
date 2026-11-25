use super::strings;

#[cfg(windows)]
#[repr(C)]
pub struct VariableNameIdTable {
	pub entries: *const strings::StringId,
	pub count: u32
}

#[cfg(unix)]
#[repr(C)]
pub struct VariableNameIdTable {
	pub count: u32,
	pub entries: *const strings::StringId
}
