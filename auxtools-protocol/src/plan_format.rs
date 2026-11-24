use std::collections::BTreeMap;

use dmasm::{
	diagnostic::{Diagnostic, Id},
	reload::{
		BuiltinInherited, ConstValue, GlobalInit, InList, InitProc, LineMove, NewProc, NewType, NewVar, Param, Plan, ProcKey, ProcReload,
		ProcSettings, Redeclared, RemovedProc, RemovedVar, Reparented, ReparentedType, SettingsChange, StaticVar, VarDefault
	},
	DMString, GlobalVar, Instruction, Label, Node, StaticScope, Value, Variable
};

use crate::{HotReloadRequest, HotReloadResource, ShadowingProc, PROTOCOL_VERSION};

const RECORDED: (u32, u64) = (15, 0x6659_BF71_49B7_7982);

fn key(path: &str, override_id: u32) -> ProcKey {
	ProcKey {
		path: path.to_owned(),
		override_id
	}
}

fn global(name: &str, scope: Option<StaticScope>) -> Variable {
	Variable::Global(GlobalVar::named(name.as_bytes(), scope))
}

fn nodes() -> Vec<Node> {
	let in_proc = StaticScope::Proc {
		path: "/datum/a/proc/f".to_owned(),
		nth: 1
	};
	let set = |var| Node::Instruction(Instruction::SetVar(var), ());
	vec![
		Node::Comment("a comment".to_owned()),
		Node::Instruction(Instruction::DbgFile(DMString(b"code/a.dm".to_vec())), ()),
		Node::Instruction(Instruction::DbgLine(7), ()),
		Node::Instruction(Instruction::PushVal(Value::Number(1.5)), ()),
		set(global("total", None)),
		Node::Instruction(Instruction::PushVal(Value::DMString(DMString(b"text".to_vec()))), ()),
		set(global("total", Some(StaticScope::Global))),
		Node::Instruction(Instruction::PushVal(Value::Path("/datum/a".to_owned())), ()),
		set(global("count", Some(StaticScope::Type("/datum/a".to_owned())))),
		Node::Instruction(Instruction::PushVal(Value::Resource("icons/a.dmi".to_owned())), ()),
		set(global("seen", Some(in_proc))),
		Node::Instruction(Instruction::Jmp(Label::Named("end".to_owned())), ()),
		Node::Label(Label::Named("end".to_owned())),
		Node::Instruction(
			Instruction::GetVar(Variable::SetCache(
				Box::new(Variable::Src),
				Box::new(Variable::Field(DMString(b"name".to_vec())))
			)),
			()
		),
		Node::Instruction(Instruction::Ret, ()),
		Node::Instruction(Instruction::End, ()),
	]
}

fn code(path: &str) -> ProcReload {
	let value = |path: &str| ProcReload {
		proc: key(path, 0),
		nodes: nodes(),
		locals: vec![],
		params: vec![],
		settings: None,
		statics: vec![],
		warnings: vec![]
	};
	let param = |name: &str, in_list| Param {
		name: name.to_owned(),
		input: 4,
		input_extra: 1,
		in_list
	};
	ProcReload {
		proc: key(path, 2),
		nodes: nodes(),
		locals: vec!["i".to_owned()],
		params: vec![
			param("a", InList::Builtin(0x7D01)),
			param("b", InList::Kept),
			param("c", InList::New(Box::new(value("(in list)")))),
		],
		settings: Some(ProcSettings {
			waitfor: false,
			background: true,
			name: Some("Name".to_owned()),
			desc: Some("What it does".to_owned()),
			category: Some("Category".to_owned()),
			no_category: true,
			hidden: true,
			popup_menu: false,
			instant: true,
			invisibility: Some(3),
			src: Some((true, 0x7F08))
		}),
		statics: vec![
			StaticVar {
				name: "seen".to_owned(),
				scope: StaticScope::Proc {
					path: path.to_owned(),
					nth: 0
				},
				value: Some(ConstValue::Number(2.0)),
				init: None
			},
			StaticVar {
				name: "seen".to_owned(),
				scope: StaticScope::Proc {
					path: path.to_owned(),
					nth: 1
				},
				value: None,
				init: Some(Box::new(value("(static)")))
			},
		],
		warnings: vec![Diagnostic::warning(Id::Text, "a warning")]
	}
}

fn sample() -> HotReloadRequest {
	let default = |var: &str, value| VarDefault {
		var: var.to_owned(),
		value,
		types: vec!["/datum/a".to_owned(), "/datum/a/b".to_owned()]
	};
	HotReloadRequest {
		plan: Plan {
			new_types: vec![NewType {
				path: "/datum/a/new".to_owned(),
				parent: "/datum/a".to_owned(),
				fields: vec![
					("name".to_owned(), ConstValue::String("new".to_owned())),
					("icon".to_owned(), ConstValue::Resource("icons/a.dmi".to_owned())),
				]
			}],
			reloads: vec![code("/datum/a/f")],
			settings: vec![SettingsChange {
				proc: key("/datum/a/g", 1),
				settings: ProcSettings::default()
			}],
			new_procs: vec![NewProc {
				type_path: "/datum/a".to_owned(),
				name: "h".to_owned(),
				declared: true,
				verb: true,
				code: code("/datum/a/h")
			}],
			removed_procs: vec![RemovedProc {
				proc: key("/datum/a/gone", 0),
				type_path: "/datum/a".to_owned(),
				name: "gone".to_owned(),
				verb: false
			}],
			new_vars: vec![NewVar {
				type_path: "/datum/a".to_owned(),
				var: "added".to_owned(),
				flags: 5,
				value: ConstValue::Null,
				init: Some(Box::new(code("(initial value)"))),
				moved_from: vec!["/datum/a/b".to_owned()]
			}],
			redeclared: vec![Redeclared {
				type_path: String::new(),
				var: "total".to_owned(),
				flags: 2,
				value: Some(ConstValue::Path("/datum/a".to_owned())),
				init: Some(Box::new(code("(initial value)")))
			}],
			var_defaults: vec![default("hp", ConstValue::Number(4.0))],
			builtin_defaults: vec![default("desc", ConstValue::String("described".to_owned()))],
			builtin_inherited: vec![BuiltinInherited {
				var: "layer".to_owned(),
				types: vec!["/obj/a".to_owned()]
			}],
			init_procs: vec![
				InitProc {
					type_path: "/datum/a".to_owned(),
					code: Some(code("/datum/a/(init)"))
				},
				InitProc {
					type_path: "/datum/a/b".to_owned(),
					code: None
				},
			],
			reparented: vec![Reparented {
				type_path: "/datum/c".to_owned(),
				parent: "/datum/a".to_owned(),
				types: vec![ReparentedType {
					path: "/datum/c".to_owned(),
					declares: vec!["own".to_owned()],
					sets: vec!["hp".to_owned()],
					builtins: vec!["name".to_owned()],
					name: ConstValue::String("c".to_owned()),
					text: ConstValue::Null
				}]
			}],
			line_moves: vec![LineMove {
				proc: key("/datum/a/moved", 0),
				lines: vec![(3, 5), (4, 6)]
			}],
			removed_vars: vec![RemovedVar {
				type_path: "/datum/a".to_owned(),
				var: "old".to_owned()
			}]
		},
		resources: vec![HotReloadResource {
			path: "icons/a.dmi".to_owned(),
			ids: vec![3, 9]
		}],
		resource_dirs: vec!["icons".to_owned()],
		global_init: Some(GlobalInit {
			code: code("(global init)"),
			table: 1,
			proc_statics: BTreeMap::from([("/datum/a/proc/f".to_owned(), BTreeMap::from([("seen".to_owned(), 2)]))])
		}),
		shadowing_procs: vec![ShadowingProc {
			global: "total".to_owned(),
			proc: key("/datum/a/f", 1)
		}]
	}
}

fn hash(bytes: impl IntoIterator<Item = u8>) -> u64 {
	bytes.into_iter().fold(0xCBF2_9CE4_8422_2325, |hash, byte| {
		(hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3)
	})
}

fn shape() -> u64 {
	let sent = bincode::serialize(&sample()).expect("the sample serializes");
	let instructions = Instruction::SHAPES
		.iter()
		.flat_map(|(name, operands)| format!("{}({});", name, operands.join(",")).into_bytes());
	hash(sent.into_iter().chain(instructions))
}

#[test]
fn a_plan_of_another_shape_is_another_protocol_version() {
	let now = (PROTOCOL_VERSION, shape());
	let version_to_record = PROTOCOL_VERSION + u32::from(PROTOCOL_VERSION == RECORDED.0 && now.1 != RECORDED.1);
	assert!(
		now == RECORDED,
		"protocol {} with a plan of shape {:#018X}, where `RECORDED` has protocol {} with {:#018X}. If the shape is another, dmasm's reload types, \
		 its instructions or `HotReloadRequest` changed: `PROTOCOL_VERSION` needs its next number. Record `({}, {:#018X})`.",
		now.0,
		now.1,
		RECORDED.0,
		RECORDED.1,
		version_to_record,
		now.1
	);
}

#[test]
fn the_sample_comes_back_as_it_went() {
	let sent = bincode::serialize(&sample()).unwrap();
	let read: HotReloadRequest = bincode::deserialize(&sent).unwrap();
	assert_eq!(bincode::serialize(&read).unwrap(), sent);
	assert_eq!(read.plan.reloads, sample().plan.reloads);
	assert_eq!(read.len(), 16);
}
