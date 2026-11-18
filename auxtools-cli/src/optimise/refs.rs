//! Add strings that keep `ref()` fast.
//!
//! DreamDaemon finds a string among all the world's strings through a tree
//! ordered by text, which it does not balance as strings are added. Things
//! made one after another have refs that sort one after another, so when a
//! world keeps them (a weak reference, `render_target = ref(src)`) the tree
//! grows one long branch, and about every thousandth kept ref stalls the
//! server for a time in proportion to the number of strings in the world.
//!
//! Evenly spaced refs put in the string table, in an order that keeps them
//! balanced, leave no run of refs longer than the spacing. They refer to
//! nothing and nothing uses them.

use dmasm::dmb::Dmb;

use super::Strings;

/// How far apart the strings are, in refs.
pub const SPACING: u32 = 256;

/// The kinds of thing whose refs worlds keep: the kind's part of a ref,
/// and how many things of the kind the default covers (a power of two).
const KINDS: [(&str, u32); 6] = [
	("1", 0x40_0000),  // turfs
	("2", 0x20_0000),  // objs
	("3", 0x8_0000),   // mobs
	("d", 0x4_0000),   // images and mutable appearances
	("f", 0x10_0000),  // lists
	("21", 0x80_0000)  // datums
];

/// How many strings the default adds.
pub const DEFAULT: u32 = {
	let mut total = 0;
	let mut kind = 0;
	while kind < KINDS.len() {
		total += KINDS[kind].1 / SPACING;
		kind += 1;
	}
	total
};

#[derive(Debug, Default)]
pub struct Report {
	/// Strings added, and those of them the world already had.
	pub added: usize,
	pub present: usize,
	/// How many were asked for.
	pub wanted: u32,
	/// The world's 16-bit string ids ran out before all were added.
	pub full: bool
}

/// The text of a ref: `[0x`, the kind, then which thing of the kind in six
/// hex digits.
fn reference(kind: &str, index: u32) -> Vec<u8> {
	format!("[0x{}{:06x}]", kind, index).into_bytes()
}

/// The refs to add for `wanted` strings in all: every `SPACING`th of each
/// kind, level by level (the middle one, then the quarters, and so on) so
/// that they do not arrive in order.
fn seeds(wanted: u32) -> Vec<Vec<u8>> {
	let mut seeds = Vec::new();
	for (kind, covered) in KINDS {
		// Scale what the default covers, keeping it a power of two
		let scaled = u64::from(covered / SPACING) * u64::from(wanted) / u64::from(DEFAULT);
		if scaled == 0 {
			continue;
		}
		let count = 1u32 << scaled.ilog2().min(24 - SPACING.ilog2());
		seeds.push(reference(kind, 0));
		let mut stride = count;
		while stride >= 2 {
			for index in (stride / 2..count).step_by(stride as usize) {
				seeds.push(reference(kind, index * SPACING));
			}
			stride /= 2;
		}
	}
	seeds
}

pub fn run(dmb: &mut Dmb, wanted: u32) -> Report {
	let mut report = Report { wanted, ..Report::default() };
	let mut strings = Strings::new(dmb);
	for seed in seeds(wanted) {
		if strings.get(&seed).is_some() {
			report.present += 1;
		} else if strings.intern(dmb, &seed).is_some() {
			report.added += 1;
		} else {
			report.full = true;
			break;
		}
	}
	report
}

#[cfg(test)]
mod tests {
	use dmasm::dmb::Dmb;

	use super::{run, seeds, DEFAULT};

	fn texts(seeds: &[Vec<u8>]) -> Vec<&str> {
		seeds.iter().map(|seed| std::str::from_utf8(seed).unwrap()).collect()
	}

	#[test]
	fn seeds_are_evenly_spaced_refs_in_an_order_that_balances() {
		assert_eq!(DEFAULT, 64512);
		let all = seeds(DEFAULT);
		assert_eq!(all.len(), 64512);
		let all = texts(&all);
		// Turfs first: the first, the middle, the quarters
		assert_eq!(all[..6], [
			"[0x1000000]",
			"[0x1200000]",
			"[0x1100000]",
			"[0x1300000]",
			"[0x1080000]",
			"[0x1180000]"
		]);
		assert_eq!(all[16384..16386], ["[0x2000000]", "[0x2100000]"]);
		// Datums last, and their last level is the odd multiples of the spacing
		assert_eq!(all[64512 - 32768..64512 - 32766], ["[0x21000000]", "[0x21400000]"]);
		assert_eq!(all[64511], "[0x217fff00]");
		let mut sorted = all.clone();
		sorted.sort_unstable();
		sorted.dedup();
		assert_eq!(sorted.len(), all.len());
	}

	#[test]
	fn the_count_scales_what_each_kind_covers() {
		assert_eq!(seeds(DEFAULT * 2).len(), 2 * 64512);
		assert_eq!(seeds(DEFAULT / 2).len(), 64512 / 2);
		// Rounded down to a power of two for each kind
		assert_eq!(seeds(DEFAULT - 1).len(), 64512 / 2);
		assert_eq!(texts(&seeds(DEFAULT / 1024)), [
			"[0x1000000]",
			"[0x1000800]",
			"[0x1000400]",
			"[0x1000c00]",
			"[0x1000200]",
			"[0x1000600]",
			"[0x1000a00]",
			"[0x1000e00]",
			"[0x1000100]",
			"[0x1000300]",
			"[0x1000500]",
			"[0x1000700]",
			"[0x1000900]",
			"[0x1000b00]",
			"[0x1000d00]",
			"[0x1000f00]",
			"[0x2000000]",
			"[0x2000400]",
			"[0x2000200]",
			"[0x2000600]",
			"[0x2000100]",
			"[0x2000300]",
			"[0x2000500]",
			"[0x2000700]",
			"[0x3000000]",
			"[0x3000100]",
			"[0xd000000]",
			"[0xf000000]",
			"[0xf000200]",
			"[0xf000100]",
			"[0xf000300]",
			"[0x21000000]",
			"[0x21001000]",
			"[0x21000800]",
			"[0x21001800]",
			"[0x21000400]",
			"[0x21000c00]",
			"[0x21001400]",
			"[0x21001c00]",
			"[0x21000200]",
			"[0x21000600]",
			"[0x21000a00]",
			"[0x21000e00]",
			"[0x21001200]",
			"[0x21001600]",
			"[0x21001a00]",
			"[0x21001e00]",
			"[0x21000100]",
			"[0x21000300]",
			"[0x21000500]",
			"[0x21000700]",
			"[0x21000900]",
			"[0x21000b00]",
			"[0x21000d00]",
			"[0x21000f00]",
			"[0x21001100]",
			"[0x21001300]",
			"[0x21001500]",
			"[0x21001700]",
			"[0x21001900]",
			"[0x21001b00]",
			"[0x21001d00]",
			"[0x21001f00]"
		]);
		assert!(seeds(0).is_empty());
		// No kind has more things than six hex digits count
		assert_eq!(seeds(u32::MAX).len(), 6 * (1 << 16));
	}

	#[test]
	fn seeds_are_added_once_and_only_while_ids_last() {
		let mut dmb = Dmb {
			flags: Dmb::LARGE_IDS,
			..Dmb::default()
		};
		dmb.strings.resize(1100, b"x".to_vec());
		dmb.strings.push(b"[0x21000000]".to_vec());
		let report = run(&mut dmb, DEFAULT);
		assert_eq!((report.added, report.present, report.full), (64511, 1, false));
		// (one more: the id that means "none" is left empty)
		assert_eq!(dmb.strings.len(), 1100 + 64512 + 1);
		assert_eq!(dmb.strings[0xFFFF], b"");
		let again = run(&mut dmb, DEFAULT);
		assert_eq!((again.added, again.present, again.full), (0, 64512, false));

		let mut narrow = Dmb::default();
		narrow.strings.resize(0xFFFF - 100, b"x".to_vec());
		let report = run(&mut narrow, DEFAULT);
		assert_eq!((report.added, report.full), (100, true));
		assert_eq!(narrow.strings.len(), 0xFFFF);
	}
}
