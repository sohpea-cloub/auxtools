use crate::{byond_string, raw_types, DMResult, StringRef, Value};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

fn next_id() -> u64 {
	static NEXT_WEAKREF_ID: AtomicU64 = AtomicU64::new(1);
	NEXT_WEAKREF_ID.fetch_add(1, Relaxed)
}

fn id_text(id: u64) -> String {
	id.to_string()
}

fn parse_id(text: &[u8]) -> Option<u64> {
	std::str::from_utf8(text).ok()?.parse().ok()
}

fn id_of(val: &Value) -> Option<u64> {
	let text = StringRef::from_value(val.get(byond_string!("__auxtools_weakref_id")).ok()?)?;
	parse_id(text.data())
}

/// A weak reference to a datum. You can store it globally or send it to other
/// threads. Upgrade it only on BYOND's thread. Every datum needs a
/// `__auxtools_weakref_id` var.
#[derive(Copy, Clone)]
pub struct WeakValue {
	inner: raw_types::values::Value,
	id: u64,
	world: u32
}

impl WeakValue {
	pub(crate) fn new(val: &Value) -> DMResult<Self> {
		let world = crate::world_generation();
		if let Some(id) = id_of(val) {
			return Ok(Self { inner: val.raw, id, world });
		}

		let id = next_id();
		val.set(byond_string!("__auxtools_weakref_id"), Value::from_string(id_text(id))?)?;
		Ok(Self { inner: val.raw, id, world })
	}

	/// Returns `None` if the datum was deleted or auxtools has shut down since
	/// the handle was made. Panics on any thread other than BYOND's.
	pub fn upgrade(&self) -> Option<Value> {
		assert!(crate::on_byond_thread(), "WeakValue::upgrade must run on BYOND's thread");
		if self.world != crate::world_generation() {
			return None;
		}
		let real_val = unsafe { Value::from_raw(self.inner) };

		if id_of(&real_val) != Some(self.id) {
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn ids_past_f32_precision_stay_distinct() {
		let ids = [1 << 24, (1 << 24) + 1, u32::MAX as u64 + 1];
		for id in ids {
			assert_eq!(
				parse_id(id_text(id).as_bytes()),
				Some(id),
				"parsing {:?} did not give back id {}",
				id_text(id),
				id
			);
		}
		assert_ne!(id_text(ids[0]), id_text(ids[1]));
	}
}
