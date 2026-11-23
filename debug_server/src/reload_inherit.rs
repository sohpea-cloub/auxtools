use super::*;

fn places(var: &str) -> Result<Vec<BuiltinWrite>, String> {
	[ConstValue::Null, ConstValue::Number(1.0), ConstValue::String("neuter".to_owned())]
		.iter()
		.find_map(|value| builtin_writes(var, value).ok())
		.ok_or_else(|| format!("{}: built-in var is unsupported by hot reload", var))
}

fn bounds_place(var: &str) -> Option<(usize, u32)> {
	Some(match var {
		"bound_x" => (0x40, 0xFFFF),
		"bound_y" => (0x40, 0xFFFF_0000),
		"bound_width" => (0x44, 0xFFFF),
		"bound_height" => (0x44, 0xFFFF_0000),
		"step_x" => (0x48, u32::MAX),
		"step_y" => (0x4C, u32::MAX),
		"step_size" => (0x50, u32::MAX),
		_ => return None
	})
}

pub(super) fn listed_entries(list: &[u32]) -> Vec<Vec<u32>> {
	let mut out = vec![];
	let mut at = 0;
	while at < list.len() {
		let len = if list.get(at + 1).is_some_and(|tag| tag & 0xFF == 0x2A) { 4 } else { 3 };
		out.push(list[at..(at + len).min(list.len())].to_vec());
		at += len;
	}
	out
}

fn listed(type_id: u32) -> Result<Vec<Vec<u32>>, String> {
	Ok(listed_entries(
		&runtime_tables::misc_words(runtime_tables::type_field(type_id, type_fields::BUILTIN_VALUES)?).unwrap_or_default()
	))
}

fn set_listed(type_id: u32, entries: &[Vec<u32>]) -> Result<(), String> {
	let words: Vec<u32> = entries.iter().flatten().copied().collect();
	let list = if words.is_empty() { NONE } else { runtime_tables::intern_misc(&words)? };
	runtime_tables::set_type_field(type_id, type_fields::BUILTIN_VALUES, list)
}

pub(super) fn builtin_in_table(type_id: u32, _var: &str) -> bool {
	type_and_ancestors(type_id)
		.into_iter()
		.any(|t| dmasm::reload::VAR_TABLE_BUILTINS.contains(&type_path(t).as_str()))
}

fn copy_table_entry(type_id: u32, from: u32, var: &str) -> Result<(), String> {
	let name = new_string(var)?;
	let key = name.get_id().0;
	let table = |t: u32| -> Result<Vec<u32>, String> {
		Ok(runtime_tables::misc_words(runtime_tables::type_field(t, type_fields::VAR_TABLE)?).unwrap_or_default())
	};
	let mut entries: Vec<Vec<u32>> = table(type_id)?.chunks(4).filter(|e| e[0] != key).map(<[u32]>::to_vec).collect();
	entries.extend(table(from)?.chunks(4).filter(|e| e[0] == key).map(<[u32]>::to_vec));
	entries.sort_by_key(|e| e[0]);
	let words: Vec<u32> = entries.into_iter().flatten().collect();
	let id = if words.is_empty() { NONE } else { runtime_tables::intern_misc(&words)? };
	runtime_tables::set_type_field(type_id, type_fields::VAR_TABLE, id)?;
	hold_table_strings(&[(type_id, words)], &[]);
	Ok(())
}

pub(super) fn copy_builtin(type_id: u32, from: u32, var: &str) -> Result<(), String> {
	use runtime_tables::{set_type_field, type_field};
	use BuiltinWrite::*;
	if builtin_in_table(type_id, var) {
		return copy_table_entry(type_id, from, var);
	}
	for place in places(var)? {
		match place {
			Word(offset, _) => set_type_field(type_id, offset, type_field(from, offset)?)?,
			Text(MOB_KEY, _) => {
				if let Ok(key) = type_string(from, MOB_KEY) {
					set_type_string(type_id, MOB_KEY, key)?
				}
			}
			Text(place, _) => set_type_string(type_id, place, type_string(from, place)?)?,
			Half(offset, _) => {
				let half = (type_field(from, offset & !3)? >> ((offset & 2) * 8)) as u16;
				runtime_tables::set_type_half(type_id, offset, half)?
			}
			Byte(offset, _) => {
				let byte = (type_field(from, offset & !3)? >> ((offset & 3) * 8)) as u8;
				runtime_tables::set_type_byte(type_id, offset, byte)?
			}
			Bits(offset, mask, _) => {
				let word = type_field(type_id, offset)?;
				set_type_field(type_id, offset, (word & !mask) | (type_field(from, offset)? & mask))?
			}
			Listed(name, ..) => {
				let name = name.get_id().0;
				let mut entries: Vec<Vec<u32>> = listed(type_id)?.into_iter().filter(|e| e[0] != name).collect();
				entries.extend(listed(from)?.into_iter().filter(|e| e[0] == name));
				set_listed(type_id, &entries)?;
				inherit_listed(type_id, from, name);
				if let Some((offset, mask)) = bounds_place(var) {
					let word = type_field(type_id, offset)?;
					set_type_field(type_id, offset, (word & !mask) | (type_field(from, offset)? & mask))?;
				}
			}
			Mob(var, _) => {
				let mut parent = None;
				if runtime_tables::set_mob_type(from, |mob| parent = Some(*mob)).is_err() {
					continue;
				}
				let parent = parent.unwrap();
				runtime_tables::set_mob_type(type_id, |mob| match var {
					"sight" => mob.sight = parent.sight,
					"see_in_dark" => mob.see_in_dark = parent.see_in_dark,
					_ => mob.see_invisible = parent.see_invisible
				})?
			}
		}
	}
	Ok(())
}

pub(super) fn prepare(inherited: &[BuiltinInherited], prepared: &mut Prepared, errors: &mut Vec<String>) {
	let mut out = vec![];
	for i in inherited {
		for path in &i.types {
			match find_type(path) {
				Ok(type_id) => {
					out.push((type_and_ancestors(type_id).len(), type_id, i.var.clone()));
					if !builtin_in_table(type_id, &i.var) {
						prepared.appearances.insert(type_id);
					}
				}
				Err(e) => errors.push(e)
			}
		}
	}
	out.sort();
	prepared.builtin_inherits = out.into_iter().map(|(_, t, var)| (t, var)).collect();
}

pub(super) fn apply(inherits: &[(u32, String)], failures: &mut Vec<String>) {
	for (type_id, var) in inherits {
		let copied = runtime_tables::type_field(*type_id, type_fields::PARENT).and_then(|parent| copy_builtin(*type_id, parent, var));
		if let Err(e) = copied {
			failures.push(format!("{}: could not inherit built-in {}: {}", type_path(*type_id), var, e));
		}
	}
}
