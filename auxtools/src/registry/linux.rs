use byond_scan::{Anchor, Extract, OperandSelect, Recipe, SignatureTreatment, VersionRange};

use super::builds::{since, until, FIRST_515, FIRST_516, LINUX_FRAMELESS, TYPEPATHS, TYPEPATH_NONE_CHECK};

const SINCE_516: VersionRange = since(FIRST_516);
const SINCE_515_1644: VersionRange = since(LINUX_FRAMELESS);
const LATE_515: VersionRange = until(LINUX_FRAMELESS, FIRST_516);
const EARLY_515: VersionRange = until(FIRST_515, LINUX_FRAMELESS);
const EVERY_BUILD: VersionRange = since(FIRST_515);

const GLOBAL_VARS_SIG: &str = "89 1D ?? ?? ?? ?? 57 E8 ?? ?? ?? ?? 83 C4 10 85 FF 74 08 85 C0 0F 84 ?? ?? ?? ?? 83 EC 0C C1 E3 02 A3 ?? ?? ?? ?? 53 \
                               E8 ?? ?? ?? ?? 83 C4 10 85 DB 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ??";

const SUSPENDED_SIG: &str = "55 57 89 C7 56 53 83 EC ?? 8B 0D ?? ?? ?? ?? 80 48 04 04 8B A8 ?? 00 00 00 A1 ?? ?? ?? ?? 89 4C 24 ?? 8B 35 ?? ?? ?? ??";

const RUNTIME_SIG: &str =
	"55 89 E5 56 53 83 EC ?? 8B 15 ?? ?? ?? ?? 85 D2 0F 84 ?? ?? ?? ?? 0F B6 42 6F 3C 01 76 ?? 31 C9 84 C0 C7 42 10 ?? ?? ?? ?? 0F 94 C3";

const RUNTIME_SIG_515: &str = "55 89 E5 56 53 83 EC 30 8B 15 ?? ?? ?? ?? 85 D2 0F 84 ?? ?? ?? ?? 0F B6 42 6D";

const PROCDEF_ACCESSOR_SIG: &str = "8B 44 24 04 39 05 ?? ?? ?? ?? 76 ?? 6B C0 ?? 03 05 ?? ?? ?? ?? C3";
const STRING_ACCESSOR_SIG: &str = "56 53 83 EC 14 8B 44 24 20 39 05 ?? ?? ?? ?? 76 ?? 8B 15 ?? ?? ?? ?? 8B 04 82";

const ALIST_SIG: &str =
	"8B 54 24 04 8B 4C 24 08 80 FA 0F 74 ?? 31 C0 80 FA 55 75 ?? 3B 0D ?? ?? ?? ?? 73 ?? A1 ?? ?? ?? ?? 8B 04 88 85 C0 74 ?? 8B 00 C3";

const TYPE_MAP_ACCESSOR: &str = "8B 54 24 04 31 C0 39 15 ?? ?? ?? ?? 72 08 A1 ?? ?? ?? ?? 8D 04 D0 C3";

const TYPE_LOADER: &str = "69 FB 94 00 00 00 83 EC 0C 89 1D ?? ?? ?? ?? 57 E8 ?? ?? ?? ?? 83 C4 10 85 FF 74 08 85 C0 0F 84 ?? ?? ?? ?? 83 EC 0C 8D \
                           3C 9D 00 00 00 00 A3 ?? ?? ?? ?? 57 89 BD ?? ?? ?? ?? E8 ?? ?? ?? ?? 83 C4 10 85 FF 0F 95 85 ?? ?? ?? ?? 0F B6 8D ?? ?? \
                           ?? ?? 85 C0 75 08 84 C9 0F 85 ?? ?? ?? ?? 83 EC 0C 8D 3C DD 00 00 00 00 A3 ?? ?? ?? ?? 57 E8 ?? ?? ?? ?? 83 C4 10 85 FF \
                           74 08 85 C0 0F 84 ?? ?? ?? ?? 83 EC 0C FF B5 ?? ?? ?? ?? A3 ?? ?? ?? ?? E8 ?? ?? ?? ?? 83 C4 10 85 C0 75 ?? 80 BD ?? ?? \
                           ?? ?? 00 0F 85 ?? ?? ?? ?? A3 ?? ?? ?? ??";

const IN_LIST_LOADER: &str = "83 EC 0C 8D 1C 85 00 00 00 00 A3 ?? ?? ?? ?? 53 E8 ?? ?? ?? ?? 83 C4 10 85 DB 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? \
                              ?? ?? 31 DB EB ?? 83 EC 0C 56 E8 ?? ?? ?? ?? 83 C4 10 8B 15 ?? ?? ?? ?? 89 04 9A";

const MOB_LOADER: &str = "89 C7 83 C4 10 89 FB 83 EC 0C 89 3D ?? ?? ?? ?? C1 E3 04 53 E8 ?? ?? ?? ?? 83 C4 10 85 DB 74 08 85 C0 0F 84 ?? ?? ?? ?? \
                          A3 ?? ?? ?? ?? 31 DB 85 FF 74 ?? 89 D8 83 EC 08 C1 E0 04 03 05";

const INSTANCE_LOADER: &str = "6B D8 0C 83 EC 0C A3 ?? ?? ?? ?? 53 E8 ?? ?? ?? ?? 83 C4 10 85 DB 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ?? 31 DB \
                               EB ?? 6B C3 0C 03 05 ?? ?? ?? ?? 57 57 56 50 E8";

const TYPE_LOADER_EARLY: &str = "A3 ?? ?? ?? ?? 69 F0 ?? 00 00 00 89 34 24 E8 ?? ?? ?? ?? 85 F6 0F 85 ?? ?? ?? ?? 8B 35 ?? ?? ?? ?? A3 ?? ?? ?? ?? \
                                 C1 E6 02 89 34 24 E8 ?? ?? ?? ?? 85 C0 0F 84 ?? ?? ?? ?? 8B 35 ?? ?? ?? ?? A3 ?? ?? ?? ?? C1 E6 03 89 34 24 E8 ?? \
                                 ?? ?? ?? 85 C0 0F 84 ?? ?? ?? ?? 8B 35 ?? ?? ?? ?? A3 ?? ?? ?? ?? C1 E6 02 89 34 24 E8 ?? ?? ?? ?? 85 C0 0F 84 ?? \
                                 ?? ?? ?? 8B 15 ?? ?? ?? ?? A3 ?? ?? ?? ??";

const VAR_LOADER_EARLY: &str =
	"A3 ?? ?? ?? ?? 8D 34 C5 00 00 00 00 89 34 24 E8 ?? ?? ?? ?? 85 F6 0F 85 ?? ?? ?? ?? 8B 35 ?? ?? ?? ?? A3 ?? ?? ?? ?? C1 E6 02";
const IN_LIST_LOADER_EARLY: &str =
	"A3 ?? ?? ?? ?? C1 E0 02 31 F6 89 04 24 E8 ?? ?? ?? ?? A3 ?? ?? ?? ?? EB ?? 89 1C 24 E8 ?? ?? ?? ?? 8B 15 ?? ?? ?? ?? 89 04 B2";
const MOB_LOADER_EARLY: &str = "A3 ?? ?? ?? ?? 89 C6 C1 E6 04 89 34 24 E8 ?? ?? ?? ?? 85 F6 0F 85 ?? ?? ?? ?? 8B 3D ?? ?? ?? ?? A3 ?? ?? ?? ??";

const INSTANCE_LOADER_EARLY: &str = "A3 ?? ?? ?? ?? 6B C0 0C 31 FF 31 F6 89 04 24 E8 ?? ?? ?? ?? A3 ?? ?? ?? ?? EB ?? A1 ?? ?? ?? ?? 83 C6 01";

const SINCE_515_1647: VersionRange = since(1647);

const APPEARANCE_SEND_EARLY: &str =
	"55 0F B7 C0 89 E5 57 56 53 89 D3 81 EC 2C 09 00 00 89 04 24 89 8D 74 F7 FF FF 89 85 4C F7 FF FF E8 ?? ?? ?? ?? 85 C0 89";

const APPEARANCE_SEND: &str =
	"55 0F B7 C0 89 E5 57 56 89 CE 53 89 D3 81 EC F8 04 00 00 89 85 30 FB FF FF 50 E8 ?? ?? ?? ?? 83 C4 10 89 85 24 FB FF FF";

const APPEARANCE_COMPARE_EARLY: &str =
	"55 89 E5 83 EC 28 89 75 F8 8B 75 08 89 5D F4 8B 5D 0C 89 7D FC 8B 46 1C 2B 43 1C 75 08 8B 46 30 2B 43 30 74 13 8B 5D F4";

const APPEARANCE_COMPARE: &str =
	"55 57 56 53 83 EC 1C 8B 74 24 30 8B 5C 24 34 8B 46 1C 2B 43 1C 75 42 8B 46 30 2B 43 30 75 3A 8B 46 34 2B 43 34 75 32 0F";

const APPEARANCE_BOUNDS_EARLY: &str =
	"55 89 E5 83 EC 38 89 5D F4 8B 5D 08 8D 45 D8 89 04 24 89 75 F8 89 7D FC C7 44 24 0C 00 00 00 00 C7 44 24 08 FF FF 00 00";

const APPEARANCE_BOUNDS: &str =
	"55 57 56 53 83 EC 1C 8B 5C 24 30 89 E0 6A 00 68 FF FF 00 00 53 50 E8 ?? ?? ?? ?? 8B 4C 24 0C 89 8B ?? 00 00 00 8B 54 24";

const APPEARANCE_RELEASE_EARLY: &str =
	"C7 04 24 ?? ?? ?? ?? E8 ?? ?? ?? ?? 8B 95 F4 FE FF FF 89 34 24 89 54 24 04 E8 ?? ?? ?? ?? 89 F8 8B 5D F4 8B 75 F8 8B 7D";

const APPEARANCE_RELEASE: &str =
	"68 ?? ?? ?? ?? E8 ?? ?? ?? ?? 58 5A 53 57 E8 ?? ?? ?? ?? 83 C4 10 C7 85 F4 FE FF FF FF FF 00 00 8B 85 F4 FE FF FF EB 28";

const DISPATCH_MASK_EARLY_515: &str = "0F B7 57 14 8B 47 10 0F B7 DA 8B 0C 98 81 F9 ?? 01 00 00 77 ?? FF 24 8D ?? ?? ?? ??";
const DISPATCH_MASK_515: &str = "0F B7 46 14 8B 56 10 8D 3C 85 00 00 00 00 66 89 85 ?? ?? ?? ?? 89 85 ?? ?? ?? ?? 8D 04 3A 8B 18 81 FB ?? 01 00 00 \
                                 0F 87 ?? ?? ?? ?? FF 24 9D ?? ?? ?? ??";
const DISPATCH_MASK: &str = "0F B7 46 ?? 8B 56 ?? 66 89 85 ?? ?? ?? ?? 89 85 ?? ?? ?? ?? C1 E0 02 89 85 ?? ?? ?? ?? 01 D0 8B 18 81 FB ?? 01 00 00 \
                             0F 87 ?? ?? ?? ?? FF 24 9D ?? ?? ?? ??";

pub(super) const RECIPES: &[Recipe] = &[
	Recipe {
		name: "get_proc_array_entry",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, PROCDEF_ACCESSOR_SIG),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_string_table_entry",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, STRING_ACCESSOR_SIG),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_string_id",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 57 56 89 CE 53 89 D3 83 EC ?? 8B 54 24 ?? 88 54 24 ?? 85 C0 0F 84 ?? ?? ?? ?? 89 44 24 ?? 84 DB 0F 88 ?? ?? ?? ?? 88 5C 24 ?? 80 FB \
			 01 0F 84 ?? ?? ?? ?? 89 F0 84 C0"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "call_proc_by_id",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 31 C0 57 56 53 81 EC ?? ?? ?? ?? F3 0F 7E 8C 24 ?? ?? ?? ?? 8B 8C 24 ?? ?? ?? ?? 8A 84 24 ?? ?? ?? ?? 8B 9C 24 ?? ?? ?? ?? F3 0F 7E \
			 84 24"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "call_datum_proc_by_name",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 31 C9 89 E5 57 56 8D 45 ?? 53 83 EC ?? F3 0F 7E 45 0C 8B 5D 14 C6 45 ?? 00 8B 55 18 8B 75 08 6A 00 66 0F D6 45 ?? 0F B6 FB 66 0F 7E \
			 45 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_variable",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC ?? F3 0F 7E 4D 0C 8B 75 08 8B 5D 14 66 0F 6F C1 66 0F 7E CA 66 0F 73 D0 20 0F B6 C2 66 0F 7E 45 ?? 3C 58 77 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_variable",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC ?? F3 0F 7E 55 08 8B 7D 10 66 0F 6F C2 66 0F 7E D2 66 0F 73 D0 20 89 D3 66 0F 7E C6 F3 0F 7E 45 14 89 75 ?? 66 \
			 0F 6F C8"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "inc_ref_count",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 56 53 83 EC ?? F3 0F 7E 4D 08 66 0F 6F C1 66 0F 7E C9 66 0F 73 D0 20 66 0F 7E C0 80 F9 4A 77 ?? 0F B6 C9 FF 24 8D ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "dec_ref_count",
		versions: SINCE_516,
		anchor: Anchor::Export(c"ByondValue_DecRef"),
		hops: &[4],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_assoc_element",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC ?? F3 0F 7E 4D 0C 8B 7D 08 66 0F 6F C1 66 0F 7E C8 66 0F 7E 4D ?? 66 0F 73 D0 20 66 0F 7E 45 ?? 3C 3C 0F 84 ?? \
			 ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_assoc_element",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 83 C4 20 E9 ?? ?? ?? ?? ?? ?? ?? ?? 31 C9 EB ?? D9 43 04 BA 01 00 00 00"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "create_list",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 57 56 53 83 EC ?? A1 ?? ?? ?? ?? 8B 7C 24 ?? 85 C0 0F 84 ?? ?? ?? ?? 8B 15 ?? ?? ?? ?? 83 E8 01 A3 ?? ?? ?? ?? 8B 1C 82 81 FB FF FF \
			 00 00"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "append_to_list",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"56 66 0F 6E CA 66 0F 6E C0 53 66 0F 62 C1 66 0F 7E C6 66 0F 6F D0 66 0F 73 D0 20 89 F1 83 EC ?? 66 0F 7E C0 F3 0F 7E 4C 24 ?? 66 0F 6F \
			 C1"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "remove_from_list",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 83 85 ?? ?? ?? ?? 01 8B 8D ?? ?? ?? ?? 83 C4 10 83 85 ?? ?? ?? ?? 08 39"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_length",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC ?? F3 0F 7E 4D 0C 8B 5D 08 66 0F 7E CA 83 EA 0C 80 FA 4B 77 ?? 66 0F 6F C1 0F B6 D2 66 0F 73 D0 20 66 0F 7E C0 \
			 FF 24 95 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "value_is_list",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 56 53 83 EC ?? F3 0F 7E 4D 08 8B 75 10 66 0F 7E CA 80 FA 55 0F 87 ?? ?? ?? ?? 80 FA 3B 76 ?? 8D 4A C4 B8 01 00 00 00 D3 E0 89 \
			 C1"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_misc_by_id",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 89 85 ?? ?? ?? ?? 83 C4 10 31 DB 85 C0 74 03 0F B7 18 8B 85 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "to_string",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC ?? 0F B6 45 08 3C 45 77 ?? 3C 27 77 ?? 8D 50 FF 80 FA 26 77 ?? FF 24 85 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "runtime",
		versions: SINCE_516,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, RUNTIME_SIG),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "current_execution_context",
		versions: SINCE_516,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, RUNTIME_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "variable_names",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, GLOBAL_VARS_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(2))
	},
	Recipe {
		name: "suspended_procs",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, SUSPENDED_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "suspended_procs_buffer",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, SUSPENDED_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(2))
	},
	Recipe {
		name: "alist_table_count",
		versions: SINCE_516,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, ALIST_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "alist_table_ptr",
		versions: SINCE_516,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, ALIST_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "get_variable",
		versions: LATE_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC 5C F3 0F 7E 4D 0C 8B 75 08 66 0F 6F C1 66 0F 7E C8"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_variable",
		versions: LATE_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC 7C F3 0F 7E 55 08 8B 75 10 66 0F 6F C2 66 0F 7E D0"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_assoc_element",
		versions: LATE_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC 3C F3 0F 7E 45 0C 8B 7D 08 0F 29 45 B8 66 0F 7E C0"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "append_to_list",
		versions: LATE_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"56 66 0F 6E CA 66 0F 6E C0 53 66 0F 62 C1 66 0F 6F C8 66 0F 7E C6 66 0F 73 D1 20"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "remove_from_list",
		versions: LATE_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 66 0F 6E CA 66 0F 6E C0 57 66 0F 62 C1 56 66 0F 6F C8 66 0F 7E C7 53"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_length",
		versions: LATE_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC 2C F3 0F 7E 4D 0C 8B 5D 08 66 0F 7E C8 3C 39 77 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "value_is_list",
		versions: LATE_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 56 53 83 EC 20 F3 0F 7E 4D 08 8B 75 10 66 0F 7E CA 80 FA 54 0F 87 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "runtime",
		versions: LATE_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, RUNTIME_SIG_515),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "current_execution_context",
		versions: LATE_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, RUNTIME_SIG_515),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "to_string",
		versions: LATE_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC 3C 0F B6 45 08 3C 23 77 ?? 84 C0 74 ?? 3C 23 77 ?? FF 24 85 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "dec_ref_count",
		versions: LATE_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 56 53 83 EC 20 F3 0F 7E 4D 08 66 0F 6F C1 66 0F 7E C9 66 0F 73 D0 20 66 0F 7E C3 80 F9 4A"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_proc_array_entry",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 0F B7 F6 89 C7 89 B5 ?? ?? ?? ?? 89 34 24 E8 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_string_table_entry",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 83 EC 18 8B 45 ?? 39 05 ?? ?? ?? ?? 76 ?? 8B 15 ?? ?? ?? ?? 8B 04 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_string_id",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 89 CE 53 89 D3 83 EC 5C 8B 55 ?? 85 C0 88 55 ?? 0F 84 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "call_proc_by_id",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 81 EC ?? ?? 00 00 0F B6 45 14 81 7D ?? ?? ?? ?? 00 8B 55 0C 8B 4D 10"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "call_datum_proc_by_name",
		versions: EARLY_515,
		anchor: Anchor::Export(c"Byond_CallProcByStrId"),
		hops: &[11, 2],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_variable",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 81 EC ?? ?? 00 00 8B 55 0C 89 5D F4 8B 5D 10 89 75 F8 8B 75 08 89 7D FC 8B 7D 14"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_variable",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 81 EC ?? ?? 00 00 89 5D F4 8B 5D 08 8B 45 0C 89 75 F8 8B 55 18 89 7D FC 8B 75 10"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "inc_ref_count",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 8B 43 ?? 80 48 ?? 04 8B 5D ?? 8B 75 ?? 8B 7D ?? 89 EC 5D"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "dec_ref_count",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? C7 06 00 00 00 00 C7 46 ?? 00 00 00 00 A1 ?? ?? ?? ?? 0F B7 50 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_assoc_element",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 83 EC 58 89 5D F4 8B 5D 0C 89 75 F8 8B 75 10 89 7D FC 80 FB 3C 89 DF 0F 84 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_assoc_element",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 83 EC 68 89 5D F4 8B 5D 08 89 75 F8 8B 75 10 8B 45 14 89 7D FC 8B 7D 0C 80 FB 3C"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "create_list",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC 2C A1 ?? ?? ?? ?? 8B 75 ?? 85 C0 0F 84 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "append_to_list",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 83 EC 38 3C 54 89 5D ?? 8B 5D ?? 89 75 ?? 8B 75 ?? 89 7D ?? 76 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "remove_from_list",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 83 EC 48 3C 54 89 5D ?? 89 C3 89 75 ?? 8B 75 ?? 89 7D ?? 8B 7D ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_length",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 57 56 53 83 EC 6C 8B 45 ?? 8B 5D ?? 3C 54 76 ?? 31 F6 8D 65 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "value_is_list",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 83 EC 48 89 5D F4 8B 5D 08 89 75 F8 ?? ?? ?? ?? 89 7D FC 8B 7D 0C 89 DE 8D 46 F1"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_misc_by_id",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 0F B7 55 ?? 03 1F 0F B7 4B ?? 89 8D ?? ?? ?? ?? 0F B7 5B ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "to_string",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 83 ?? ?? 89 5D F4 8D ?? ?? 89 75 F8 89 7D FC 80 ?? ?? ?? ?? ?? B8"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "runtime",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 31 C0 8D B4 26 00 00 00 00 8B 5D ?? 8B 75 ?? 8B 7D ?? 89 EC"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "current_execution_context",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"A1 ?? ?? ?? ?? C7 44 24 ?? 00 00 00 00 C7 44 24 ?? 00 00 00 00 89 74 24"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "variable_names",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"A1 ?? ?? ?? ?? 8B 13 8B 39 8B 75 ?? 8B 14 ?? 89 7D ?? 8B 3C ?? 83 EE 02"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "suspended_procs",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"E8 ?? ?? ?? ?? A1 ?? ?? ?? ?? 3B 05 ?? ?? ?? ?? 0F 83 ?? ?? ?? ?? 8B 15 ?? ?? ?? ?? 3B 34 82 0F 85 ?? ?? ?? ?? 8B 8E 88 00 00 00"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "suspended_procs_buffer",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"E8 ?? ?? ?? ?? A1 ?? ?? ?? ?? 3B 05 ?? ?? ?? ?? 0F 83 ?? ?? ?? ?? 8B 15 ?? ?? ?? ?? 3B 34 82 0F 85 ?? ?? ?? ?? 8B 8E 88 00 00 00"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(2))
	},
	Recipe {
		name: "procdef_add",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"57 56 53 83 EC 10 A1 ?? ?? ?? ?? 8B 7C 24 20 85 C0 0F 84 ?? ?? ?? ?? 31 F6 31 DB EB ?? 8D 76 00 A1 ?? ?? ?? ?? 83 C3 01"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "misc_intern_copy",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"57 56 53 8B 1D ?? ?? ?? ?? 8B 7C 24 10 8B 74 24 14 85 DB 74 ?? 8B 43 08 A3 ?? ?? ?? ?? 66 89 33 0F B7 F6"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "obj_type_count",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_MAP_ACCESSOR),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "proc_name_maps",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_MAP_ACCESSOR),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "global_values",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, GLOBAL_VARS_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "global_vars",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"66 D1 EA 0F B7 D2 E8 ?? ?? ?? ?? 59 58 0F B7 07 50 53 E8 ?? ?? ?? ?? 89 C7 89 1C 24 A3 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "AllocShared",
		versions: EVERY_BUILD,
		anchor: Anchor::Export(c"_Z11AllocSharedm"),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "FreeShared",
		versions: EVERY_BUILD,
		anchor: Anchor::Export(c"_Z10FreeSharedPv"),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "CopyShared",
		versions: EVERY_BUILD,
		anchor: Anchor::Export(c"_Z10CopySharedPv"),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "type_preorder",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(4))
	},
	Recipe {
		name: "type_preorder",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(4))
	},
	Recipe {
		name: "type_appearance_cache",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(6))
	},
	Recipe {
		name: "type_appearance_cache",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(8))
	},
	Recipe {
		name: "in_list_count",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, IN_LIST_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "in_list_procs",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, IN_LIST_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(2))
	},
	Recipe {
		name: "world_name_map",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"0F B6 45 ?? 89 7D 0C C7 45 08 ?? ?? ?? ?? 89 45 14 89 F0 0F B6 F0 89 75 10 8D 65 F4 5B 5E 5F 5D E9"
		),
		hops: &[],
		extract: Extract::Offset(10)
	},
	Recipe {
		name: "mob_count",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, MOB_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "mobs",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, MOB_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "instance_count",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, INSTANCE_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "instances",
		versions: SINCE_515_1644,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, INSTANCE_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "register_typepath",
		versions: since(TYPEPATH_NONE_CHECK),
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"53 83 EC 18 8B 44 24 20 3B 05 ?? ?? ?? ?? 73 ?? 3D FF FF 00 00 74 ?? 69 D8 94 00 00 00 03 1D ?? ?? ?? ?? 74 ?? 80 BB 8C 00 00 00 00 74 \
			 ?? 8D 44 24 08 83 EC 04 6A 06"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "register_typepath",
		versions: until(LINUX_FRAMELESS, TYPEPATH_NONE_CHECK),
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"53 83 EC 18 8B 44 24 20 3B 05 ?? ?? ?? ?? 73 ?? 69 D8 94 00 00 00 03 1D ?? ?? ?? ?? 74 ?? 80 BB 8C 00 00 00 00 74 ?? 8D 44 24 08 83 EC \
			 04 6A 06"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "procdef_add",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 31 C0 89 E5 57 56 53 83 EC 2C 8B 35 ?? ?? ?? ?? 8B 7D 08 85 F6 74 ?? 31 F6 31 DB"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "misc_intern",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 B8 FF FF 00 00 89 E5 83 EC 68 89 5D F4 8B 5D 08 89 75 F8 89 7D FC 85 DB 74"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "obj_type_count",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "proc_name_maps",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(6))
	},
	Recipe {
		name: "global_values",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, VAR_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(3))
	},
	Recipe {
		name: "global_vars",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"66 D1 E8 0F B7 C0 89 44 24 04 E8 ?? ?? ?? ?? 0F B7 06 89 3C 24 89 44 24 04 E8 ?? ?? ?? ?? 89 3C 24 A3 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "in_list_count",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, IN_LIST_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "in_list_procs",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, IN_LIST_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "mob_count",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, MOB_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "mobs",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, MOB_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(2))
	},
	Recipe {
		name: "instance_count",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, INSTANCE_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "instances",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, INSTANCE_LOADER_EARLY),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "world_name_map",
		versions: EARLY_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"89 7C 24 0C 89 5C 24 08 C7 04 24 ?? ?? ?? ?? 89 74 24 04 E8 ?? ?? ?? ?? E9"
		),
		hops: &[],
		extract: Extract::Offset(11)
	},
	Recipe {
		name: "register_typepath",
		versions: until(TYPEPATHS, LINUX_FRAMELESS),
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 89 E5 53 83 EC 24 8B 45 08 3B 05 ?? ?? ?? ?? 73 ?? 69 D8 94 00 00 00 03 1D ?? ?? ?? ?? 74 ?? 80 BB 8C 00 00 00 00 74 ?? C7 44 24 08 \
			 06 00 00 00"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_send",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND_EARLY),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_send",
		versions: SINCE_515_1647,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "connection_count",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND_EARLY),
		hops: &[0],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "connection_count",
		versions: SINCE_515_1647,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND),
		hops: &[0],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "connections",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND_EARLY),
		hops: &[0],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "connections",
		versions: SINCE_515_1647,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND),
		hops: &[0],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "appearance_manager",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND_EARLY),
		hops: &[1],
		extract: Extract::AbsMem(OperandSelect::FirstAbsMem)
	},
	Recipe {
		name: "appearance_manager",
		versions: SINCE_515_1647,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND),
		hops: &[1],
		extract: Extract::AbsMem(OperandSelect::FirstAbsMem)
	},
	Recipe {
		name: "appearance_compare",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_COMPARE_EARLY),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_compare",
		versions: SINCE_515_1647,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_COMPARE),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_update_bounds",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_BOUNDS_EARLY),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_update_bounds",
		versions: SINCE_515_1647,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_BOUNDS),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_tree_unlink",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_RELEASE_EARLY),
		hops: &[1, 0],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_tree_unlink",
		versions: SINCE_515_1647,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_RELEASE),
		hops: &[1, 0],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_tree_rebalance",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_RELEASE_EARLY),
		hops: &[1, 1],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_tree_rebalance",
		versions: SINCE_515_1647,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_RELEASE),
		hops: &[1, 3],
		extract: Extract::Entry
	},
	Recipe {
		name: "execute_instruction",
		versions: SINCE_516,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, DISPATCH_MASK),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "execute_instruction",
		versions: LATE_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, DISPATCH_MASK_515),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "execute_instruction",
		versions: EARLY_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, DISPATCH_MASK_EARLY_515),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "Byond_ReadPointer",
		versions: EVERY_BUILD,
		anchor: Anchor::Export(c"Byond_ReadPointer"),
		hops: &[],
		extract: Extract::Entry
	}
];
