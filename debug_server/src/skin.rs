use std::sync::OnceLock;

use auxtools::{
	byond_scan::{Anchor, Extract, OperandSelect, Recipe, RegMemDispSelect, Register, SignatureTreatment, VersionRange},
	raw_types::values::{ValueData, ValueTag},
	Proc, Value
};

use crate::server_types::{Reconnected, SkinReloaded, SkinRequest};

const NO_SKIN: u32 = 0xFFFF;

#[cfg(unix)]
mod recipes {
	use super::*;

	const EARLY: &str = "C7 44 24 04 AF 00 00 00 89 1C 24 E8 ?? ?? ?? ?? 89 B5 ?? ?? ?? ?? 89 34 24 E8 ?? ?? ?? ?? 85 C0 89 C7 74 ?? A1 ?? ?? ?? ?? \
	                     8B 40 0C 8B 50 ?? 81 FA FF FF 00 00";
	const LATER: &str = "68 AF 00 00 00 53 0F B7 FF E8 ?? ?? ?? ?? 89 3C 24 E8 ?? ?? ?? ?? 89 C6 83 C4 10 85 C0 74 ?? A1 ?? ?? ?? ?? 8B 40 0C 8B 50 \
	                     ?? 81 FA FF FF 00 00";
	const EARLY_BUILDS: VersionRange = VersionRange { min: 1637, max: 1643 };
	const LATER_BUILDS: VersionRange = VersionRange { min: 1644, max: 1688 };

	const fn recipe(versions: VersionRange, signature: &'static str, extract: Extract) -> Recipe {
		Recipe {
			name: "world_skin",
			versions,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, signature),
			hops: &[],
			extract
		}
	}

	pub const GLOBAL: &[Recipe] = &[
		recipe(EARLY_BUILDS, EARLY, Extract::AbsMem(OperandSelect::FirstAbsMem)),
		recipe(LATER_BUILDS, LATER, Extract::AbsMem(OperandSelect::FirstAbsMem))
	];
	pub const DATA: &[Recipe] = &[
		recipe(EARLY_BUILDS, EARLY, Extract::RegMemDisp(RegMemDispSelect::First { base: Register::EAX })),
		recipe(LATER_BUILDS, LATER, Extract::RegMemDisp(RegMemDispSelect::First { base: Register::EAX }))
	];
	pub const FIELD: &[Recipe] = &[
		recipe(
			EARLY_BUILDS,
			EARLY,
			Extract::RegMemDisp(RegMemDispSelect::Nth {
				base: Register::EAX,
				index: 1
			})
		),
		recipe(
			LATER_BUILDS,
			LATER,
			Extract::RegMemDisp(RegMemDispSelect::Nth {
				base: Register::EAX,
				index: 1
			})
		)
	];
}

#[cfg(windows)]
mod recipes {
	use super::*;

	const SIGNATURE: &str = "68 AF 00 00 00 8D 8D ?? ?? ?? ?? E8 ?? ?? ?? ?? 56 C7 45 FC ?? ?? ?? ?? E8 ?? ?? ?? ?? 8B D8 83 C4 04 85 DB 0F 84 ?? \
	                         ?? ?? ?? A1 ?? ?? ?? ?? 8B 48 0C 8B 41 ?? 3D FF FF 00 00";
	const BUILDS: VersionRange = VersionRange { min: 1637, max: 1688 };

	const fn recipe(extract: Extract) -> Recipe {
		Recipe {
			name: "world_skin",
			versions: BUILDS,
			anchor: Anchor::Signature(SignatureTreatment::NoOffset, SIGNATURE),
			hops: &[],
			extract
		}
	}

	pub const GLOBAL: &[Recipe] = &[recipe(Extract::AbsMem(OperandSelect::FirstAbsMem))];
	pub const DATA: &[Recipe] = &[recipe(Extract::RegMemDisp(RegMemDispSelect::First { base: Register::EAX }))];
	pub const FIELD: &[Recipe] = &[recipe(Extract::RegMemDisp(RegMemDispSelect::First { base: Register::ECX }))];
}

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
				global: auxtools::find_recipe_of(recipes::GLOBAL)?,
				data: auxtools::find_recipe_of(recipes::DATA)?,
				field: auxtools::find_recipe_of(recipes::FIELD)?
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
