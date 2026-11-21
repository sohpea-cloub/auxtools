//! Faster string lookup for BYOND.
//!
//! BYOND keeps every string of a world in one binary search tree ordered by
//! `strcmp`, and the function that finds or adds a string walks it from the
//! root, comparing at every node. Nothing balances the tree as it grows:
//! strings added in order, such as the refs of things made one after another,
//! hang in one long branch, and a walk that gets 1,001 nodes deep sorts every
//! string of the world to build the tree again. In tgstation's init the walks
//! and the sorting were a sixth of the time, and afterwards each sort stalls
//! the world for as long as it has strings: half a second with 400,000.
//!
//! The detour on that function does two things. It answers for a string the
//! world already has, from a cache by the text's hash or else from its own
//! walk, and leaves only new strings to BYOND. And once BYOND has added a
//! string it rotates the new node up the tree while its priority, a hash of
//! its ID, is above its parent's. That makes the tree a treap: a branch is
//! as deep as the logarithm of the string count however the strings arrive,
//! so no walk gets near the depth that sets off the sort. BYOND's own walks,
//! when it removes a string, take the same short branches.

use std::{
	cell::RefCell,
	ffi::{c_char, c_int, c_void}
};

use retour::RawDetour;

use crate::raw_types::{funcs, strings::StringEntry};

extern "C" {
	// BYOND's function past the detour (funcs.cpp)
	static mut get_string_id_original: *const c_void;
	// What the detour jumps to, with BYOND's calling convention (funcs.cpp)
	fn get_string_id_hook();
	// BYOND's accessor: the entry, or a stand-in with another ID for an ID
	// that has no string
	fn get_string_entry_unchecked(id: u32) -> *mut StringEntry;
	fn strcmp(a: *const c_char, b: *const c_char) -> c_int;
}

/// What `string_tree_find` returns for a string BYOND has to look up
const NOT_FOUND: u32 = u32::MAX;

/// Slots of the cache
const CACHED: usize = 1 << 20;

/// The deepest walk made here. BYOND builds the tree again in a walk that
/// passes 1,000 nodes, so a deeper one is left to it.
const DEEPEST: usize = 1000;

struct Tree {
	/// The ID of a string by its text's hash, `NOT_FOUND` for none. An ID can
	/// have gone to another string since, so a hit is compared.
	cache: Box<[u32]>,
	/// The nodes of the last walk, from the root down
	path: Vec<*mut StringEntry>,
	/// The cache slot of the string the last walk didn't find, when `path`
	/// ends at the node it belongs under
	absent: Option<usize>
}

/// Only BYOND's thread makes strings
static mut TREE: Option<Tree> = None;

thread_local!(static DETOUR: RefCell<Option<RawDetour>> = const { RefCell::new(None) });

/// The entry of a string that exists
unsafe fn entry(id: u32) -> Option<*mut StringEntry> {
	let entry = unsafe { get_string_entry_unchecked(id) };
	(!entry.is_null() && unsafe { (*entry).this.0 == id && !(*entry).data.is_null() }).then_some(entry)
}

/// A node's priority in the treap. Its ID stays with it for as long as it is
/// in the tree.
unsafe fn priority(node: *mut StringEntry) -> u32 {
	let mixed = unsafe { (*node).this.0 }.wrapping_mul(0x9E37_79B1);
	mixed ^ (mixed >> 15)
}

/// The text's hash, and whether every byte of it is ASCII
unsafe fn hash(text: *const c_char) -> (u32, bool) {
	let mut hash = 0x811C_9DC5u32;
	let mut all = 0u8;
	let mut at = text as *const u8;
	loop {
		let byte = unsafe { *at };
		if byte == 0 {
			return (hash, all < 0x80);
		}
		all |= byte;
		hash = (hash ^ byte as u32).wrapping_mul(0x0100_0193);
		at = unsafe { at.add(1) };
	}
}

/// The ID of `text` if the world has the string, `NOT_FOUND` if BYOND's
/// function has to run: to add it, or because it changes the text first.
/// The arguments are that function's. Called by `get_string_id_hook`, which
/// frees the text of a string found, as BYOND does when it is given to keep.
#[no_mangle]
extern "C" fn string_tree_find(text: *const c_char, kind: u8, other: u8) -> u32 {
	unsafe {
		let Some(tree) = TREE.as_mut() else {
			return NOT_FOUND;
		};
		tree.path.clear();
		tree.absent = None;
		// Kind 1 is text BYOND converts to UTF-8 before looking it up, and a
		// kind with the top bit set is text it converts if it finds it isn't
		// UTF-8, which ASCII always is
		if text.is_null() || other != 0 || kind == 1 {
			return NOT_FOUND;
		}
		let (hash, ascii) = hash(text);
		if kind >= 0x80 && !ascii {
			return NOT_FOUND;
		}
		let slot = hash as usize & (CACHED - 1);
		let cached = tree.cache[slot];
		if cached != NOT_FOUND && entry(cached).is_some_and(|entry| strcmp(text, (*entry).data) == 0) {
			return cached;
		}

		// The root is the first string and stays the root
		let Some(mut node) = entry(0) else {
			return NOT_FOUND;
		};
		loop {
			let data = (*node).data;
			if data.is_null() || tree.path.len() == DEEPEST {
				tree.path.clear();
				return NOT_FOUND;
			}
			let order = strcmp(text, data);
			if order == 0 {
				let id = (*node).this.0;
				tree.cache[slot] = id;
				tree.path.clear();
				return id;
			}
			tree.path.push(node);
			let next = if order > 0 { (*node).right } else { (*node).left };
			if next.is_null() {
				break;
			}
			node = next;
		}
		tree.absent = Some(slot);
		NOT_FOUND
	}
}

/// Fill `path` with the nodes above `node`, from the root down. False if the
/// walk doesn't reach it.
unsafe fn walk_to(path: &mut Vec<*mut StringEntry>, node: *mut StringEntry) -> bool {
	unsafe {
		path.clear();
		let Some(mut at) = entry(0) else {
			return false;
		};
		while at != node {
			if path.len() == DEEPEST || (*at).data.is_null() {
				return false;
			}
			let order = strcmp((*node).data, (*at).data);
			if order == 0 {
				return false;
			}
			path.push(at);
			at = if order > 0 { (*at).right } else { (*at).left };
			if at.is_null() {
				return false;
			}
		}
		!path.is_empty()
	}
}

/// Rotate `node` above each parent with a lower priority. `path` is the
/// nodes above it from the root down; the root keeps its place.
unsafe fn raise(node: *mut StringEntry, path: &[*mut StringEntry]) {
	unsafe {
		let own = priority(node);
		let mut above = path.len();
		while above >= 2 {
			let parent = path[above - 1];
			if priority(parent) >= own {
				break;
			}
			if (*parent).left == node {
				(*parent).left = (*node).right;
				(*node).right = parent;
			} else {
				(*parent).right = (*node).left;
				(*node).left = parent;
			}
			let grandparent = path[above - 2];
			if (*grandparent).left == parent {
				(*grandparent).left = node;
			} else {
				(*grandparent).right = node;
			}
			above -= 1;
		}
	}
}

/// BYOND's function returned `id` for the string `string_tree_find` left to
/// it. If it added the string, the node finds its level in the treap.
#[no_mangle]
extern "C" fn string_tree_added(id: u32) {
	unsafe {
		let Some(tree) = TREE.as_mut() else {
			return;
		};
		let absent = tree.absent.take();
		let Some(node) = entry(id) else {
			return;
		};
		// BYOND adds a string as a leaf
		if !(*node).left.is_null() || !(*node).right.is_null() {
			return;
		}
		match absent {
			Some(slot) => {
				tree.cache[slot] = id;
				// Under the node the walk ended at, unless BYOND changed text
				if !tree.path.last().is_some_and(|&parent| (*parent).left == node || (*parent).right == node) {
					return;
				}
			}
			// A string whose text BYOND changed: it may have been there
			// already, at its level
			None => {
				if !walk_to(&mut tree.path, node) {
					return;
				}
			}
		}
		raise(node, &tree.path);
	}
}

/// Detour BYOND's string lookup
pub fn init() {
	unsafe {
		let target = funcs::get_string_id_byond;
		if target.is_null() || TREE.is_some() {
			return;
		}
		let Ok(detour) = RawDetour::new(target as *const (), get_string_id_hook as *const ()) else {
			return;
		};
		get_string_id_original = detour.trampoline() as *const () as *const c_void;
		TREE = Some(Tree {
			cache: vec![NOT_FOUND; CACHED].into_boxed_slice(),
			path: Vec::with_capacity(DEEPEST),
			absent: None
		});
		if detour.enable().is_ok() {
			DETOUR.with(|cell| *cell.borrow_mut() = Some(detour));
		} else {
			TREE = None;
		}
	}
}

pub fn shutdown() {
	DETOUR.with(|cell| {
		if let Some(detour) = cell.borrow_mut().take() {
			let _ = unsafe { detour.disable() };
		}
	});
	unsafe {
		TREE = None;
	}
}
