use std::collections::HashMap;
use std::path::Path;

use dmasm_bytecode::{Instruction, Label, Node, Value, Variable};
use dmasm_compiler::compiler::{compile_proc, ProcSource};
use dmasm_compiler::diagnostic::{Diagnostic, Id};
use dreammaker::objtree::{ProcValue, TypeRef};
use dreammaker::FileId;

use crate::dmb::Dmb;
use crate::env::{Assemble, Disassemble};

type Nodes = Vec<Node>;

#[derive(Default)]
pub struct Outcome {
    pub procs: usize,
    pub exact: usize,
    pub equivalent: usize,
    pub known: usize,
    pub findings: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Allowed {
    Anything,
    RoundedAngles,
}

pub fn compare(
    dme: &Path,
    dmb: &Dmb,
    byond: dmasm_compiler::ByondVersion,
    known: &HashMap<String, Allowed>,
    show: bool,
) -> Outcome {
    let mut out = Outcome::default();
    let base = dme.parent().unwrap();
    let ctx = dreammaker::Context::default();
    byond.read_as(&ctx);
    let pp = match dreammaker::Preprocessor::new(&ctx, dme.to_owned()) {
        Ok(pp) => pp,
        Err(e) => {
            out.findings.push(format!("the parser can't read it: {e}"));
            return out;
        }
    };
    let mut parser = dreammaker::Parser::new(&ctx, pp);
    parser.enable_procs();
    let tree = parser.parse_object_tree();
    dmasm_compiler::compiler::new_tree();
    let file_name = |id: FileId| {
        let path = ctx.file_path(id);
        path.strip_prefix(base)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/")
    };

    let mut by_path: HashMap<&[u8], Vec<usize>> = HashMap::new();
    for (i, p) in dmb.procs.iter().enumerate() {
        if let Some(path) = dmb.proc_path(p) {
            by_path.entry(path).or_default().push(i);
        }
    }
    let mut call_names: HashMap<String, Vec<u8>> = HashMap::new();
    for p in &dmb.procs {
        if let (Some(path), Some(name)) = (dmb.proc_path(p), p.name.and_then(|n| dmb.string(n))) {
            call_names
                .entry(String::from_utf8_lossy(path).into_owned())
                .or_insert_with(|| name.to_vec());
        }
    }
    let mut asm = Assemble::new(dmb);
    let mut used: HashMap<String, usize> = HashMap::new();

    for ty in tree.iter_types() {
        for (name, type_proc) in ty.get().procs.iter() {
            for (index, value) in type_proc.value.iter().enumerate() {
                if is_builtin(value) {
                    continue;
                }
                let path = dmb_path(ty, name.as_str(), index);
                let n = used.entry(path.clone()).or_default();
                let Some(&id) = by_path.get(path.as_bytes()).and_then(|v| v.get(*n)) else {
                    out.findings
                        .push(format!("{path}: not in DreamMaker's .dmb"));
                    continue;
                };
                *n += 1;
                out.procs += 1;
                let theirs = dmb.bytecode(&dmb.procs[id]);

                let mut file = file_name(value.location.file);
                if let Some(Node::Instruction(Instruction::DbgFile(f), _)) =
                    disassemble(dmb, &theirs[..theirs.len().min(2)]).first()
                {
                    let f = String::from_utf8_lossy(&f.0).into_owned();
                    if file.ends_with(&format!("/{f}")) {
                        file = f;
                    }
                }
                let source = ProcSource {
                    tree: &tree,
                    ty,
                    name: name.as_str(),
                    value,
                    file: &file,
                    debug_info: theirs.first() == Some(&dmasm_bytecode::Opcode::DbgFile.word()),
                    index: None,
                    byond,
                    file_names: Some(&file_name),
                };
                let allowed = known.get(&path).copied();
                let verdict = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    compile_proc(&source)
                })) {
                    Ok(Ok(compiled)) => {
                        let angles = if allowed == Some(Allowed::RoundedAngles) {
                            folded_angles(&compiled.warnings)
                        } else {
                            vec![]
                        };
                        asm.set_proc_globals(theirs);
                        match dmasm_bytecode::assembler::assemble(&compiled.nodes, &mut asm) {
                            Ok(ours) => judge(dmb, &ours, theirs, &call_names, &angles, show),
                            Err(e) => Err(format!("assemble error: {e:?}")),
                        }
                    }
                    Ok(Err(e)) => Err(format!("compile error: {e}")),
                    Err(_) => Err("the compiler panicked".to_owned()),
                };
                match (verdict, allowed) {
                    (Ok(Verdict::RoundedAngles), _) => out.known += 1,
                    (Ok(_), Some(_)) => out.findings.push(format!(
                        "{path}: is listed as a known difference, but its code is DreamMaker's"
                    )),
                    (Ok(Verdict::Exact), None) => out.exact += 1,
                    (Ok(_), None) => out.equivalent += 1,
                    (Err(_), Some(Allowed::Anything)) => out.known += 1,
                    (Err(why), _) => out.findings.push(format!("{path}: {why}")),
                }
            }
        }
    }
    for path in known.keys() {
        if !used.contains_key(path) {
            out.findings.push(format!(
                "{path}: listed as a known difference, but the case has no such proc"
            ));
        }
    }
    out
}

pub enum Verdict {
    Exact,
    Equivalent,
    RoundedAngles,
}

fn judge(
    dmb: &Dmb,
    ours: &[u32],
    theirs: &[u32],
    names: &HashMap<String, Vec<u8>>,
    angles: &[(f32, f32)],
    show: bool,
) -> Result<Verdict, String> {
    if ours == theirs {
        return Ok(Verdict::Exact);
    }
    let mut our_nodes = disassemble(dmb, ours);
    let mut their_nodes = disassemble(dmb, theirs);
    let ours_reused = normalize(&mut our_nodes, names);
    let theirs_reused = normalize(&mut their_nodes, names);
    let taken = take_rounded_angles(&mut our_nodes, &their_nodes, angles);
    let both = |what: &str| {
        let mut s = what.to_owned();
        if show {
            s.push_str(&side_by_side(&our_nodes, &their_nodes));
        }
        s
    };
    if canonical(&our_nodes) == canonical(&their_nodes) {
        if let Some(at) = ours_reused
            .iter()
            .zip(&theirs_reused)
            .position(|(o, t)| *o > 1 && *t == 0)
        {
            return Err(both(&format!(
                "ours reuses the cache where DreamMaker reloads it, at node {at}"
            )));
        }
        if let Some(at) = ours_reused
            .iter()
            .zip(&theirs_reused)
            .position(|(o, t)| *o == 0 && *t > 1)
        {
            return Err(both(&format!(
                "ours reloads a chain through fields where DreamMaker reuses it, at node {at}"
            )));
        }
        return Ok(if taken > 0 {
            Verdict::RoundedAngles
        } else {
            Verdict::Equivalent
        });
    }
    if without_labels(&our_nodes) == without_labels(&their_nodes) {
        return Err(both("differs only in jump targets"));
    }
    Err(both(&first_difference(&our_nodes, &their_nodes)))
}

fn folded_angles(warnings: &[Diagnostic]) -> Vec<(f32, f32)> {
    warnings
        .iter()
        .filter(|w| w.id == Id::FoldedAngle)
        .filter_map(|w| {
            let (_, ours) = w.message.split_once(" is folded to ")?;
            let (_, theirs) = w.message.rsplit_once(" folds it to ")?;
            let ours = ours.split_once(',')?.0.parse().ok()?;
            let theirs = theirs.strip_suffix('.')?.parse().ok()?;
            Some((ours, theirs))
        })
        .collect()
}

fn take_rounded_angles(ours: &mut Nodes, theirs: &Nodes, angles: &[(f32, f32)]) -> usize {
    fn pushed(node: &Node) -> Option<f32> {
        match node {
            Node::Instruction(Instruction::PushVal(Value::Number(n)), _) => Some(*n),
            Node::Instruction(Instruction::PushInt(n), _) => Some(*n as f32),
            _ => None,
        }
    }
    let instruction = |n: &&mut Node| matches!(n, Node::Instruction(..));
    let mut our_code: Vec<&mut Node> = ours.iter_mut().filter(instruction).collect();
    let their_code: Vec<&Node> = theirs
        .iter()
        .filter(|n| matches!(n, Node::Instruction(..)))
        .collect();
    if angles.is_empty() || our_code.len() != their_code.len() {
        return 0;
    }
    let mut taken = 0;
    for (o, t) in our_code.iter_mut().zip(their_code) {
        if let (Some(a), Some(b)) = (pushed(o), pushed(t)) {
            let bits = (a.to_bits(), b.to_bits());
            if angles
                .iter()
                .any(|(x, y)| (x.to_bits(), y.to_bits()) == bits)
            {
                **o = t.clone();
                taken += 1;
            }
        }
    }
    taken
}

fn dmb_path(ty: TypeRef, name: &str, index: usize) -> String {
    let tp = ty.get();
    let declared_here = tp
        .procs
        .get(name)
        .and_then(|p| p.declaration.as_ref())
        .filter(|d| d.location.file != FileId::BUILTINS);
    let first_real = tp.procs[name]
        .value
        .iter()
        .position(|v| !is_builtin(v))
        .unwrap_or(0);
    let mut path = String::new();
    if !ty.is_root() {
        path.push_str(&tp.path);
    }
    match declared_here {
        Some(decl) if index == first_real => {
            path.push('/');
            path.push_str(decl.kind.name());
            path.push('/');
            path.push_str(name);
        }
        _ => {
            path.push('/');
            path.push_str(name);
        }
    }
    path
}

fn is_builtin(value: &ProcValue) -> bool {
    value.location.file == FileId::BUILTINS
}

fn disassemble(dmb: &Dmb, words: &[u32]) -> Nodes {
    let (nodes, _) = dmasm_bytecode::disassembler::disassemble(words, &mut Disassemble(dmb));
    nodes.into_iter().map(Node::strip_debug_data).collect()
}

fn normalize(nodes: &mut Nodes, names: &HashMap<String, Vec<u8>>) -> Vec<usize> {
    use Instruction::*;
    fn fix(var: &mut Variable, names: &HashMap<String, Vec<u8>>) {
        match var {
            Variable::StaticProc(p) | Variable::StaticVerb(p) => {
                let name = dmasm_bytecode::DMString(
                    names
                        .get(&p.0)
                        .cloned()
                        .unwrap_or_else(|| p.0.clone().into_bytes()),
                );
                *var = if matches!(var, Variable::StaticProc(_)) {
                    Variable::DynamicProc(name)
                } else {
                    Variable::DynamicVerb(name)
                };
            }
            Variable::SetCache(a, b) => {
                fix(a, names);
                fix(b, names);
            }
            Variable::Initial(a)
            | Variable::IsSaved(a)
            | Variable::PtrRef(a)
            | Variable::PtrDeref(a) => fix(a, names),
            Variable::Global(g) => {
                if let Some(id) = g.id.take() {
                    g.name.0.extend_from_slice(format!("#{id}").as_bytes());
                }
            }
            _ => {}
        }
    }
    for node in nodes.iter_mut() {
        if let Node::Instruction(ins, _) = node {
            if let Some(v) = var_operand(ins) {
                fix(v, names);
            }
        }
    }
    nodes.retain(|n| !matches!(n, Node::Instruction(DbgLine(_) | DbgFile(_), _)));

    let mut jumps: HashMap<u32, (u32, Option<Vec<Variable>>)> = HashMap::new();
    let mut held: Option<Vec<Variable>> = None;
    let mut saved: Vec<Option<Vec<Variable>>> = vec![];
    let mut falls_through = true;
    let mut expanded = vec![0; nodes.len()];
    for (index, node) in nodes.iter_mut().enumerate() {
        let ins = match node {
            Node::Label(Label::Offset(off)) => {
                held = match (falls_through, jumps.remove(off)) {
                    (false, Some((1, state))) => state,
                    _ => None,
                };
                falls_through = true;
                continue;
            }
            Node::Label(_) => {
                held = None;
                continue;
            }
            Node::Instruction(ins, _) => ins,
            Node::Comment(_) => continue,
        };
        falls_through = !matches!(
            ins,
            Jmp(_) | JmpLoop(_) | Ret | End | Switch(_) | SwitchRange(_)
        );
        let mut targets = jump_targets(ins);
        targets.sort_unstable();
        targets.dedup();
        for target in targets {
            let entry = jumps.entry(target).or_insert((0, held.clone()));
            entry.0 += 1;
            if entry.1 != held {
                entry.1 = None;
            }
        }
        match ins {
            PushCache => {
                saved.push(held.clone());
                continue;
            }
            PopCache => {
                held = saved.pop().flatten();
                continue;
            }
            SetVar(Variable::Cache) | SetCacheJmpIfNull(_) | SetCachePopJmpIfNull(_) => {
                held = None;
                continue;
            }
            _ => {}
        }
        let Some(v) = var_operand(ins) else { continue };
        let mut bases = vec![];
        let mut cursor = &*v;
        while let Variable::SetCache(b, rest) = cursor {
            bases.push((**b).clone());
            cursor = rest;
        }
        if !bases.is_empty() {
            held = Some(bases);
        } else if uses_cache(v) {
            if let Some(h) = &held {
                let mut full = std::mem::replace(v, Variable::Null);
                for b in h.iter().rev() {
                    full = Variable::SetCache(Box::new(b.clone()), Box::new(full));
                }
                *v = full;
                expanded[index] = h.len();
            }
        }
    }
    expanded
}

fn jump_targets(ins: &Instruction) -> Vec<u32> {
    use Instruction::*;
    let off = |l: &Label| match l {
        Label::Offset(o) => Some(*o),
        Label::Named(_) => None,
    };
    match ins {
        Jmp(l)
        | Jz(l)
        | Jnz(l)
        | JmpLoop(l)
        | JzLoop(l)
        | JnzLoop(l)
        | JmpOr(l)
        | JmpAnd(l)
        | SetCacheJmpIfNull(l)
        | SetCachePopJmpIfNull(l)
        | Spawn(l)
        | Try(l)
        | Catch(l)
        | TryJmp(l)
        | ForRange(l, _)
        | ForRangeStep(l, _) => off(l).into_iter().collect(),
        Switch(p) => p
            .cases
            .iter()
            .map(|c| &c.1)
            .chain([&p.default])
            .filter_map(off)
            .collect(),
        SwitchRange(p) => p
            .cases
            .iter()
            .map(|c| &c.1)
            .chain(p.range_cases.iter().map(|c| &c.2))
            .chain([&p.default])
            .filter_map(off)
            .collect(),
        PickSwitch(p) => p
            .cases
            .iter()
            .map(|c| &c.1)
            .chain([&p.default])
            .filter_map(off)
            .collect(),
        PickProb(p) => p.cases.iter().filter_map(off).collect(),
        _ => vec![],
    }
}

fn uses_cache(v: &Variable) -> bool {
    match v {
        Variable::Field(_)
        | Variable::DynamicProc(_)
        | Variable::DynamicVerb(_)
        | Variable::StaticProc(_)
        | Variable::StaticVerb(_) => true,
        Variable::Initial(inner) | Variable::IsSaved(inner) => uses_cache(inner),
        _ => false,
    }
}

pub fn var_operand(ins: &mut Instruction) -> Option<&mut Variable> {
    use Instruction::*;
    match ins {
        GetVar(v)
        | SetVar(v)
        | SetVarExpr(v)
        | AugAdd(v)
        | AugSub(v)
        | AugMul(v)
        | AugDiv(v)
        | AugMod(v)
        | AugBand(v)
        | AugBor(v)
        | AugXor(v)
        | AugLShift(v)
        | AugRShift(v)
        | AugFloatMod(v)
        | AssignInto(v)
        | PreInc(v)
        | PostInc(v)
        | PreDec(v)
        | PostDec(v)
        | Inc(v)
        | Dec(v)
        | Call(v, _)
        | CallStatement(v, _)
        | KeyValueIter(v)
        | ForRange(_, v)
        | ForRangeStep(_, v) => Some(v),
        _ => None,
    }
}

fn canonical(nodes: &Nodes) -> Vec<String> {
    let mut ordinal = HashMap::new();
    for node in nodes {
        if let Node::Label(l) = node {
            let n = ordinal.len();
            ordinal.entry(l.to_string()).or_insert(n);
        }
    }
    nodes
        .iter()
        .filter_map(|node| match node {
            Node::Label(l) => Some(format!("L{}:", ordinal[&l.to_string()])),
            Node::Instruction(i, _) => Some(
                i.to_string()
                    .split(' ')
                    .map(|w| {
                        let (label, rest) = w.split_at(w.find(',').unwrap_or(w.len()));
                        match ordinal.get(label) {
                            Some(n) if label.starts_with("LAB_") => format!("L{n}{rest}"),
                            _ => w.to_owned(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            Node::Comment(_) => None,
        })
        .collect()
}

fn without_labels(nodes: &Nodes) -> Vec<String> {
    nodes
        .iter()
        .filter_map(|n| match n {
            Node::Instruction(i, _) => Some(
                i.to_string()
                    .split(' ')
                    .map(|w| if w.starts_with("LAB_") { "LAB" } else { w })
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        })
        .collect()
}

fn first_difference(ours: &Nodes, theirs: &Nodes) -> String {
    let (a, b) = (without_labels(ours), without_labels(theirs));
    let same = |x: &String, y: &String| x == y || (x == "PushVal null" && y == "GetVar null");
    let at = a
        .iter()
        .zip(&b)
        .position(|(x, y)| !same(x, y))
        .unwrap_or(a.len().min(b.len()));
    let show = |v: &[String]| v.get(at).map_or("<end>", String::as_str).to_owned();
    format!(
        "at instruction {at}: ours `{}`, DreamMaker's `{}`",
        show(&a),
        show(&b)
    )
}

fn side_by_side(ours: &Nodes, theirs: &Nodes) -> String {
    let (a, b) = (canonical(ours), canonical(theirs));
    let mut s = format!("\n      {:<60} | DreamMaker's\n", "ours");
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).map_or("", String::as_str);
        let y = b.get(i).map_or("", String::as_str);
        let mark = if x == y { ' ' } else { '*' };
        s.push_str(&format!("    {mark} {x:<60} | {y}\n"));
    }
    s
}
