use std::{
	collections::{HashMap, HashSet},
	sync::OnceLock
};

use crate::registry;

const LEFT: usize = 0x0;
const RIGHT: usize = 0x4;
const PARENT: usize = 0x8;
const LARGER: usize = 0xC;
const SMALLER: usize = 0x10;
const ID: usize = 0x14;
const COLOR: usize = 0x18;
const ICON: usize = 0x30;
const OVERLAYS: usize = 0x38;
const UNDERLAYS: usize = 0x3C;
const UNUSED_ID: usize = 0xFFFF;
const RED: u16 = 0;
const BLACK: u16 = 1;
const FREE: u16 = 0xFFFF;

const TREE: usize = 0x14;
const TABLE: usize = 0x40;
const CAPACITY: usize = 0x44;

const POINTERS_515: usize = 0x70;
const POINTERS_516: usize = 0x74;
const ICON_W_BUILD: u32 = 1648;
const NEWEST_CHECKED_BUILD: u32 = 1689;
const RESOURCE_TAG: u8 = 0x0C;

#[cfg(windows)]
type Compare = extern "thiscall" fn(*const u8, *const u8) -> i32;
#[cfg(windows)]
type TreeFn = extern "thiscall" fn(*mut u8, *mut u8) -> *mut u8;
#[cfg(windows)]
type Bounds = extern "thiscall" fn(*mut u8);
#[cfg(unix)]
type Compare = extern "C" fn(*const u8, *const u8) -> i32;
#[cfg(unix)]
type TreeFn = extern "C" fn(*mut u8, *mut u8) -> *mut u8;
#[cfg(unix)]
type Bounds = extern "C" fn(*mut u8);

struct Tables {
	manager: usize,
	connection_count: usize,
	connections: usize,
	send: usize,
	compare: Compare,
	unlink: TreeFn,
	rebalance: TreeFn,
	bounds: Bounds,
	pointers: usize
}

static TABLES: OnceLock<Result<Tables, String>> = OnceLock::new();

fn find() -> Result<Tables, String> {
	let build = crate::byond_build()?;
	if build > NEWEST_CHECKED_BUILD {
		return Err(format!(
			"build {build} is newer than the latest validated resource-reload build ({NEWEST_CHECKED_BUILD})"
		));
	}
	let tables = unsafe {
		Tables {
			manager: registry::find!("appearance_manager")?,
			connection_count: registry::find!("connection_count")?,
			connections: registry::find!("connections")?,
			send: registry::find!("appearance_send")?,
			compare: std::mem::transmute::<usize, Compare>(registry::find!("appearance_compare")?),
			unlink: std::mem::transmute::<usize, TreeFn>(registry::find!("appearance_tree_unlink")?),
			rebalance: std::mem::transmute::<usize, TreeFn>(registry::find!("appearance_tree_rebalance")?),
			bounds: std::mem::transmute::<usize, Bounds>(registry::find!("appearance_update_bounds")?),
			pointers: if build >= ICON_W_BUILD { POINTERS_516 } else { POINTERS_515 }
		}
	};
	if tables.unlink as usize == tables.rebalance as usize {
		return Err("Could not distinguish appearance-tree unlink and rebalance functions".to_owned());
	}
	Ok(tables)
}

fn tables() -> Result<&'static Tables, String> {
	TABLES
		.get_or_init(find)
		.as_ref()
		.map_err(|e| format!("resource reload unavailable on this BYOND build: {e}"))
}

pub fn check() -> Result<(), String> {
	let tables = tables()?;
	unsafe { Appearances::new(tables)?.check().map_err(unexpected) }
}

fn unexpected(why: String) -> String {
	format!("BYOND's appearance tree isn't laid out as expected ({why}); the resource reload changed nothing")
}

unsafe fn word(at: *mut u8, offset: usize) -> *mut *mut u8 {
	at.add(offset) as *mut *mut u8
}
unsafe fn link(node: *mut u8, offset: usize) -> *mut u8 {
	*word(node, offset)
}
unsafe fn set_link(node: *mut u8, offset: usize, to: *mut u8) {
	*word(node, offset) = to;
}
unsafe fn color(node: *mut u8) -> u16 {
	if node.is_null() {
		BLACK
	} else {
		*(node.add(COLOR) as *const u16)
	}
}
unsafe fn set_color(node: *mut u8, color: u16) {
	*(node.add(COLOR) as *mut u16) = color;
}

struct Appearances<'a> {
	tables: &'a Tables,
	tree: *mut *mut u8,
	table: *mut *mut u8,
	capacity: usize
}

impl<'a> Appearances<'a> {
	unsafe fn new(tables: &'a Tables) -> Result<Self, String> {
		let manager = *(tables.manager as *const *mut u8);
		if manager.is_null() {
			return Err("the appearance manager isn't set up yet".to_owned());
		}
		Ok(Appearances {
			tables,
			tree: manager.add(TREE) as *mut *mut u8,
			table: *(manager.add(TABLE) as *const *mut *mut u8),
			capacity: *(manager.add(CAPACITY) as *const u32) as usize
		})
	}

	unsafe fn live(&self) -> impl Iterator<Item = *mut u8> + '_ {
		(0..self.capacity).filter_map(move |id| {
			let node = *self.table.add(id);
			(!node.is_null() && color(node) != FREE).then_some(node)
		})
	}

	unsafe fn root(&self) -> *mut u8 {
		*self.tree
	}

	unsafe fn check(&self) -> Result<(), String> {
		let live = self.live().count();
		let bad = |why: &str| Err(why.to_owned());
		let root = self.root();
		if !root.is_null() && !link(root, PARENT).is_null() {
			return bad("the root has a parent");
		}
		if color(root) != BLACK {
			return bad("the root isn't black");
		}
		let mut black_height = None;
		let mut count = 0usize;
		let mut skipped_id = 0usize;
		let mut previous: *mut u8 = std::ptr::null_mut();
		let mut node = root;
		while !node.is_null() && !link(node, LEFT).is_null() {
			node = link(node, LEFT);
		}
		if node != *self.tree.add(1) {
			return bad("the smallest node isn't the header's");
		}
		while !node.is_null() {
			count += 1;
			if count > live + 1 {
				return bad("the tree has more nodes than there are appearances");
			}
			if link(node, SMALLER) != previous {
				return bad("the threaded list doesn't follow the tree");
			}
			let node_color = color(node);
			if node_color != RED && node_color != BLACK {
				return bad("a node is neither red nor black");
			}
			let mut leaf = false;
			for child in [LEFT, RIGHT] {
				let child = link(node, child);
				leaf |= child.is_null();
				if !child.is_null() && link(child, PARENT) != node {
					return bad("a child doesn't name its parent");
				}
				if node_color == RED && color(child) == RED {
					return bad("a red node has a red child");
				}
			}
			if leaf {
				let (mut blacks, mut depth, mut at) = (0, 0, node);
				while !at.is_null() && depth < 128 {
					blacks += usize::from(color(at) == BLACK);
					depth += 1;
					at = link(at, PARENT);
				}
				if *black_height.get_or_insert(blacks) != blacks || depth == 128 {
					return bad("the tree isn't balanced as a red-black tree is");
				}
			}
			if !previous.is_null() && self.compare(previous, node) > 0 {
				return bad("the nodes aren't in the comparator's order");
			}
			let id = *(node.add(ID) as *const u32) as usize;
			let listed = id < self.capacity && *self.table.add(id) == node;
			if !listed {
				let unlisted = id == UNUSED_ID && (id >= self.capacity || (*self.table.add(id)).is_null());
				if !unlisted || skipped_id > 0 {
					return bad("a node's id doesn't lead back to it");
				}
				skipped_id += 1;
			}
			previous = node;
			if !link(node, RIGHT).is_null() {
				node = link(node, RIGHT);
				while !link(node, LEFT).is_null() {
					node = link(node, LEFT);
				}
			} else {
				let mut child = node;
				node = link(node, PARENT);
				while !node.is_null() && link(node, RIGHT) == child {
					child = node;
					node = link(node, PARENT);
				}
			}
			if !node.is_null() && link(previous, LARGER) != node {
				return bad("the threaded list doesn't follow the tree");
			}
		}
		if previous != *self.tree.add(2) {
			return bad("the largest node isn't the header's");
		}
		if count - skipped_id != live {
			return bad("some appearances aren't in the tree");
		}
		Ok(())
	}

	unsafe fn compare(&self, a: *mut u8, b: *mut u8) -> i32 {
		(self.tables.compare)(a, b)
	}

	unsafe fn rotate(&self, x: *mut u8, left: bool) {
		let (toward, away) = if left { (LEFT, RIGHT) } else { (RIGHT, LEFT) };
		let y = link(x, away);
		set_link(x, away, link(y, toward));
		if !link(y, toward).is_null() {
			set_link(link(y, toward), PARENT, x);
		}
		let parent = link(x, PARENT);
		set_link(y, PARENT, parent);
		if parent.is_null() {
			*self.tree = y;
		} else if link(parent, LEFT) == x {
			set_link(parent, LEFT, y);
		} else {
			set_link(parent, RIGHT, y);
		}
		set_link(y, toward, x);
		set_link(x, PARENT, y);
	}

	unsafe fn insert(&self, node: *mut u8) {
		set_link(node, LEFT, std::ptr::null_mut());
		set_link(node, RIGHT, std::ptr::null_mut());
		set_color(node, RED);
		let root = self.root();
		if root.is_null() {
			set_link(node, PARENT, std::ptr::null_mut());
			set_link(node, LARGER, std::ptr::null_mut());
			set_link(node, SMALLER, std::ptr::null_mut());
			*self.tree = node;
			*self.tree.add(1) = node;
			*self.tree.add(2) = node;
			set_color(node, BLACK);
			return;
		}
		let mut parent = root;
		let mut order;
		loop {
			order = self.compare(node, parent);
			let child = link(parent, if order > 0 { RIGHT } else { LEFT });
			if child.is_null() {
				break;
			}
			parent = child;
		}
		set_link(node, PARENT, parent);
		if order > 0 {
			set_link(parent, RIGHT, node);
			let mut at = parent;
			while !link(at, LARGER).is_null() && self.compare(node, link(at, LARGER)) > 0 {
				at = link(at, LARGER);
			}
			let larger = link(at, LARGER);
			set_link(node, SMALLER, at);
			set_link(node, LARGER, larger);
			if larger.is_null() {
				*self.tree.add(2) = node;
			} else {
				set_link(larger, SMALLER, node);
			}
			set_link(at, LARGER, node);
		} else {
			set_link(parent, LEFT, node);
			let mut at = parent;
			while !link(at, SMALLER).is_null() && self.compare(link(at, SMALLER), node) > 0 {
				at = link(at, SMALLER);
			}
			let smaller = link(at, SMALLER);
			set_link(node, LARGER, at);
			set_link(node, SMALLER, smaller);
			if smaller.is_null() {
				*self.tree.add(1) = node;
			} else {
				set_link(smaller, LARGER, node);
			}
			set_link(at, SMALLER, node);
		}

		let mut z = node;
		while color(link(z, PARENT)) == RED {
			let parent = link(z, PARENT);
			let grand = link(parent, PARENT);
			let on_left = link(grand, LEFT) == parent;
			let uncle = link(grand, if on_left { RIGHT } else { LEFT });
			if color(uncle) == RED {
				set_color(parent, BLACK);
				set_color(uncle, BLACK);
				set_color(grand, RED);
				z = grand;
				continue;
			}
			if z == link(parent, if on_left { RIGHT } else { LEFT }) {
				z = parent;
				self.rotate(z, on_left);
			}
			let parent = link(z, PARENT);
			let grand = link(parent, PARENT);
			set_color(parent, BLACK);
			set_color(grand, RED);
			self.rotate(grand, !on_left);
		}
		set_color(self.root(), BLACK);
	}
}

pub struct Repointed {
	pub appearances: Vec<u32>,
	pub counts: HashMap<u32, u32>,
	pub pointers: usize,
	pub warnings: Vec<String>
}

pub fn repoint(mappings: &[(u32, u32)]) -> Result<Repointed, String> {
	let tables = tables()?;
	let mut new_of: HashMap<u32, u32> = HashMap::new();
	for &(old, new) in mappings {
		if old != new {
			new_of.entry(old).or_insert(new);
		}
	}
	unsafe {
		let appearances = Appearances::new(tables)?;
		appearances.check().map_err(unexpected)?;
		let pointer = |node: *mut u8, n: usize| node.add(tables.pointers + 8 * n);
		let points = |node: *mut u8, n: usize| -> Option<u32> {
			let value = pointer(node, n);
			if *value != RESOURCE_TAG {
				return None;
			}
			new_of.get(&*(value.add(4) as *const u32)).copied()
		};
		let changing: Vec<*mut u8> = appearances
			.live()
			.filter(|&node| new_of.contains_key(&*(node.add(ICON) as *const u32)) || (0..3).any(|n| points(node, n).is_some()))
			.collect();
		if changing.is_empty() {
			return Ok(Repointed {
				appearances: vec![],
				counts: HashMap::new(),
				pointers: 0,
				warnings: vec![]
			});
		}
		let shown_in = appearances.shown_in();

		let tree = appearances.tree as *mut u8;
		for &node in &changing {
			let unlinked = (tables.unlink)(tree, node);
			if color(unlinked) != RED {
				(tables.rebalance)(tree, unlinked);
			}
		}
		let mut counts: HashMap<u32, u32> = HashMap::new();
		let mut pointers = 0;
		for &node in &changing {
			let icon = node.add(ICON) as *mut u32;
			let mut to = None;
			if let Some(&new) = new_of.get(&*icon) {
				*icon = new;
				to = Some(new);
			}
			for n in 0..3 {
				if let Some(new) = points(node, n) {
					*(pointer(node, n).add(4) as *mut u32) = new;
					pointers += 1;
					to = to.or(Some(new));
				}
			}
			if let Some(to) = to {
				*counts.entry(to).or_default() += 1;
			}
		}
		for node in appearances.bounds_order(&changing, &shown_in) {
			(tables.bounds)(node);
		}
		let mut warnings = vec![];
		for &node in &changing {
			appearances.insert(node);
			if let Some(same) = appearances.equal_neighbour(node) {
				warnings.push(format!(
					"appearance {} now duplicates appearance {}; both IDs are retained with their existing client appearances",
					*(node.add(ID) as *const u32),
					*(same.add(ID) as *const u32)
				));
			}
		}
		if let Err(why) = appearances.check() {
			warnings.push(format!(
				"appearance-tree validation failed after reload ({why}); restart the world to avoid incorrect behavior"
			));
		}
		Ok(Repointed {
			appearances: changing.iter().map(|&node| *(node.add(ID) as *const u32)).collect(),
			counts,
			pointers,
			warnings
		})
	}
}

impl Appearances<'_> {
	unsafe fn shown_in(&self) -> HashMap<u32, Vec<*mut u8>> {
		let mut shown_in: HashMap<u32, Vec<*mut u8>> = HashMap::new();
		for node in self.live() {
			let mut shown: Vec<u32> = [OVERLAYS, UNDERLAYS]
				.iter()
				.flat_map(|&list| crate::runtime_tables::misc_words(*(node.add(list) as *const u32)).unwrap_or_default())
				.collect();
			shown.sort_unstable();
			shown.dedup();
			for id in shown {
				shown_in.entry(id).or_default().push(node);
			}
		}
		shown_in
	}

	unsafe fn bounds_order(&self, changed: &[*mut u8], shown_in: &HashMap<u32, Vec<*mut u8>>) -> Vec<*mut u8> {
		let id = |node: *mut u8| *(node.add(ID) as *const u32);
		let mut affected: Vec<*mut u8> = changed.to_vec();
		let mut seen: HashSet<*mut u8> = affected.iter().copied().collect();
		let mut at = 0;
		while at < affected.len() {
			for &parent in shown_in.get(&id(affected[at])).into_iter().flatten() {
				if seen.insert(parent) {
					affected.push(parent);
				}
			}
			at += 1;
		}
		let mut waiting: HashMap<*mut u8, usize> = affected.iter().map(|&node| (node, 0)).collect();
		for &node in &affected {
			for &parent in shown_in.get(&id(node)).into_iter().flatten() {
				*waiting.get_mut(&parent).unwrap() += 1;
			}
		}
		let mut ready: Vec<*mut u8> = affected.iter().copied().filter(|node| waiting[node] == 0).collect();
		let mut order = Vec::with_capacity(affected.len());
		while let Some(node) = ready.pop() {
			order.push(node);
			for &parent in shown_in.get(&id(node)).into_iter().flatten() {
				let count = waiting.get_mut(&parent).unwrap();
				*count -= 1;
				if *count == 0 {
					ready.push(parent);
				}
			}
		}
		if order.len() < affected.len() {
			let placed: HashSet<*mut u8> = order.iter().copied().collect();
			order.extend(affected.iter().copied().filter(|node| !placed.contains(node)));
		}
		order
	}

	unsafe fn equal_neighbour(&self, node: *mut u8) -> Option<*mut u8> {
		[link(node, SMALLER), link(node, LARGER)]
			.into_iter()
			.find(|&other| !other.is_null() && self.compare(node, other) == 0)
	}
}

pub fn resend(appearances: &[u32]) -> Result<usize, String> {
	let tables = tables()?;
	if appearances.is_empty() {
		return Ok(0);
	}
	unsafe {
		let count = *(tables.connection_count as *const u16) as u32;
		let connections = *(tables.connections as *const *const *mut u8);
		let mut reached = 0;
		for connection in 0..count {
			if connections.is_null() || (*connections.add(connection as usize)).is_null() {
				continue;
			}
			for &id in appearances {
				send(tables.send, connection, id);
			}
			reached += 1;
		}
		Ok(reached)
	}
}

#[cfg(windows)]
unsafe fn send(function: usize, connection: u32, appearance: u32) {
	let send: extern "cdecl" fn(u32, u32, i32) = std::mem::transmute(function);
	send(connection, appearance, 1);
}

#[cfg(unix)]
unsafe fn send(function: usize, connection: u32, appearance: u32) {
	std::arch::asm!(
		"mov edi, esp",
		"and esp, -16",
		"call {function}",
		"mov esp, edi",
		function = in(reg) function,
		out("edi") _,
		inout("eax") connection => _,
		inout("edx") appearance => _,
		inout("ecx") 1u32 => _,
		clobber_abi("C"),
	);
}
