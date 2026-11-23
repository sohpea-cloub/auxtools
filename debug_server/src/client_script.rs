use std::sync::{Mutex, OnceLock};

use auxtools::{
	raw_types::values::{ValueData, ValueTag},
	registry, shutdown, StringRef, Value
};

use auxtools_protocol::{ScriptReloaded, ScriptRequest};

const NO_TEXT: u32 = 0xFFFF;

struct Place {
	global: usize,
	data: usize,
	count: usize
}

fn place() -> Result<&'static Place, String> {
	static PLACE: OnceLock<Result<Place, String>> = OnceLock::new();
	PLACE
		.get_or_init(|| {
			Ok(Place {
				global: registry::find!("client_script_send")?,
				data: registry::find!("client_script_data")?,
				count: registry::find!("client_script_count")?
			})
		})
		.as_ref()
		.map_err(|e| format!("cannot locate the client script: {}", e))
}

#[cfg_attr(not(feature = "test-commands"), allow(dead_code))]
struct Fields {
	text: *mut u32,
	list: *const *const u32,
	count: *const u16
}

fn fields() -> Result<Fields, String> {
	let place = place()?;
	unsafe {
		let holder = *(place.global as *const *const u8);
		if holder.is_null() {
			return Err("world data is not loaded yet".to_owned());
		}
		let data = *(holder.add(place.data) as *const *mut u8);
		if data.is_null() {
			return Err("world data is not loaded yet".to_owned());
		}
		Ok(Fields {
			text: data.add(place.count - 8) as *mut u32,
			list: data.add(place.count - 4) as *const *const u32,
			count: data.add(place.count) as *const u16
		})
	}
}

impl Fields {
	#[cfg(feature = "test-commands")]
	fn files(&self) -> Vec<u32> {
		unsafe {
			let count = *self.count as usize;
			let list = *self.list;
			if count == 0 || list.is_null() {
				return vec![];
			}
			std::slice::from_raw_parts(list, count).to_vec()
		}
	}

	#[cfg(feature = "test-commands")]
	fn text(&self) -> Option<u32> {
		let id = unsafe { *self.text };
		(id != NO_TEXT).then_some(id)
	}
}

static OUR_TEXT: Mutex<Option<Option<u32>>> = Mutex::new(None);

#[shutdown]
fn forget_world() {
	*OUR_TEXT.lock().unwrap() = None;
}

fn check_built(fields: &Fields, built: Option<&str>) -> Result<(), String> {
	if OUR_TEXT.lock().unwrap().is_some() {
		return Ok(());
	}
	let expected = match built {
		Some(text) => Some(StringRef::new(text).map_err(|e| e.message)?),
		None => None
	};
	let found = unsafe { *fields.text };
	if found == expected.as_ref().map_or(NO_TEXT, |string| string.get_id().0) {
		return Ok(());
	}
	Err(
		"the client script does not match the build. Rebuild the world. If the source already matches, auxtools does not support this BYOND script \
		 layout"
			.to_owned()
	)
}

fn write_text(fields: &Fields, text: Option<Value>) {
	let id = match text {
		Some(value) => {
			let id = unsafe { value.raw.data.id };
			std::mem::forget(value);
			Some(id)
		}
		None => None
	};
	let old = OUR_TEXT.lock().unwrap().replace(id);
	unsafe { *fields.text = id.unwrap_or(NO_TEXT) };
	if let Some(Some(old)) = old {
		unsafe {
			auxtools::raw_types::funcs::dec_ref_count(auxtools::raw_types::values::Value {
				tag: ValueTag::String,
				data: ValueData { id: old }
			});
		}
	}
}

#[cfg(feature = "test-commands")]
fn full(fields: &Fields) -> Result<String, String> {
	let mut script = String::new();
	for id in fields.files() {
		let file = unsafe { Value::new(ValueTag::Resource, ValueData { id }) };
		let text = crate::skin::run("file2text(file)", &[("file", &file)])?;
		script += &text.as_string().map_err(|e| e.message)?;
		script.push('\n');
	}
	if let Some(id) = fields.text() {
		let text = unsafe { Value::new(ValueTag::String, ValueData { id }) };
		script += &text.as_string().map_err(|e| e.message)?;
	}
	Ok(script)
}

pub fn set(request: &ScriptRequest) -> Result<ScriptReloaded, String> {
	let fields = fields()?;
	check_built(&fields, request.built.as_deref())?;
	let text = match &request.text {
		Some(text) => Some(Value::from_string(text).map_err(|e| e.message)?),
		None => None
	};
	let clients = crate::skin::clients()?;
	write_text(&fields, text);
	Ok(ScriptReloaded {
		clients,
		hosted: crate::skin::hosted()
	})
}

#[cfg(feature = "test-commands")]
pub fn state() -> Result<String, String> {
	let fields = fields()?;
	let text = fields.text().map_or("none".to_owned(), |id| id.to_string());
	let files: Vec<String> = fields.files().iter().map(u32::to_string).collect();
	Ok(format!("text {}\nfiles {}\n{}", text, files.join(" "), full(&fields)?))
}
