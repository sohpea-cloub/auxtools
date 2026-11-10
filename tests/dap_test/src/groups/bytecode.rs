//! Bytecode: the disassembly of the code a frame runs, old code included

use serde_json::{json, Value};

use crate::{
	groups::reloading::{copy_of_the_world, edit},
	ok,
	world::World
};

/// An address as the launcher writes it
fn address(text: &Value) -> u64 {
	let text = text.as_str().unwrap_or_else(|| panic!("{} isn't an address", text));
	u64::from_str_radix(text.strip_prefix("0x").unwrap(), 16).unwrap()
}

fn hex(address: u64) -> String {
	format!("0x{:016X}", address)
}

/// The proc and which of its codes an address is in, as an address
const CODE: u64 = !0xFFFF;

/// `count` instructions from `shift` on from the one at `reference`
fn disassemble(world: &mut World, reference: u64, shift: i64, count: i64) -> Vec<Value> {
	let answer = world.request(
		"disassemble",
		json!({ "memoryReference": hex(reference), "instructionOffset": shift, "instructionCount": count, "resolveSymbols": true })
	);
	answer["instructions"].as_array().unwrap().clone()
}

/// Whether it's one of the code's, not a place before or after it
fn is_code(instruction: &Value) -> bool {
	instruction["presentationHint"] != "invalid"
}

/// The whole of the code an address is in, from its start
fn listing(world: &mut World, reference: u64) -> Vec<Value> {
	disassemble(world, reference & CODE, 0, 1000).into_iter().filter(is_code).collect()
}

fn texts(listing: &[Value]) -> Vec<String> {
	listing.iter().map(|i| i["instruction"].as_str().unwrap().to_owned()).collect()
}

/// The paused frame's instruction
fn frame_address(world: &mut World) -> u64 {
	let frame = world.top_frame();
	address(&frame["instructionPointerReference"])
}

pub fn run() {
	let mut world = World::start();
	let at = world.marks.clone();

	world.run_to("middle", "/leaf", "leaf_1");
	let frame = world.top_frame();
	let here = address(&frame["instructionPointerReference"]);
	let one = disassemble(&mut world, here, 0, 1);
	assert_eq!(one.len(), 1, "{:?}", one);
	assert_eq!(address(&one[0]["address"]), here);
	assert_eq!(one[0]["line"], at.line("leaf_1"));
	assert_eq!(one[0]["location"]["path"], frame["source"]["path"]);
	assert_eq!(one[0]["symbol"], "/leaf");
	ok("a paused frame's instruction reference disassembles to its instruction, on the frame's line and file");

	// Far more than leaf() has, on both sides
	let window = disassemble(&mut world, here, -100, 300);
	assert_eq!(window.len(), 300);
	let addresses: Vec<u64> = window.iter().map(|i| address(&i["address"])).collect();
	assert!(addresses.windows(2).all(|pair| pair[0] < pair[1]), "addresses out of order: {:?}", addresses);
	assert!(addresses.contains(&here));
	let code: Vec<usize> = (0..window.len()).filter(|&i| is_code(&window[i])).collect();
	let (first, last) = (code[0], *code.last().unwrap());
	assert!(first > 0 && last < window.len() - 1, "no room before and after leaf()");
	assert_eq!(code.len(), last - first + 1, "something that isn't code in the middle of leaf()");
	assert_eq!(address(&window[first]["address"]), here & CODE, "leaf()'s code doesn't start at offset 0");
	assert!(window[..first].iter().chain(&window[last + 1..]).all(|i| i["instruction"] == "" && i["instructionBytes"].is_null()));
	ok("a window that starts before the proc and ends after it has as many instructions as asked for, in order, the code's and nothing round them");

	// Each instruction's words take up the addresses to the next one
	for pair in window[first..=last + 1].windows(2) {
		let words = pair[0]["instructionBytes"].as_str().unwrap().split(' ').count() as u64;
		assert_eq!(address(&pair[0]["address"]) + words, address(&pair[1]["address"]), "{} and {}", pair[0], pair[1]);
	}
	ok("an instruction's address is its offset in words: the next is as many on as it has words");

	// What's around an instruction is the same, whichever of them a client
	// counts from
	let from_start = disassemble(&mut world, here & CODE, 0, 5);
	let from_third = disassemble(&mut world, address(&from_start[2]["address"]), -2, 5);
	assert_eq!(from_start, from_third);
	// A reference in the middle of an instruction is that instruction
	let wide = window[first..=last].iter().find(|i| i["instructionBytes"].as_str().unwrap().contains(' ')).unwrap();
	let inside = disassemble(&mut world, address(&wide["address"]) + 1, 0, 1);
	assert_eq!(&inside[0], wide);
	ok("instructions are counted from the one a reference is in, wherever in it");

	// leaf(x): `var/y = x + 1` and `return y * 2`. The opcodes' numbers
	// differ between BYOND versions; their names don't.
	let leaf = listing(&mut world, here);
	let texts = texts(&leaf);
	let lines: Vec<u64> = leaf.iter().filter_map(|i| i["line"].as_u64()).collect();
	assert!(texts[0].starts_with("DbgFile"), "{:?}", texts);
	assert!(texts.iter().any(|t| *t == format!("DbgLine {}", at.line("leaf_1"))), "{:?}", texts);
	assert!(texts.iter().any(|t| *t == format!("DbgLine {}", at.line("leaf_2"))), "{:?}", texts);
	assert!(texts.iter().any(|t| t.starts_with("Ret")), "{:?}", texts);
	assert_eq!(texts.last().unwrap(), "End", "{:?}", texts);
	assert!(lines.windows(2).all(|pair| pair[0] <= pair[1]) && lines.len() == leaf.len(), "{:?}", lines);
	ok("leaf()'s instructions: its file, a DbgLine for each of its lines, a return and the end, each on its line");
	world.resume_through("middle");

	world.end();
	old_code();
}

/// A frame on code a hot reload replaced disassembles to that code
fn old_code() {
	let dir = copy_of_the_world();
	let mut world = World::launch_in(dir, json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload: watching");

	world.run_to("middle", "/leaf", "leaf_1");
	let paused = frame_address(&mut world);
	let before = listing(&mut world, paused);
	edit(&mut world, "stepping.dm", "\tvar/y = x + 1 // @leaf_1\n", "\tvar/y = x + 1 // @leaf_1\n\tvar/extra = 5 // @leaf_extra\n\ty += extra\n");
	// Its last line is two further down in the new code
	let extra = format!("DbgLine {}", world.marks.line("leaf_2"));

	let frame = world.top_frame();
	assert_eq!(frame["name"], "/leaf (old code)");
	let here = address(&frame["instructionPointerReference"]);
	assert_ne!(here & 0xFFFF_0000, 0, "an old code's address is the proc's");
	let old = listing(&mut world, here);
	assert_eq!(old, before.iter().map(|i| {
		let mut i = i.clone();
		i["address"] = hex(address(&i["address"]) - (address(&before[0]["address"]) & CODE) + (here & CODE)).into();
		i["symbol"] = "/leaf (old code)".into();
		i["location"] = Value::Null;
		i
	}).collect::<Vec<_>>());
	assert!(!texts(&old).contains(&extra));
	let one = disassemble(&mut world, here, 0, 1);
	assert_eq!(one[0]["line"], frame["line"]);
	let now = listing(&mut world, here & 0xFFFF_FFFF_0000_0000);
	assert!(texts(&now).contains(&extra), "{:?}", texts(&now));
	ok("a frame on old code after a hot reload disassembles to the code it runs, and the proc's address to its new code");

	world.resume_through("middle");
	world.end();
}
