use super::{raw_types, string};
use crate::{list, runtime, runtime::DMResult};
use std::{
	ffi::{CStr, CString},
	fmt,
	marker::PhantomData
};

/// A value that a DM var can hold. Don't keep it after your hook returns.
///
/// Dropping it releases its reference, so its raw value can only be read, with
/// [`Value::raw`].
///
/// ```compile_fail
/// let mut value = auxtools::Value::NULL;
/// value.raw.tag = auxtools::raw_types::values::ValueTag::String;
/// ```
pub struct Value {
	pub(crate) raw: raw_types::values::Value,
	phantom: PhantomData<*mut ()>
}

impl PartialEq for Value {
	fn eq(&self, other: &Self) -> bool {
		unsafe { self.raw.tag == other.raw.tag && self.raw.data.id == other.raw.data.id }
	}
}

impl Eq for Value {}

impl std::hash::Hash for Value {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		unsafe {
			self.raw.tag.hash(state);
			self.raw.data.id.hash(state);
		}
	}
}

impl Drop for Value {
	fn drop(&mut self) {
		if self.holds_no_reference() {
			return;
		}
		unsafe {
			raw_types::funcs::dec_ref_count(self.raw);
		}
	}
}

impl Value {
	pub const NULL: Self = Self {
		raw: raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { number: 0.0 }
		},
		phantom: PhantomData {}
	};

	const fn holds_no_reference(&self) -> bool {
		matches!(
			self.raw.tag,
			raw_types::values::ValueTag::Null | raw_types::values::ValueTag::Number | raw_types::values::ValueTag::World
		)
	}

	const fn world_value(id: u32) -> Self {
		Self {
			raw: raw_types::values::Value {
				tag: raw_types::values::ValueTag::World,
				data: raw_types::values::ValueData { id }
			},
			phantom: PhantomData {}
		}
	}

	#[track_caller]
	pub fn world() -> Self {
		crate::expect_live("Value::world");
		Self::world_value(0)
	}

	#[track_caller]
	pub fn globals() -> Self {
		crate::expect_live("Value::globals");
		Self::world_value(1)
	}

	pub unsafe fn new(tag: raw_types::values::ValueTag, data: raw_types::values::ValueData) -> Value {
		let value = Value {
			raw: raw_types::values::Value { tag, data },
			phantom: PhantomData {}
		};
		if !value.holds_no_reference() {
			raw_types::funcs::inc_ref_count(value.raw);
		}
		value
	}

	#[deprecated(note = "please use the `NULL` const instead")]
	pub const fn null() -> Value {
		Self::NULL
	}

	pub const unsafe fn turf_by_id_unchecked(id: u32) -> Value {
		Value {
			raw: raw_types::values::Value {
				tag: raw_types::values::ValueTag::Turf,
				data: raw_types::values::ValueData { id }
			},
			phantom: PhantomData {}
		}
	}

	pub fn turf_by_id(id: u32) -> DMResult {
		crate::check_live("Value::turf_by_id")?;
		let world = Self::world();
		let max_x = world.get_number(crate::byond_string!("maxx"))? as u32;
		let max_y = world.get_number(crate::byond_string!("maxy"))? as u32;
		let max_z = world.get_number(crate::byond_string!("maxz"))? as u32;
		if (0..max_x * max_y * max_z).contains(&(id - 1)) {
			Ok(unsafe { Value::turf_by_id_unchecked(id) })
		} else {
			Err(runtime!("Attempted to get tile with invalid ID {}", id))
		}
	}

	/// Coordinates start at 1, as in DM.
	pub fn turf(x: u32, y: u32, z: u32) -> DMResult {
		crate::check_live("Value::turf")?;
		let world = Self::world();
		let max_x = world.get_number(crate::byond_string!("maxx"))? as u32;
		let max_y = world.get_number(crate::byond_string!("maxy"))? as u32;
		let max_z = world.get_number(crate::byond_string!("maxz"))? as u32;
		let x = x - 1;
		let y = y - 1;
		let z = z - 1;
		if (0..max_x).contains(&x) && (0..max_y).contains(&y) && (0..max_z).contains(&z) {
			Ok(unsafe { Value::turf_by_id_unchecked(x + y * max_x + z * max_x * max_y) })
		} else {
			Err(runtime!("Attempted to get out-of-range tile at coords {} {} {}", x + 1, y + 1, z + 1))
		}
	}

	fn get_by_id(&self, name_id: raw_types::strings::StringId) -> DMResult {
		let mut val = raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { id: 0 }
		};

		unsafe {
			if raw_types::funcs::get_variable(&mut val, self.raw, name_id) != 1 {
				let varname: String = string::StringRef::from_id(name_id).into();
				return Err(runtime!("Could not read {}.{}", &self, varname));
			}

			Ok(Self::from_raw(val))
		}
	}

	fn set_by_id(&self, name_id: raw_types::strings::StringId, new_value: raw_types::values::Value) -> Result<(), runtime::Runtime> {
		unsafe {
			if raw_types::funcs::set_variable(self.raw, name_id, new_value) != 1 {
				let varname: String = string::StringRef::from_id(name_id).into();
				return Err(runtime!("Could not write to {}.{}", self, varname));
			}
		}
		Ok(())
	}

	pub fn get<S: Into<string::StringRef>>(&self, name: S) -> DMResult {
		let name = name.into();
		self.get_by_id(name.get_id())
	}

	pub fn get_number<S: Into<string::StringRef>>(&self, name: S) -> DMResult<f32> {
		self.get(name)?.as_number()
	}

	pub fn get_string<S: Into<string::StringRef>>(&self, name: S) -> DMResult<String> {
		self.get(name)?.as_string()
	}

	pub fn get_list<S: Into<string::StringRef>>(&self, name: S) -> DMResult<list::List> {
		let var = self.get(name)?;
		var.as_list()
	}

	pub fn set<S: Into<string::StringRef>, V: Into<Value>>(&self, name: S, value: V) -> DMResult<()> {
		let value = value.into();

		self.set_by_id(name.into().get_id(), value.raw)?;
		Ok(())
	}

	pub fn as_number(&self) -> DMResult<f32> {
		match self.raw.tag {
			raw_types::values::ValueTag::Number => unsafe { Ok(self.raw.data.number) },
			_ => Err(runtime!("Attempt to interpret non-number value as number"))
		}
	}

	pub fn as_string(&self) -> DMResult<String> {
		match self.raw.tag {
			raw_types::values::ValueTag::String => unsafe { Ok(string::StringRef::from_id(self.raw.data.string).into()) },
			_ => Err(runtime!("Attempt to interpret non-string value as String"))
		}
	}

	pub fn as_list(&self) -> DMResult<list::List> {
		list::List::from_value(self)
	}

	pub fn call<S: AsRef<str>>(&self, procname: S, args: &[&Value]) -> DMResult {
		let mut ret = raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { id: 0 }
		};

		let name_ref = string::StringRef::new(&procname.as_ref().replace("_", " "))?;
		let mut raw_args: Vec<_> = args.iter().map(|e| e.raw).collect();

		unsafe {
			for v in args {
				raw_types::funcs::inc_ref_count(v.raw);
			}

			if raw_types::funcs::call_datum_proc_by_name(
				&mut ret,
				Value::NULL.raw,
				2,
				name_ref.get_id(),
				self.raw,
				raw_args.as_mut_ptr(),
				raw_args.len(),
				0,
				0
			) == 1
			{
				return Ok(Value::from_raw_owned(ret));
			}
		}

		Err(runtime!("External proc call failed"))
	}

	pub fn to_dmstring(&self) -> DMResult<string::StringRef> {
		match self.raw.tag {
			raw_types::values::ValueTag::Null | raw_types::values::ValueTag::Number | raw_types::values::ValueTag::String => {
				return string::StringRef::new(format!("{}", self).as_str())
			}

			_ => {}
		}

		let mut id = raw_types::strings::StringId(0);

		unsafe {
			if raw_types::funcs::to_string(&mut id, self.raw) != 1 {
				return Err(runtime!("to_string failed on {:?}", self));
			}
			Ok(string::StringRef::from_id(id))
		}
	}

	pub fn to_string(&self) -> DMResult<String> {
		match self.raw.tag {
			raw_types::values::ValueTag::Null | raw_types::values::ValueTag::Number | raw_types::values::ValueTag::String => {
				return Ok(format!("{}", self))
			}

			_ => {}
		}

		let mut id = raw_types::strings::StringId(0);

		unsafe {
			if raw_types::funcs::to_string(&mut id, self.raw) != 1 {
				return Err(runtime!("to_string failed on {:?}", self));
			}
			Ok(String::from(string::StringRef::from_id(id)))
		}
	}

	pub fn get_type(&self) -> Result<String, runtime::Runtime> {
		self.get(crate::byond_string!("type"))?.to_string()
	}

	pub fn is_exact_type<S: AsRef<str>>(&self, typepath: S) -> bool {
		match self.get_type() {
			Err(_) => false,
			Ok(my_type) => my_type == typepath.as_ref()
		}
	}

	pub fn is_truthy(&self) -> bool {
		match self.raw.tag {
			raw_types::values::ValueTag::Null => false,
			raw_types::values::ValueTag::Number => unsafe { self.raw.data.number != 0.0 },

			_ => true
		}
	}

	pub fn from_string<S: AsRef<str>>(data: S) -> DMResult {
		Self::from_string_raw(data.as_ref().as_bytes())
	}

	pub fn from_string_raw(data: &[u8]) -> DMResult {
		crate::check_live("Value::from_string")?;
		let string = CString::new(data).map_err(|_| runtime!("tried to create string containing NUL"))?;

		unsafe {
			let mut id = raw_types::strings::StringId(0);

			assert_eq!(raw_types::funcs::get_string_id(&mut id, string.as_ptr()), 1);

			Ok(Value::new(raw_types::values::ValueTag::String, raw_types::values::ValueData {
				string: id
			}))
		}
	}

	/// Takes a new reference to `v`.
	pub unsafe fn from_raw(v: raw_types::values::Value) -> Self {
		Value::new(v.tag, v.data)
	}

	pub const fn raw(&self) -> raw_types::values::Value {
		self.raw
	}

	/// Takes over a reference to `v` that the caller holds.
	pub const unsafe fn from_raw_owned(v: raw_types::values::Value) -> Value {
		Value {
			raw: v,
			phantom: PhantomData {}
		}
	}
}

impl Clone for Value {
	fn clone(&self) -> Value {
		unsafe { Value::from_raw(self.raw) }
	}
}

impl fmt::Display for Value {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self.raw.tag {
			raw_types::values::ValueTag::String => unsafe {
				let mut entry: *mut raw_types::strings::StringEntry = std::ptr::null_mut();
				assert_eq!(raw_types::funcs::get_string_table_entry(&mut entry, self.raw.data.string), 1);
				write!(f, "{:?}", string::text_of(CStr::from_ptr((*entry).data).to_bytes()))
			},
			_ => write!(f, "{}", self.raw)
		}
	}
}

impl fmt::Debug for Value {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self.raw.tag {
			raw_types::values::ValueTag::String => fmt::Display::fmt(self, f),
			_ => write!(f, "{:?}", self.raw)
		}
	}
}
