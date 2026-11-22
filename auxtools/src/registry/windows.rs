use byond_scan::{Anchor, Extract, OperandSelect, Recipe, SignatureTreatment, VersionRange};

use super::builds::{since, until, FIRST_515, FIRST_516, TYPEPATHS, TYPEPATH_NONE_CHECK};

const EVERY_BUILD: VersionRange = since(FIRST_515);
const SINCE_516: VersionRange = since(FIRST_516);
const BYOND_515: VersionRange = until(FIRST_515, FIRST_516);

const GLOBAL_VARS_SIG: &str = "A3 ?? ?? ?? ?? 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 4B 01 00 00 A3 ?? ?? ?? ?? A1 ?? ?? ?? ?? 8D 34 85 00 00 00 \
                               00 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 2D 01 00 00 33 DB A3 ?? ?? ?? ?? 39 1D";

const SUSPENDED_SIG: &str =
	"55 8B EC 56 8B 75 08 56 80 4E 04 04 E8 ?? ?? ?? ?? 8B 0D ?? ?? ?? ?? 83 C4 04 3B 0D ?? ?? ?? ?? 73 ?? A1 ?? ?? ?? ?? 8B 04 88";

const PROCDEF_ACCESSOR_SIG: &str = "55 8B EC 8B 45 08 3B 05 ?? ?? ?? ?? 72 04 33 C0 5D C3 6B C0 2C 03 05";
const STRING_ACCESSOR_SIG: &str = "55 8B EC 8B 4D 08 3B 0D ?? ?? ?? ?? 73 ?? A1 ?? ?? ?? ?? 8B 04 88 85 C0 0F 85";

const ALIST_SIG: &str = "3B 35 ?? ?? ?? ?? 73 ?? A1 ?? ?? ?? ?? C7 04 B0 00 00 00 00 46 3B 35 ?? ?? ?? ?? 72 EB";

const TYPE_MAP_ACCESSOR: &str = "55 8B EC 8B 4D 08 3B 0D ?? ?? ?? ?? 77 0A A1 ?? ?? ?? ?? 8D 04 C8 5D C3 33 C0 5D C3";

const TYPE_LOADER: &str = "69 F0 98 00 00 00 83 C4 04 A3 ?? ?? ?? ?? 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ?? A1 ?? ?? \
                           ?? ?? 8D 34 85 00 00 00 00 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ?? A1 ?? ?? ?? ?? 8D 34 C5 \
                           00 00 00 00 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ?? A1 ?? ?? ?? ?? 8D 34 85 00 00 00 00 56 \
                           FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? 33 F6 A3 ?? ?? ?? ??";

const TYPE_LOADER_515: &str = "A3 ?? ?? ?? ?? 8D 34 C0 C1 E6 04 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ?? A1 ?? ?? ?? ?? \
                               8D 34 85 00 00 00 00 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ?? A1 ?? ?? ?? ?? 8D 34 C5 00 \
                               00 00 00 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? A3 ?? ?? ?? ?? A1 ?? ?? ?? ?? 8D 34 85 00 00 00 00 56 \
                               FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? 33 F6 A3 ?? ?? ?? ??";

const BEFORE_TYPEPATHS: VersionRange = until(FIRST_515, TYPEPATHS);

const WITH_TYPEPATHS: VersionRange = since(TYPEPATHS);

const IN_LIST_LOADER: &str = "8D 34 85 00 00 00 00 83 C4 04 A3 ?? ?? ?? ?? 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? 33 F6 A3 ?? ?? ?? \
                              ?? 39 35 ?? ?? ?? ?? 76 ?? 80 3D ?? ?? ?? ?? 00 57 74 ?? E8 ?? ?? ?? ?? 8B C8 EB ?? E8 ?? ?? ?? ?? 0F B7 C8 A1 ?? ?? \
                              ?? ?? 83 C4 04 89 0C B0";

const MOB_LOADER: &str = "89 35 ?? ?? ?? ?? C1 E6 04 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? 33 DB A3 ?? ?? ?? ?? 39 1D ?? ?? ?? ?? 76";

const INSTANCE_LOADER: &str = "A3 ?? ?? ?? ?? 8D 34 40 C1 E6 02 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 ?? ?? ?? ?? 33 DB A3 ?? ?? ?? ?? 39 1D ?? \
                               ?? ?? ?? 76 ?? 33 F6 A1 ?? ?? ?? ?? 03 C6 57 50 E8";

const APPEARANCE_SEND_515: &str =
	"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 81 EC E4 08 00 00 A1 ?? ?? ?? ?? 33 C5 89 45 F0 53 56 57 50 8D 45 F4 64 A3 00 00 00 00 0F B7";

const APPEARANCE_SEND: &str =
	"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 81 EC E4 08 00 00 A1 ?? ?? ?? ?? 33 C5 89 45 F0 53 56 57 50 8D 45 F4";

const APPEARANCE_COMPARE_515: &str =
	"55 8B EC 56 8B F1 57 8B 7D 08 8B 56 1C 2B 57 1C 0F 85 ?? ?? ?? ?? 8B 56 30 2B 57 30 0F 85 ?? ?? ?? ?? 8B 56 34 2B 57 34";

const APPEARANCE_COMPARE: &str =
	"55 8B EC 53 56 8B F1 57 8B 7D 08 8B 56 1C 2B 57 1C 0F 85 ?? ?? ?? ?? 8B 56 30 2B 57 30 0F 85 ?? ?? ?? ?? 8B 56 34 2B 57";

const APPEARANCE_BOUNDS_515: &str =
	"55 8B EC 83 EC 0C 8D 45 F4 56 57 6A 00 68 FF FF 00 00 50 8B F1 E8 ?? ?? ?? ?? 80 BE B1 00 00 00 00 8D BE C8 00 00 00 F3";

const APPEARANCE_BOUNDS: &str =
	"55 8B EC 83 EC 0C 8D 45 F4 56 57 6A 00 68 FF FF 00 00 50 8B F1 E8 ?? ?? ?? ?? 80 BE B5 00 00 00 00 8D BE CC 00 00 00 F3";

const APPEARANCE_RELEASE: &str =
	"68 ?? ?? ?? ?? E8 ?? ?? ?? ?? 83 C4 0C 8D 4F 14 56 E8 ?? ?? ?? ?? B8 FF FF 00 00 EB 50 8B 47 44 8B 5E 14 89 85 F8 FE FF";

pub(super) const RECIPES: &[Recipe] = &[
	Recipe {
		name: "get_proc_array_entry",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, PROCDEF_ACCESSOR_SIG),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_string_table_entry",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, STRING_ACCESSOR_SIG),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_string_id",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 8B 45 ?? 83 EC ?? 53 56 8B 35 ?? ?? ?? ?? 57 85 C0 75 ?? 68 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "call_proc_by_id",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 81 EC ?? ?? ?? ?? A1 ?? ?? ?? ?? 33 C5 89 45 ?? 8B 55 ?? 8B 45 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "call_datum_proc_by_name",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC ?? 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 75 1C 8D 45 \
			 F3 8B 7D 18 8B 5D 10 6A 00"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_variable",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC 0C 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 5D ?? 8B 75 \
			 ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_variable",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC 24 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 4D ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "inc_ref_count",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? FF ?? ?? FF ?? ?? E8 ?? ?? ?? ?? 8D ?? ?? 56 E8"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "dec_ref_count",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 83 C4 0C 81 FF FF FF 00 00 74 ?? 85 FF 74"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_assoc_element",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 51 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 5D 08 80"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_assoc_element",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 ?? ?? ?? ?? 50 83 EC ?? 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 ?? 64 A3 ?? ?? ?? ?? 8B 5D ?? 80 FB \
			 0F"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "create_list",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 8B 0D ?? ?? ?? ?? 56 85 C9 74 1B A1 ?? ?? ?? ?? 49 89 0D ?? ?? ?? ?? 8B 34 88 81 FE FF FF 00 00"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "append_to_list",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 8B 4D ?? 0F B6 C1 48 56 57 83 F8 54 0F 87 ?? ?? ?? ?? 0F B6 80 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "remove_from_list",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 83 EC 08 53 8B 5D ?? 0F B6 C3 48 56 57 83 F8 54 0F 87 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_length",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 8B 4D ?? 83 EC ?? 0F B6 C1 48 53 56 57 83 F8 ?? 0F 87 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "value_is_list",
		versions: SINCE_516,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC ?? 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 89 65 ?? 8B 45 \
			 08 0F B6 C0 83 C0 F1 83 F8 46"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "get_misc_by_id",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 83 C4 04 85 C0 74 08 0F B7 38 8B 70 08 EB 04 33 FF 33 F6 0F B7 C7 50 89 45 F8 E8 ?? ?? ?? ??"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "to_string",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 83 C4 08 8B F0 EB ?? 80 FB 2A 75 ?? 66 83 7D F8"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "runtime",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::OffsetByCall,
			"E8 ?? ?? ?? ?? 83 C4 04 8B 85 ?? ?? ?? ?? 0F B6 C0 51 66 0F 6E C0 0F 5B C0"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "current_execution_context",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"A1 ?? ?? ?? ?? 56 53 6A 00 8B 00 57 6A ?? 89 4D ?? FF 70 ?? 8B 4D ?? FF"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "variable_names",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, GLOBAL_VARS_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(4))
	},
	Recipe {
		name: "suspended_procs",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, SUSPENDED_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "suspended_procs_buffer",
		versions: EVERY_BUILD,
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
		versions: BYOND_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 28 A1 ?? ?? ?? ?? 33 C5 89 45 F0 53 56 57 50 8D 45 F4 64 A3 00 00 00 00 8B 5D"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_variable",
		versions: BYOND_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 0C 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 4D"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "set_assoc_element",
		versions: BYOND_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 83 EC 14 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 8B 5D 08 80"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "append_to_list",
		versions: BYOND_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 8B 4D 08 0F B6 C1 48 56 57 83 F8 53 0F 87 AA 00 00 00 0F B6 80 ?? ?? ?? ?? FF 24 85 ?? ?? ?? ?? FF 75 0C E8 ?? ?? ?? ?? 8B F0 \
			 83 C4 04 85 F6 0F 84 94 00 00 00 8B 46 0C 40 50 56 E8 ?? ?? ?? ?? 8B 7E 0C 83 C4 08 85"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "remove_from_list",
		versions: BYOND_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 83 EC 08 53 8B 5D 08 0F B6 C3 48 56 57 83 F8 53 0F 87 23 01 00 00 0F B6 80 ?? ?? ?? ?? 8B 4D 10 FF 24 85 ?? ?? ?? ?? 8B 7D 0C \
			 6A 0F 57 53 E8 ?? ?? ?? ?? 50 E8 ?? ?? ?? ?? 83 C4 10 85 C0 0F 84 02 01 00 00 8B 08 8B 40 0C 85 C0 0F 84 F5 00 00 00 8B 75 14 8B 5D 10"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "value_is_list",
		versions: BYOND_515,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 51 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 64 A3 00 00 00 00 89 65 F0 8B 4D 08 0F \
			 B6 C1 83 C0 F1 83 F8 45"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "procdef_add",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC A1 ?? ?? ?? ?? 53 8B 5D 08 56 33 F6 57 85 C0 74 ?? 33 FF A1 ?? ?? ?? ?? 03 C7 53 50 E8"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "misc_intern_copy",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 56 8B 35 ?? ?? ?? ?? 57 85 F6 74 0A 8B 46 08 A3 ?? ?? ?? ?? EB 0D 6A 0C FF 15 ?? ?? ?? ?? 83 C4 04 8B F0 85 F6 74 ?? 66 8B 4D \
			 0C"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "obj_type_count",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_MAP_ACCESSOR),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "proc_name_maps",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_MAP_ACCESSOR),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "global_values",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, GLOBAL_VARS_SIG),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "global_vars",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"0F B7 07 50 53 E8 ?? ?? ?? ?? 53 A3 ?? ?? ?? ?? FF 15 ?? ?? ?? ?? FF 35"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "AllocShared",
		versions: EVERY_BUILD,
		anchor: Anchor::Export(c"?AllocShared@@YAPAXK@Z"),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "FreeShared",
		versions: EVERY_BUILD,
		anchor: Anchor::Export(c"?FreeShared@@YA_NPAX@Z"),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "CopyShared",
		versions: EVERY_BUILD,
		anchor: Anchor::Export(c"?CopyShared@@YAPAXPAX@Z"),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "type_preorder",
		versions: WITH_TYPEPATHS,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(4))
	},
	Recipe {
		name: "type_preorder",
		versions: BEFORE_TYPEPATHS,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER_515),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(4))
	},
	Recipe {
		name: "type_appearance_cache",
		versions: WITH_TYPEPATHS,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(10))
	},
	Recipe {
		name: "type_appearance_cache",
		versions: BEFORE_TYPEPATHS,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_LOADER_515),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(10))
	},
	Recipe {
		name: "in_list_count",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, IN_LIST_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "in_list_procs",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, IN_LIST_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(2))
	},
	Recipe {
		name: "world_name_map",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"F6 45 08 02 0F 84 ?? ?? ?? ?? FF 75 20 FF 75 10 FF 75 0C 68 ?? ?? ?? ?? E8 ?? ?? ?? ?? 83 C4 10"
		),
		hops: &[],
		extract: Extract::Offset(20)
	},
	Recipe {
		name: "mob_count",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, MOB_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "mobs",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, MOB_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "instance_count",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, INSTANCE_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "instances",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, INSTANCE_LOADER),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "register_typepath",
		versions: since(TYPEPATH_NONE_CHECK),
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 57 8B 7D 08 3B 3D ?? ?? ?? ?? 0F 83 ?? ?? ?? ?? 81 FF FF FF 00 00 0F 84 ?? ?? ?? ?? 56 69 F7 98 00 00 00 03 35 ?? ?? ?? ?? 0F \
			 84 ?? ?? ?? ?? 80 BE 8C 00 00 00 00 75"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "register_typepath",
		versions: until(TYPEPATHS, TYPEPATH_NONE_CHECK),
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 57 8B 7D 08 3B 3D ?? ?? ?? ?? 0F 83 ?? ?? ?? ?? 56 69 F7 98 00 00 00 03 35 ?? ?? ?? ?? 0F 84 ?? ?? ?? ?? 80 BE 8C 00 00 00 00 \
			 75"
		),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_send",
		versions: BYOND_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND_515),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_send",
		versions: SINCE_516,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "connection_count",
		versions: BYOND_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND_515),
		hops: &[0],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "connection_count",
		versions: SINCE_516,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND),
		hops: &[0],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	},
	Recipe {
		name: "connections",
		versions: BYOND_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND_515),
		hops: &[0],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "connections",
		versions: SINCE_516,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND),
		hops: &[0],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	},
	Recipe {
		name: "appearance_manager",
		versions: BYOND_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND_515),
		hops: &[1],
		extract: Extract::AbsMem(OperandSelect::FirstAbsMem)
	},
	Recipe {
		name: "appearance_manager",
		versions: SINCE_516,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_SEND),
		hops: &[1],
		extract: Extract::AbsMem(OperandSelect::FirstAbsMem)
	},
	Recipe {
		name: "appearance_compare",
		versions: BYOND_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_COMPARE_515),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_compare",
		versions: SINCE_516,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_COMPARE),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_update_bounds",
		versions: BYOND_515,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_BOUNDS_515),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_update_bounds",
		versions: SINCE_516,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_BOUNDS),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_tree_unlink",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_RELEASE),
		hops: &[1, 0],
		extract: Extract::Entry
	},
	Recipe {
		name: "appearance_tree_rebalance",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, APPEARANCE_RELEASE),
		hops: &[1, 1],
		extract: Extract::Entry
	},
	Recipe {
		name: "execute_instruction",
		versions: EVERY_BUILD,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"0F B7 47 ?? 8B 4F ?? 8B F0 8B 14 ?? 89 95 ?? ?? ?? ?? 81 FA ?? 01 00 00"
		),
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
