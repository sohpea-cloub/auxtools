use crate::{byond_string, raw_types, DMResult, Value};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

fn get_next_id() -> f32 {
	static NEXT_WEAKREF_ID: AtomicU32 = AtomicU32::new(1);
	let id = NEXT_WEAKREF_ID.fetch_add(1, Relaxed);
	id as f32
}

/// A weak reference to a datum, which can be stored globally or sent to other
/// threads. Upgrade it only on the main thread. Every datum needs a
/// `__auxtools_weakref_id` var.
#[derive(Copy, Clone)]
pub struct WeakValue {
	inner: raw_types::values::Value,
	id: f32
}

impl WeakValue {
	pub(crate) fn new(val: &Value) -> DMResult<Self> {
		if let Ok(id) = val.get_number(byond_string!("__auxtools_weakref_id")) {
			return Ok(Self { inner: val.raw, id });
		}

		let id = get_next_id();
		val.set(byond_string!("__auxtools_weakref_id"), Value::from(id))?;
		Ok(Self { inner: val.raw, id })
	}

	/// `None` if the datum has been deleted.
	pub fn upgrade(&self) -> Option<Value> {
		let real_val = unsafe { Value::from_raw(self.inner) };
		let id = real_val.get_number(byond_string!("__auxtools_weakref_id")).ok()?;

		if self.id != id {
			return None;
		}

		Some(real_val)
	}

	pub fn upgrade_or_null(&self) -> Value {
		match self.upgrade() {
			Some(v) => v,
			None => Value::NULL
		}
	}
}

impl Value {
	pub fn as_weak(&self) -> DMResult<WeakValue> {
		WeakValue::new(self)
	}
}
