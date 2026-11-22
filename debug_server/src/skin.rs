use std::sync::OnceLock;

use auxtools::{
	raw_types::values::{ValueData, ValueTag},
	registry, Proc, Value
};

use auxtools_protocol::{Reconnected, SkinReloaded, SkinRequest};

const NO_SKIN: u32 = 0xFFFF;

struct Place {
	global: usize,
	data: usize,
	field: usize
}

fn place() -> Result<&'static Place, String> {
	static PLACE: OnceLock<Result<Place, String>> = OnceLock::new();
	PLACE
		.get_or_init(|| {
			Ok(Place {
				global: registry::find!("world_skin")?,
				data: registry::find!("world_skin_data")?,
				field: registry::find!("world_skin_field")?
			})
		})
		.as_ref()
		.map_err(|e| format!("cannot locate the skin: {}", e))
}

fn field() -> Result<*mut u32, String> {
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
		Ok(data.add(place.field) as *mut u32)
	}
}

pub(crate) fn hosted() -> bool {
	std::env::current_exe()
		.ok()
		.and_then(|exe| exe.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()))
		.is_some_and(|stem| stem == "dreamseeker")
}

pub(crate) fn run(code: &str, params: &[(&str, &Value)]) -> Result<Value, String> {
	let names: Vec<&str> = params.iter().map(|(name, _)| *name).collect();
	let values: Vec<&Value> = params.iter().map(|(_, value)| *value).collect();
	let body = dmasm::compiler::compile_body_in(code, &names, None, crate::world_byond()).map_err(|e| e.to_string())?;
	let assembly = dmasm::assembler::assemble(&body.nodes, &mut crate::assemble_env::AssembleEnv).map_err(|e| format!("{:?}", e))?;
	let proc = Proc::find("/proc/auxtools_expr_stub").ok_or("the world has no evaluation stub")?;
	proc.set_bytecode(assembly);
	let result = crate::server::Server::call_with_locals(&proc, &body.locals, &values)?.map_err(|e| e.message)?;
	result.as_list().and_then(|list| list.get(1)).map_err(|e| e.message)
}

pub(crate) fn text(value: &str) -> Result<Value, String> {
	Value::from_string(value).map_err(|e| e.message)
}

pub fn set(request: &SkinRequest) -> Result<SkinReloaded, String> {
	let field = field()?;
	if !std::path::Path::new(&request.path).is_file() {
		return Err(format!("{}: no such file", request.path));
	}
	let resource = run("fcopy_rsc(file)", &[("file", &text(&request.path)?)])?;
	if resource.raw.tag != ValueTag::Resource {
		return Err(format!("{}: could not cache the skin resource", request.path));
	}
	let clients = clients()?;
	unsafe { *field = resource.raw.data.id };
	Ok(SkinReloaded { clients, hosted: hosted() })
}

pub(crate) fn clients() -> Result<u32, String> {
	let count = run("var/n = 0\nfor(var/client/C)\n\tif(C.key)\n\t\tn++\nn\n", &[])?;
	count.as_number().map(|n| n as u32).map_err(|e| e.message)
}

pub fn state() -> Result<Option<(u32, String)>, String> {
	let id = unsafe { *field()? };
	if id == NO_SKIN {
		return Ok(None);
	}
	let resource = unsafe { Value::new(ValueTag::Resource, ValueData { id }) };
	let text = run("file2text(skin)", &[("skin", &resource)])?;
	Ok(Some((id, text.as_string().map_err(|e| e.message)?)))
}

pub fn reconnect() -> Result<Reconnected, String> {
	if hosted() {
		return Ok(Reconnected::Hosted);
	}
	let count = run(
		"var/n = 0\nfor(var/client/C)\n\tif(C.key)\n\t\twinset(C, null, \"command=.reconnect\")\n\t\tn++\nn\n",
		&[]
	)?;
	let count = count.as_number().map_err(|e| e.message)?;
	Ok(Reconnected::Clients(count as u32))
}
