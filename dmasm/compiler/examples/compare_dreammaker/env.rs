use std::collections::{HashMap, HashSet};

use dmasm_bytecode::{Node, StaticScope, Variable};

use crate::dmb::Dmb;

mod tags {
    pub const MOB: u8 = 0x08;
    pub const OBJ: u8 = 0x09;
    pub const TURF: u8 = 0x0A;
    pub const AREA: u8 = 0x0B;
    pub const RESOURCE: u8 = 0x0C;
    pub const DATUM: u8 = 0x20;
    pub const SAVEFILE: u8 = 0x24;
    pub const PROC: u8 = 0x26;
    pub const LIST: u8 = 0x28;
    pub const CLIENT: u8 = 0x3B;
    pub const IMAGE: u8 = 0x3F;
    pub const BUILTIN: u8 = 0x59;
}

pub struct Disassemble<'a>(pub &'a Dmb);

impl dmasm_bytecode::disassembler::DisassembleEnv for Disassemble<'_> {
    fn get_string_data(&mut self, index: u32) -> Option<Vec<u8>> {
        self.0.string(index).map(<[u8]>::to_vec)
    }

    fn get_variable_name(&mut self, index: u32) -> Option<Vec<u8>> {
        self.0.variable_name(index).map(<[u8]>::to_vec)
    }

    fn get_proc_name(&mut self, index: u32) -> Option<String> {
        let path = self.0.proc_path(self.0.procs.get(index as usize)?)?;
        Some(String::from_utf8_lossy(path).into_owned())
    }

    fn value_to_string_data(&mut self, tag: u32, data: u32) -> Option<Vec<u8>> {
        let dmb = self.0;
        let named = |name: Option<&[u8]>, what: &str| {
            name.map_or_else(|| format!("<{what}#{data}>").into_bytes(), <[u8]>::to_vec)
        };
        Some(match tag as u8 {
            tags::PROC => named(
                dmb.procs.get(data as usize).and_then(|p| dmb.proc_path(p)),
                "proc",
            ),
            tags::RESOURCE => named(dmb.resource_name(data).map(str::as_bytes), "resource"),
            tags::MOB => named(
                dmb.mob_types
                    .get(data as usize)
                    .and_then(|&t| dmb.type_path(t)),
                "mob type",
            ),
            tags::CLIENT => b"/client".to_vec(),
            tags::LIST => b"/list".to_vec(),
            tags::SAVEFILE => b"/savefile".to_vec(),
            _ => named(dmb.type_path(data), &format!("value tag {tag:#04x}")),
        })
    }
}

pub struct Assemble<'a> {
    dmb: &'a Dmb,
    strings: HashMap<&'a [u8], u32>,
    variables: HashMap<&'a [u8], u32>,
    procs: HashMap<&'a [u8], u32>,
    types: HashMap<&'a [u8], u32>,
    mobs: HashMap<u32, u32>,
    resources: HashMap<&'a str, u32>,
    scoped_vars: HashMap<(&'a [u8], String), u32>,
    proc_globals: HashMap<Vec<u8>, Vec<u32>>,
}

impl<'a> Assemble<'a> {
    pub fn new(dmb: &'a Dmb) -> Self {
        let mut strings = HashMap::new();
        for (i, s) in dmb.strings().iter().enumerate() {
            strings.entry(s.as_slice()).or_insert(i as u32);
        }
        let mut variables = HashMap::new();
        for i in 0..dmb.variable_names.len() as u32 {
            if let Some(s) = dmb.variable_name(i) {
                variables.entry(s).or_insert(i);
            }
        }
        let mut procs = HashMap::new();
        for (i, p) in dmb.procs.iter().enumerate() {
            if let Some(s) = dmb.proc_path(p) {
                procs.entry(s).or_insert(i as u32);
            }
        }
        let mut types = HashMap::new();
        for i in 0..dmb.type_count() as u32 {
            if let Some(s) = dmb.type_path(i) {
                types.entry(s).or_insert(i);
            }
        }
        let mut mobs = HashMap::new();
        for (i, &t) in dmb.mob_types.iter().enumerate() {
            mobs.entry(t).or_insert(i as u32);
        }
        let mut defined_by = HashMap::new();
        for (t, vars) in dmb.type_vars.iter().enumerate() {
            for pair in vars.and_then(|m| dmb.misc(m)).unwrap_or_default().chunks(2) {
                defined_by.entry(pair[0]).or_insert(t as u32);
            }
        }
        let mut frame_names: HashSet<u32> = HashSet::new();
        for &(locals, params) in &dmb.proc_frames {
            frame_names.extend(locals.and_then(|m| dmb.misc(m)).unwrap_or_default());
            frame_names.extend(
                params
                    .and_then(|m| dmb.misc(m))
                    .unwrap_or_default()
                    .chunks(4)
                    .filter_map(|c| c.get(2)),
            );
        }
        let mut scoped_vars = HashMap::new();
        for i in 0..dmb.variable_names.len() as u32 {
            let Some(name) = dmb.variable_name(i) else {
                continue;
            };
            let scope = match defined_by.get(&i) {
                Some(&t) => {
                    String::from_utf8_lossy(dmb.type_path(t).unwrap_or_default()).into_owned()
                }
                None if frame_names.contains(&i) => continue,
                None => String::new(),
            };
            scoped_vars.entry((name, scope)).or_insert(i);
        }
        let mut resources = HashMap::new();
        for (name, id) in &dmb.resource_names {
            resources.entry(name.as_str()).or_insert(*id);
        }
        Assemble {
            dmb,
            strings,
            variables,
            procs,
            types,
            mobs,
            resources,
            scoped_vars,
            proc_globals: HashMap::new(),
        }
    }

    pub fn set_proc_globals(&mut self, bytecode: &[u32]) {
        fn visit(v: &Variable, out: &mut HashMap<Vec<u8>, Vec<u32>>) {
            match v {
                Variable::Global(g) => {
                    if let Some(id) = g.id {
                        let ids = out.entry(g.name.0.clone()).or_default();
                        if let Err(at) = ids.binary_search(&id) {
                            ids.insert(at, id);
                        }
                    }
                }
                Variable::SetCache(a, b) => {
                    visit(a, out);
                    visit(b, out);
                }
                Variable::Initial(a)
                | Variable::IsSaved(a)
                | Variable::PtrRef(a)
                | Variable::PtrDeref(a) => visit(a, out),
                _ => {}
            }
        }
        self.proc_globals.clear();
        let (nodes, _) =
            dmasm_bytecode::disassembler::disassemble(bytecode, &mut Disassemble(self.dmb));
        for node in nodes {
            if let Node::Instruction(mut ins, _) = node {
                if let Some(v) = crate::compare::var_operand(&mut ins) {
                    visit(v, &mut self.proc_globals);
                }
            }
        }
    }

    fn type_tag(&self, mut index: u32) -> u8 {
        for _ in 0..256 {
            match self.dmb.type_path(index) {
                Some(b"/mob") => return tags::MOB,
                Some(b"/obj" | b"/atom/movable") => return tags::OBJ,
                Some(b"/turf" | b"/atom") => return tags::TURF,
                Some(b"/area") => return tags::AREA,
                Some(b"/client") => return tags::CLIENT,
                Some(b"/image") => return tags::IMAGE,
                Some(b"/alist" | b"/pixloc" | b"/vector" | b"/callee") => return tags::BUILTIN,
                _ => {}
            }
            match self.dmb.type_parent(index) {
                Some(parent) if parent != index => index = parent,
                _ => break,
            }
        }
        tags::DATUM
    }
}

impl dmasm_bytecode::assembler::AssembleEnv for Assemble<'_> {
    fn get_string_index(&mut self, string: &[u8]) -> Option<u32> {
        self.strings.get(string).copied()
    }

    fn get_variable_name_index(&mut self, name: &[u8]) -> Option<u32> {
        self.variables.get(name).copied()
    }

    fn get_proc_index(&mut self, path: &str) -> Option<u32> {
        self.procs.get(path.as_bytes()).copied()
    }

    fn get_type(&mut self, path: &str) -> Option<(u8, u32)> {
        match path {
            "/list" => return Some((tags::LIST, 0)),
            "/savefile" => return Some((tags::SAVEFILE, 0)),
            "/client" => return Some((tags::CLIENT, 0)),
            _ => {}
        }
        if let Some(&index) = self.types.get(path.as_bytes()) {
            let tag = self.type_tag(index);
            if tag == tags::MOB {
                return Some((tag, *self.mobs.get(&index)?));
            }
            return Some((tag, index));
        }
        self.procs.get(path.as_bytes()).map(|&id| (tags::PROC, id))
    }

    fn get_global_index(&mut self, name: &[u8], scope: Option<&StaticScope>) -> Option<u32> {
        let declaring = match scope {
            Some(StaticScope::Proc { nth, .. }) => {
                if let Some(ids) = self.proc_globals.get(name) {
                    return ids.get(*nth as usize).or(ids.last()).copied();
                }
                None
            }
            Some(StaticScope::Global) => Some(""),
            Some(StaticScope::Type(path)) => Some(path.as_str()),
            None => None,
        };
        if let Some(&id) = declaring.and_then(|path| self.scoped_vars.get(&(name, path.to_owned())))
        {
            return Some(id);
        }
        self.variables.get(name).copied()
    }

    fn get_resource_index(&mut self, path: &str) -> Option<u32> {
        if let Some(&id) = self.resources.get(path) {
            return Some(id);
        }
        path.strip_prefix("<resource#")?
            .strip_suffix('>')?
            .parse()
            .ok()
    }

    fn get_instance(&mut self, path: &str, init: &[u32]) -> Option<u32> {
        let (tag, value) = self.get_type(path)?;
        let dmb = self.dmb;
        dmb.instances
            .iter()
            .position(|inst| {
                inst.kind == tag
                    && inst.value == value
                    && inst
                        .init
                        .and_then(|p| dmb.procs.get(p as usize))
                        .is_some_and(|p| dmb.bytecode(p) == init)
            })
            .map(|i| i as u32)
    }
}
