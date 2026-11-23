use super::*;

#[derive(Default)]
pub(super) struct PreparedReparents {
	parents: Vec<(u32, u32)>,
	tree: Option<runtime_tables::ParentPlan>,
	name_maps: Vec<(u32, Vec<(u32, Vec<u32>, u8)>)>,
	map_plans: Vec<runtime_tables::NameMapPlan>,
	relinks: Vec<(u32, u32)>,
	builtins: Vec<InheritedBuiltins>
}

struct InheritedBuiltins {
	type_id: u32,
	parent: u32,
	own: HashSet<u32>,
	own_names: HashSet<String>,
	name: TypeString,
	text: TypeString
}

static STAGED_PARENTS: Mutex<Option<HashMap<u32, u32>>> = Mutex::new(None);

pub(super) fn parent_of(type_id: u32) -> Result<u32, String> {
	if let Some(&parent) = STAGED_PARENTS.lock().unwrap().as_ref().and_then(|staged| staged.get(&type_id)) {
		return Ok(parent);
	}
	runtime_tables::type_field(type_id, type_fields::PARENT)
}

fn string_id(text: &str) -> Result<u32, String> {
	leaked_string(text)
}

pub(super) fn prepare(
	reparented: &[Reparented],
	type_vars: &mut TypeVars,
	staged: &mut StagedProcs,
	prepared: &mut Prepared,
	errors: &mut Vec<String>
) {
	*STAGED_PARENTS.lock().unwrap() = Some(HashMap::new());
	let mut tops = vec![];
	for r in reparented {
		match find_type(&r.type_path).and_then(|top| Ok((top, find_type(&r.parent)?))) {
			Ok((top, parent)) => tops.push((r, top, parent)),
			Err(e) => errors.push(format!("{}: parent type changed: {}", r.type_path, e))
		}
	}
	if tops.is_empty() {
		return;
	}
	if let Err(e) = type_vars.subtree(tops[0].1) {
		errors.push(e);
		return;
	}
	for &(_, top, parent) in &tops {
		type_vars.parents.as_mut().unwrap()[top as usize] = parent;
		STAGED_PARENTS.lock().unwrap().as_mut().unwrap().insert(top, parent);
		prepared.reparent.parents.push((top, parent));
	}
	tops.sort_by_key(|&(_, top, _)| type_and_ancestors(top).len());
	let mut maps = HashMap::new();
	for (r, top, parent) in tops {
		if type_and_ancestors(parent).contains(&top) {
			errors.push(format!(
				"{}: parent type changed: the new parent {} is a subtype of this type",
				r.type_path, r.parent
			));
			continue;
		}
		if let Err(e) = prepare_one(r, top, parent, &mut maps, type_vars, staged, prepared) {
			errors.push(format!("{}: parent type changed: {}", r.type_path, e));
		}
	}
	if !errors.is_empty() {
		return;
	}
	match runtime_tables::plan_parents(&prepared.reparent.parents) {
		Ok(plan) => prepared.reparent.tree = Some(plan),
		Err(e) => errors.push(format!("could not apply parent changes: {}", e))
	}
	for (type_id, map) in &prepared.reparent.name_maps {
		match runtime_tables::plan_name_map(*type_id, map) {
			Ok(plan) => prepared.reparent.map_plans.push(plan),
			Err(e) => errors.push(format!("{}: could not prepare proc-name map: {}", type_path(*type_id), e))
		}
	}
}

fn map_of(maps: &HashMap<u32, Vec<(u32, Vec<u32>, u8)>>, type_id: u32) -> Result<Vec<(u32, Vec<u32>, u8)>, String> {
	match maps.get(&type_id) {
		Some(map) => Ok(map.clone()),
		None => runtime_tables::name_map_entries(type_id)
	}
}

pub(super) fn finish() {
	*STAGED_PARENTS.lock().unwrap() = None;
}

fn prepare_one(
	r: &Reparented,
	top: u32,
	parent: u32,
	maps: &mut HashMap<u32, Vec<(u32, Vec<u32>, u8)>>,
	type_vars: &mut TypeVars,
	staged: &mut StagedProcs,
	prepared: &mut Prepared
) -> Result<(), String> {
	let subtree = type_vars.subtree(top)?;
	let mut by_path = HashMap::new();
	for t in &r.types {
		let id = find_type(&t.path)?;
		if !subtree.contains(&id) {
			return Err(format!("{} is outside the reparented subtree in the running world", t.path));
		}
		by_path.insert(id, t);
	}
	let depth = |t: u32| type_and_ancestors(t).len();
	let mut order = subtree.clone();
	order.sort_by_key(|&t| depth(t));

	let own_procs =
		|t: u32| -> Vec<u32> { runtime_tables::misc_words(runtime_tables::type_field(t, type_fields::PROCS).unwrap_or(NONE)).unwrap_or_default() };
	let chain_procs = |t: u32| -> HashSet<u32> {
		let mut procs = HashSet::new();
		for a in type_and_ancestors(t) {
			procs.extend(own_procs(a));
			if a == top {
				break;
			}
		}
		procs
	};
	let parent_map = map_of(maps, parent)?;
	let parent_lookup: HashMap<u32, (Vec<u32>, u8)> = parent_map.iter().map(|(k, ids, nl)| (*k, (ids.clone(), *nl))).collect();

	for &t in &order {
		let Some(own) = by_path.get(&t) else {
			return Err(format!("{} is missing from the reparent request", type_path(t)));
		};
		let up = parent_of(t)?;

		let declares: HashSet<u32> = own.declares.iter().map(|n| string_id(n)).collect::<Result<_, _>>()?;
		let sets: HashSet<u32> = own.sets.iter().chain(&own.declares).map(|n| string_id(n)).collect::<Result<_, _>>()?;
		let own_builtins: HashSet<u32> = if inherit::builtin_in_table(t, "") {
			own.builtins.iter().map(|n| string_id(n)).collect::<Result<_, _>>()?
		} else {
			HashSet::new()
		};
		let parent_declared = type_vars.declared(up)?.clone();
		let parent_table = type_vars.table(up)?.clone();
		let current_declared = type_vars.declared(t)?.clone();
		let current_table = type_vars.table(t)?.clone();
		let mut declared: Vec<[u32; 2]> = parent_declared.chunks(2).map(|c| [c[0], c[1]]).collect();
		for c in current_declared.chunks(2) {
			if declares.contains(&type_vars.name_of(c[0])) {
				declared.push([c[0], c[1]]);
			}
		}
		declared.sort_by_key(|e| type_vars.name_of(e[0]));
		let var_of = |name: u32| declared.iter().find(|e| type_vars.name_of(e[0]) == name).map(|e| e[0]);
		let mut table: BTreeMap<u32, [u32; 4]> = parent_table.chunks(4).map(|c| (c[0], [c[0], c[1], c[2], c[3]])).collect();
		for c in current_table.chunks(4) {
			if sets.contains(&c[0]) {
				let var = var_of(c[0]).ok_or_else(|| format!("{}: var {} has no declaration", type_path(t), c[0]))?;
				table.insert(c[0], [c[0], var, c[2], c[3]]);
			} else if own_builtins.contains(&c[0]) {
				table.insert(c[0], [c[0], c[1], c[2], c[3]]);
			}
		}
		*type_vars.declared(t)? = declared.into_iter().flatten().collect();
		*type_vars.table(t)? = table.into_values().flatten().collect();

		let chain = chain_procs(t);
		let current = map_of(maps, t)?;
		let mut map: BTreeMap<u32, (Vec<u32>, u8)> = BTreeMap::new();
		for (key, ids, no_link) in &current {
			let kept: Vec<u32> = ids.iter().copied().filter(|id| chain.contains(id)).collect();
			if !kept.is_empty() {
				map.insert(*key, (kept, *no_link));
			}
		}
		for (key, (ids, no_link)) in &parent_lookup {
			let entry = map.entry(*key).or_insert_with(|| (vec![], *no_link));
			entry.0.extend(ids.iter().copied());
		}
		for (key, ..) in &current {
			staged
				.maps
				.insert((t, *key), map.get(key).map(|(ids, _)| ids.clone()).unwrap_or_default());
		}
		for (key, (ids, _)) in &map {
			staged.maps.insert((t, *key), ids.clone());
		}
		let entries: Vec<(u32, Vec<u32>, u8)> = map.into_iter().map(|(k, (ids, nl))| (k, ids, nl)).collect();
		maps.insert(t, entries.clone());
		prepared.reparent.name_maps.push((t, entries));

		for proc in own_procs(t) {
			let Some(entry) = staged.entry(proc) else { continue };
			let next = entry.0[ProcEntryWords::NEXT_OVERRIDE];
			if chain.contains(&next) {
				continue;
			}
			let name = entry.0[1];
			let target = parent_lookup
				.get(&name)
				.filter(|(_, no_link)| *no_link == 0)
				.and_then(|(ids, _)| ids.first().copied())
				.unwrap_or(NONE);
			if target != next {
				staged.next_override.insert(proc, target);
				prepared.reparent.relinks.push((proc, target));
			}
		}

		prepared.reparent.builtins.push(InheritedBuiltins {
			type_id: t,
			parent: up,
			own: own.builtins.iter().map(|n| string_id(n)).collect::<Result<_, _>>()?,
			own_names: own.builtins.iter().cloned().collect(),
			name: match &own.name {
				ConstValue::String(name) => made_string(name)?,
				_ => TypeString::Built(NONE)
			},
			text: match &own.text {
				ConstValue::String(text) => made_string(text)?,
				_ => TypeString::Built(NONE)
			}
		});
		prepared.appearances.insert(t);
	}
	Ok(())
}

pub(super) fn apply_tree(prepared: &mut PreparedReparents) {
	let Some(tree) = prepared.tree.take() else {
		return;
	};
	tree.write();
	for plan in prepared.map_plans.drain(..) {
		plan.write();
	}
	for &(proc, next) in &prepared.relinks {
		runtime_tables::set_proc_entry_word(raw_types::procs::ProcId(proc), ProcEntryWords::NEXT_OVERRIDE, next);
	}
}

pub(super) fn apply_builtins(prepared: &PreparedReparents, warnings: &mut Vec<String>, failures: &mut Vec<String>) {
	for inherited in &prepared.builtins {
		match inherit_builtins(inherited) {
			Ok(None) => {}
			Ok(Some(bounds)) => warnings.push(format!(
				"{}: unsupported bounds \"{}\". Expected \"x1,y1 to x2,y2\" or \"WxH\". New instances use the parent's bounds until a rebuild",
				type_path(inherited.type_id),
				bounds
			)),
			Err(e) => failures.push(format!(
				"{}: could not inherit built-ins from the new parent: {}",
				type_path(inherited.type_id),
				e
			))
		}
	}
}

const RECORD_BUILTINS: &[&str] = &[
	"desc",
	"icon",
	"icon_state",
	"suffix",
	"maptext",
	"layer",
	"dir",
	"maptext_x",
	"maptext_y",
	"maptext_width",
	"maptext_height",
	"opacity",
	"density",
	"mouse_drop_zone",
	"gender",
	"mouse_opacity",
	"animate_movement",
	"appearance_flags",
	"key",
	"sight",
	"see_in_dark",
	"see_invisible"
];

fn inherit_builtins(b: &InheritedBuiltins) -> Result<Option<String>, String> {
	use runtime_tables::{set_type_field, type_field};
	let (t, p) = (b.type_id, b.parent);
	for var in RECORD_BUILTINS.iter().filter(|var| !b.own_names.contains(**var)) {
		inherit::copy_builtin(t, p, var)?;
	}
	if !b.own_names.contains("transform") {
		runtime_tables::copy_type_matrix(t, p, type_fields::TRANSFORM)?;
	}
	if !b.own_names.contains("color") {
		runtime_tables::copy_type_matrix(t, p, type_fields::COLOR_MATRIX)?;
	}
	set_type_string(t, type_fields::NAME, b.name.clone())?;
	set_type_string(t, type_fields::TEXT, b.text.clone())?;

	let parent_list = inherit::listed_entries(&runtime_tables::misc_words(type_field(p, type_fields::BUILTIN_VALUES)?).unwrap_or_default());
	let own_values = runtime_tables::misc_words(type_field(t, type_fields::BUILTIN_VALUES)?).unwrap_or_default();
	let mut words = vec![];
	for entry in parent_list.iter().filter(|e| !b.own.contains(&e[0])) {
		words.extend_from_slice(entry);
	}
	for entry in inherit::listed_entries(&own_values).iter().filter(|e| b.own.contains(&e[0])) {
		words.extend_from_slice(entry);
	}
	let list = if words.is_empty() { NONE } else { runtime_tables::intern_misc(&words)? };
	set_type_field(t, type_fields::BUILTIN_VALUES, list)?;

	let mut block = [0u32; 5];
	for (i, word) in block.iter_mut().enumerate() {
		*word = type_field(p, 0x40 + i * 4)?;
	}
	let unread = bounds_block(&mut block, &own_values, &b.own);
	for (i, word) in block.iter().enumerate() {
		set_type_field(t, 0x40 + i * 4, *word)?;
	}
	Ok(unread)
}

fn bounds_block(block: &mut [u32; 5], values: &[u32], own: &HashSet<u32>) -> Option<String> {
	let id = |name: &str| StringRef::new(name).ok().map(|name| name.get_id().0);
	let bounds = id("bounds");
	let steps = ["step_x", "step_y", "step_size"].map(id);
	let bound = ["bound_x", "bound_y", "bound_width", "bound_height"].map(id);
	let half = |word: &mut u32, high: bool, value: u32| {
		let shift = if high { 16 } else { 0 };
		*word = (*word & !(0xFFFF << shift)) | ((value & 0xFFFF) << shift);
	};
	let mut unread = None;
	let mut at = 0;
	while at + 2 < values.len() {
		let (name, tag) = (values[at], values[at + 1]);
		let len = if tag & 0xFF == 0x2A { 4 } else { 3 };
		if own.contains(&name) {
			if tag & 0xFF == 0x2A && at + 3 < values.len() {
				let number = f32::from_bits((values[at + 2] << 16) | (values[at + 3] & 0xFFFF));
				if let Some(step) = steps.iter().position(|id| *id == Some(name)) {
					block[2 + step] = number.to_bits();
				} else if let Some(n) = bound.iter().position(|id| *id == Some(name)) {
					let value = if n < 2 { number as i32 as u32 } else { number as u32 };
					half(&mut block[n / 2], n % 2 == 1, value);
				}
			} else if Some(name) == bounds && tag & 0xFF == 0x06 {
				let id = ((tag & 0xFF00) << 8) | values[at + 2];
				let text = String::from(unsafe { StringRef::from_id(raw_types::strings::StringId(id)) });
				match pixel_bounds(&text) {
					Some([x, y, width, height]) => {
						half(&mut block[0], false, x as u32);
						half(&mut block[0], true, y as u32);
						half(&mut block[1], false, width as u32);
						half(&mut block[1], true, height as u32);
					}
					None => unread = Some(text)
				}
			}
		}
		at += len;
	}
	unread
}

fn pixel_bounds(text: &str) -> Option<[i32; 4]> {
	let text = text.trim();
	let number = |n: &str| n.trim().parse::<i32>().ok();
	if !text.contains(',') {
		let (width, height) = text.split_once(['x', 'X'])?;
		return Some([0, 0, number(width)?, number(height)?]);
	}
	let (from, to) = text.split_once(" to ")?;
	let pair = |p: &str| p.split_once(',').and_then(|(x, y)| Some((number(x)?, number(y)?)));
	let ((x1, y1), (x2, y2)) = (pair(from)?, pair(to)?);
	Some([x1 - 1, y1 - 1, x2 - x1 + 1, y2 - y1 + 1])
}

#[cfg(test)]
mod tests {
	use super::pixel_bounds;

	#[test]
	fn bounds_in_pixels_are_read_and_tiles_are_not() {
		assert_eq!(pixel_bounds("1,1 to 32,32"), Some([0, 0, 32, 32]));
		assert_eq!(pixel_bounds("5,9 to 20,24"), Some([4, 8, 16, 16]));
		assert_eq!(pixel_bounds("64x48"), Some([0, 0, 64, 48]));
		assert_eq!(pixel_bounds("1:1,1:1 to 2:32,2:32"), None);
	}
}
