use crate::{byond_string, raw_types, DMResult, StringRef, Value};
use std::{
	collections::hash_map::RandomState,
	hash::BuildHasher,
	sync::{
		atomic::{AtomicU64, Ordering::Relaxed},
		LazyLock
	}
};

type Id = (u64, u64);

struct Issuer {
	namespace: u64,
	next: AtomicU64
}

impl Issuer {
	fn new() -> Self {
		Self {
			namespace: RandomState::new().hash_one(std::process::id()),
			next: AtomicU64::new(1)
		}
	}

	fn issue(&self) -> Id {
		(self.namespace, self.next.fetch_add(1, Relaxed))
	}
}

static ISSUER: LazyLock<Issuer> = LazyLock::new(Issuer::new);

fn id_text((namespace, serial): Id) -> String {
	format!("{:x}-{}", namespace, serial)
}

fn parse_id(text: &[u8]) -> Option<Id> {
	let (namespace, serial) = std::str::from_utf8(text).ok()?.split_once('-')?;
	Some((u64::from_str_radix(namespace, 16).ok()?, serial.parse().ok()?))
}

fn id_of(val: &Value) -> Option<Id> {
	let text = StringRef::from_value(val.get(byond_string!("__auxtools_weakref_id")).ok()?)?;
	parse_id(text.data())
}

/// A weak reference to a datum. You can store it globally or send it to other
/// threads. Upgrade it only on BYOND's thread. Every datum needs a
/// `__auxtools_weakref_id` var.
#[derive(Copy, Clone)]
pub struct WeakValue {
	inner: raw_types::values::Value,
	id: Id,
	world: u32
}

impl WeakValue {
	pub(crate) fn new(val: &Value) -> DMResult<Self> {
		let world = crate::world_generation();
		if let Some(id) = id_of(val) {
			return Ok(Self { inner: val.raw, id, world });
		}

		let id = ISSUER.issue();
		val.set(byond_string!("__auxtools_weakref_id"), Value::from_string(id_text(id))?)?;
		Ok(Self { inner: val.raw, id, world })
	}

	/// Returns `None` if the datum was deleted or auxtools has shut down since
	/// the handle was made. Panics on any thread other than BYOND's.
	pub fn upgrade(&self) -> Option<Value> {
		assert!(crate::on_byond_thread(), "WeakValue::upgrade must run on BYOND's thread");
		if !crate::world_is_live() || self.world != crate::world_generation() {
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
		let namespace = Issuer::new().namespace;
		let ids = [1 << 24, (1 << 24) + 1, u32::MAX as u64 + 1].map(|serial| (namespace, serial));
		for id in ids {
			assert_eq!(
				parse_id(id_text(id).as_bytes()),
				Some(id),
				"parsing {:?} did not give back id {:?}",
				id_text(id),
				id
			);
		}
		assert_ne!(id_text(ids[0]), id_text(ids[1]));
	}

	#[test]
	fn a_second_library_never_reissues_an_adopted_id() {
		let (first, second) = (Issuer::new(), Issuer::new());
		let adopted = parse_id(id_text(first.issue()).as_bytes());
		let reissued = parse_id(id_text(second.issue()).as_bytes());
		assert_ne!(
			adopted, reissued,
			"the second library adopted the first library's id for a datum, then issued the same id to a new datum, which can reuse the slot"
		);
	}
}
