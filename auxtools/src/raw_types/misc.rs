#![allow(non_camel_case_types)]
use std::ffi::c_void;

use super::strings;

#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct MiscId(u32);

#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct BytecodeId(u32);

#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct LocalsId(u32);

#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct ParametersId(u32);

impl MiscId {
	pub const fn from_raw(id: u32) -> Self {
		Self(id)
	}

	pub const fn raw(self) -> u32 {
		self.0
	}
}

impl AsMiscId for MiscId {
	fn as_misc_id(&self) -> MiscId {
		*self
	}
}

pub trait AsMiscId {
	fn as_misc_id(&self) -> MiscId;
}

impl AsMiscId for BytecodeId {
	fn as_misc_id(&self) -> MiscId {
		MiscId(self.0)
	}
}

impl AsMiscId for LocalsId {
	fn as_misc_id(&self) -> MiscId {
		MiscId(self.0)
	}
}

impl AsMiscId for ParametersId {
	fn as_misc_id(&self) -> MiscId {
		MiscId(self.0)
	}
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Bytecode_V2 {
	pub count: u16,
	unk_0: u32,
	pub bytecode: *mut u32
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Locals_V2 {
	pub count: u16,
	unk_0: u32,
	pub names: *const strings::VariableId
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Parameters_V2 {
	params_count_mul_4: u16,
	unk_0: u32,
	pub data: *const ParametersData
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union Misc_V2 {
	pub bytecode: Bytecode_V2,
	pub locals: Locals_V2,
	pub parameters: Parameters_V2
}

#[repr(C)]
pub struct ParametersData {
	unk_0: u32,
	unk_1: u32,
	pub name: strings::VariableId,
	unk_4: u32
}

impl Parameters_V2 {
	pub const fn count(&self) -> usize {
		(self.params_count_mul_4 / 4) as usize
	}
}

pub fn set_bytecode(id: BytecodeId, new_bytecode: *mut u32, new_bytecode_count: u16) {
	let mut misc: *mut c_void = std::ptr::null_mut();
	unsafe {
		assert_eq!(super::funcs::get_misc_by_id(&mut misc, id.as_misc_id()), 1);
	}

	let misc = misc as *mut Misc_V2;
	unsafe {
		(*misc).bytecode.bytecode = new_bytecode;
		(*misc).bytecode.count = new_bytecode_count;
	}
}

/// Takes a reference to a misc entry for good, so BYOND never frees it.
pub fn add_ref<T: AsMiscId>(id: &T) {
	let mut misc: *mut c_void = std::ptr::null_mut();
	unsafe {
		assert_eq!(super::funcs::get_misc_by_id(&mut misc, id.as_misc_id()), 1);
	}

	let misc = misc as *mut Misc_V2;
	unsafe {
		(*misc).locals.unk_0 += 1;
	}
}

pub fn get_bytecode(id: BytecodeId) -> (*mut u32, u16) {
	let mut misc: *mut c_void = std::ptr::null_mut();
	unsafe {
		assert_eq!(super::funcs::get_misc_by_id(&mut misc, id.as_misc_id()), 1);
	}

	let misc = misc as *mut Misc_V2;
	unsafe { ((*misc).bytecode.bytecode, (*misc).bytecode.count) }
}

pub fn get_locals(id: LocalsId) -> (*const strings::VariableId, usize) {
	let mut misc: *mut c_void = std::ptr::null_mut();
	unsafe {
		assert_eq!(super::funcs::get_misc_by_id(&mut misc, id.as_misc_id()), 1);
	}

	let misc = misc as *mut Misc_V2;
	unsafe { ((*misc).locals.names, (*misc).locals.count as usize) }
}

pub fn get_parameters(id: ParametersId) -> (*const ParametersData, usize) {
	let mut misc: *mut c_void = std::ptr::null_mut();
	unsafe {
		assert_eq!(super::funcs::get_misc_by_id(&mut misc, id.as_misc_id()), 1);
	}

	let misc = misc as *mut Misc_V2;
	unsafe { ((*misc).parameters.data, (*misc).parameters.count()) }
}
