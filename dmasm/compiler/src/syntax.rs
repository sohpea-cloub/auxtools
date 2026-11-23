use dreammaker::ast::{
    Case, Expression, Follow, ForKeyValueStatement, ForListStatement, ForRangeStatement, MiniExpr,
    Parameter, Prefab, Spanned, Statement, Term, VarStatement, VarType,
};
use dreammaker::constants::{Constant, Pop};
use dreammaker::objtree::ProcValue;

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum Node<'a> {
    Parameter(&'a Parameter),
    Statement(&'a Statement),
    Expression(&'a Expression),
    Term(&'a Term),
    Follow(&'a Follow),
    Name(&'a str),
}

pub fn walk_proc<'a>(value: &'a ProcValue, each: &mut dyn FnMut(Node<'a>)) {
    for parameter in value.parameters.iter() {
        let Parameter {
            var_type,
            name,
            default,
            in_list,
            location: _,
            absolute: _,
        } = parameter;
        each(Node::Parameter(parameter));
        type_names(var_type, each);
        each(Node::Name(name.as_str()));
        optional(default, each);
        optional(in_list, each);
    }
    if let Some(code) = &value.code {
        walk_block(code, each);
    }
}

pub(crate) fn walk_block<'a>(block: &'a [Spanned<Statement>], each: &mut dyn FnMut(Node<'a>)) {
    for statement in block {
        walk_statement(&statement.elem, each);
    }
}

pub(crate) fn walk_statement<'a>(statement: &'a Statement, each: &mut dyn FnMut(Node<'a>)) {
    each(Node::Statement(statement));
    match statement {
        Statement::Expr(value) | Statement::Throw(value) | Statement::Del(value) => {
            walk_expression(value, each)
        }
        Statement::Return(value) | Statement::Crash(value) => optional(value, each),
        Statement::While { condition, block } => {
            walk_expression(&condition.elem, each);
            walk_block(block, each);
        }
        Statement::DoWhile { block, condition } => {
            walk_block(block, each);
            walk_expression(&condition.elem, each);
        }
        Statement::If { arms, else_arm } => {
            for (condition, block) in arms {
                walk_expression(&condition.elem, each);
                walk_block(block, each);
            }
            if let Some(block) = else_arm {
                walk_block(block, each);
            }
        }
        Statement::ForInfinite { block } => walk_block(block, each),
        Statement::ForLoop {
            init,
            test,
            inc,
            block,
        } => {
            if let Some(init) = init {
                walk_statement(init, each);
            }
            if let Some(test) = test {
                walk_expression(test, each);
            }
            if let Some(inc) = inc {
                walk_statement(inc, each);
            }
            walk_block(block, each);
        }
        Statement::ForList(for_list) => {
            let ForListStatement {
                var_type,
                name,
                input_type: _,
                in_list,
                block,
            } = &**for_list;
            if let Some(var_type) = var_type {
                type_names(var_type, each);
            }
            each(Node::Name(name.as_str()));
            optional(in_list, each);
            walk_block(block, each);
        }
        Statement::ForKeyValue(for_key_value) => {
            let ForKeyValueStatement {
                var_type,
                key,
                key_input_type: _,
                value,
                in_list,
                block,
            } = &**for_key_value;
            if let Some(var_type) = var_type {
                type_names(var_type, each);
            }
            each(Node::Name(key.as_str()));
            each(Node::Name(value.as_str()));
            optional(in_list, each);
            walk_block(block, each);
        }
        Statement::ForRange(for_range) => {
            let ForRangeStatement {
                var_type,
                name,
                start,
                end,
                step,
                block,
            } = &**for_range;
            if let Some(var_type) = var_type {
                type_names(var_type, each);
            }
            each(Node::Name(name.as_str()));
            walk_expression(start, each);
            walk_expression(end, each);
            optional(step, each);
            walk_block(block, each);
        }
        Statement::Var(var) => declared(var, each),
        Statement::Vars(vars) => {
            for var in vars {
                declared(var, each);
            }
        }
        Statement::Setting {
            name,
            mode: _,
            value,
        } => {
            each(Node::Name(name.as_str()));
            walk_expression(value, each);
        }
        Statement::Spawn { delay, block } => {
            optional(delay, each);
            walk_block(block, each);
        }
        Statement::Switch {
            input,
            cases,
            default,
        } => {
            walk_expression(input, each);
            for (values, block) in cases.iter() {
                for case in &values.elem {
                    match case {
                        Case::Exact(value) => walk_expression(value, each),
                        Case::Range(from, to) => {
                            walk_expression(from, each);
                            walk_expression(to, each);
                        }
                    }
                }
                walk_block(block, each);
            }
            if let Some(block) = default {
                walk_block(block, each);
            }
        }
        Statement::TryCatch {
            try_block,
            catch_params,
            catch_block,
        } => {
            walk_block(try_block, each);
            for path in catch_params.iter() {
                for part in path.iter() {
                    each(Node::Name(part.as_str()));
                }
            }
            walk_block(catch_block, each);
        }
        Statement::Continue(label) | Statement::Break(label) => {
            if let Some(label) = label {
                each(Node::Name(label.as_str()));
            }
        }
        Statement::Goto(label) => each(Node::Name(label.as_str())),
        Statement::Label { name, block } => {
            each(Node::Name(name.as_str()));
            walk_block(block, each);
        }
    }
}

pub fn walk_expression<'a>(expression: &'a Expression, each: &mut dyn FnMut(Node<'a>)) {
    each(Node::Expression(expression));
    match expression {
        Expression::Base { term, follow } => {
            walk_term(&term.elem, each);
            for follow in follow.iter() {
                each(Node::Follow(&follow.elem));
                match &follow.elem {
                    Follow::Index(_, index) => walk_expression(index, each),
                    Follow::Field(_, name)
                    | Follow::StaticField(name)
                    | Follow::ProcReference(name) => each(Node::Name(name.as_str())),
                    Follow::Call(_, name, args) => {
                        each(Node::Name(name.as_str()));
                        all(args, each);
                    }
                    Follow::Unary(_) => {}
                }
            }
        }
        Expression::BinaryOp { op: _, lhs, rhs } | Expression::AssignOp { op: _, lhs, rhs } => {
            walk_expression(lhs, each);
            walk_expression(rhs, each);
        }
        Expression::TernaryOp { cond, if_, else_ } => {
            walk_expression(cond, each);
            walk_expression(if_, each);
            walk_expression(else_, each);
        }
    }
}

fn walk_term<'a>(term: &'a Term, each: &mut dyn FnMut(Node<'a>)) {
    each(Node::Term(term));
    match term {
        Term::Null
        | Term::Int(_)
        | Term::Float(_)
        | Term::String(_)
        | Term::Resource(_)
        | Term::As(_)
        | Term::__PROC__
        | Term::__TYPE__
        | Term::__IMPLIED_TYPE__ => {}
        Term::Ident(name) | Term::GlobalIdent(name) => each(Node::Name(name.as_str())),
        Term::Expr(inner) => walk_expression(inner, each),
        Term::Prefab(prefab) => walk_prefab(prefab, each),
        Term::InterpString(_, parts) => {
            for (value, _) in parts.iter() {
                optional(value, each);
            }
        }
        Term::Call(name, args) | Term::GlobalCall(name, args) => {
            each(Node::Name(name.as_str()));
            all(args, each);
        }
        Term::SelfCall(args) | Term::ParentCall(args) | Term::List(args) | Term::Alist(args) => {
            all(args, each)
        }
        Term::NewImplicit { args } => maybe_all(args, each),
        Term::NewPrefab { prefab, args } => {
            walk_prefab(prefab, each);
            maybe_all(args, each);
        }
        Term::NewMiniExpr { expr, args } => {
            let MiniExpr { ident, fields } = &**expr;
            each(Node::Name(ident.as_str()));
            for field in fields.iter() {
                each(Node::Name(field.ident.as_str()));
            }
            maybe_all(args, each);
        }
        Term::Input {
            args,
            input_type: _,
            in_list,
        } => {
            all(args, each);
            if let Some(in_list) = in_list {
                walk_expression(in_list, each);
            }
        }
        Term::Locate { args, in_list } => {
            all(args, each);
            if let Some(in_list) = in_list {
                walk_expression(in_list, each);
            }
        }
        Term::Pick(choices) => {
            for (weight, value) in choices.iter() {
                optional(weight, each);
                walk_expression(value, each);
            }
        }
        Term::DynamicCall(target, args) => {
            all(target, each);
            all(args, each);
        }
        Term::ExternalCall {
            library,
            function,
            args,
        } => {
            if let Some(library) = library {
                walk_expression(library, each);
            }
            walk_expression(function, each);
            all(args, each);
        }
    }
}

fn walk_prefab<'a>(prefab: &'a Prefab, each: &mut dyn FnMut(Node<'a>)) {
    let Prefab { path, vars } = prefab;
    for (_, part) in path.iter() {
        each(Node::Name(part.as_str()));
    }
    for (name, value) in vars.iter() {
        each(Node::Name(name.as_str()));
        walk_expression(value, each);
    }
}

fn declared<'a>(var: &'a VarStatement, each: &mut dyn FnMut(Node<'a>)) {
    let VarStatement {
        var_type,
        name,
        value,
    } = var;
    type_names(var_type, each);
    each(Node::Name(name.as_str()));
    optional(value, each);
}

fn type_names<'a>(var_type: &'a VarType, each: &mut dyn FnMut(Node<'a>)) {
    let VarType {
        flags: _,
        type_path,
        input_type: _,
    } = var_type;
    for part in type_path.iter() {
        each(Node::Name(part.as_str()));
    }
}

fn optional<'a>(expression: &'a Option<Expression>, each: &mut dyn FnMut(Node<'a>)) {
    if let Some(expression) = expression {
        walk_expression(expression, each);
    }
}

fn all<'a>(expressions: &'a [Expression], each: &mut dyn FnMut(Node<'a>)) {
    for expression in expressions {
        walk_expression(expression, each);
    }
}

fn maybe_all<'a>(expressions: &'a Option<Box<[Expression]>>, each: &mut dyn FnMut(Node<'a>)) {
    if let Some(expressions) = expressions {
        all(expressions, each);
    }
}

pub(crate) fn has_static_field(expression: &Expression) -> bool {
    let mut found = false;
    walk_expression(expression, &mut |node| {
        found |= matches!(node, Node::Follow(Follow::StaticField(_)));
    });
    found
}

pub fn walk_constant<'a>(constant: &'a Constant, each: &mut dyn FnMut(&'a Constant)) {
    fn arguments<'a>(args: &'a [(Constant, Option<Constant>)], each: &mut dyn FnMut(&'a Constant)) {
        for (key, value) in args {
            walk_constant(key, each);
            if let Some(value) = value {
                walk_constant(value, each);
            }
        }
    }
    fn vars<'a>(pop: &'a Pop, each: &mut dyn FnMut(&'a Constant)) {
        let Pop { path: _, vars } = pop;
        for value in vars.values() {
            walk_constant(value, each);
        }
    }
    each(constant);
    match constant {
        Constant::Null(_) | Constant::String(_) | Constant::Resource(_) | Constant::Float(_) => {}
        Constant::New { type_, args } => {
            if let Some(pop) = type_ {
                vars(pop, each);
            }
            if let Some(args) = args {
                arguments(args, each);
            }
        }
        Constant::List(args) | Constant::Call(_, args) => arguments(args, each),
        Constant::Prefab(pop) => vars(pop, each),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dreammaker::objtree::ObjectTree;

    fn parsed(code: &str) -> ObjectTree {
        let ctx = dreammaker::Context::default();
        let pp = dreammaker::Preprocessor::from_buffer(&ctx, "code.dm".into(), code.to_owned());
        let mut parser = dreammaker::Parser::new(&ctx, pp);
        parser.enable_procs();
        let tree = parser.parse_object_tree_without_builtins();
        let errors: Vec<String> = ctx.errors().iter().map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "{:?}", errors);
        tree
    }

    fn proc_of<'a>(tree: &'a ObjectTree, name: &str) -> &'a ProcValue {
        &tree.root().get().procs[name].value[0]
    }

    fn returned_by_f(tree: &ObjectTree) -> &Expression {
        match &proc_of(tree, "f").code.as_ref().unwrap()[0].elem {
            Statement::Return(Some(value)) => value,
            other => panic!("not a return: {:?}", other),
        }
    }

    const EVERYTHING: &str = r#"
/proc/everything(mob/living/par_typed, par_default = par_use + 'par.dmi', par_in in par_list)
	set set_name = "set text"
	var/obj/item/var_typed = var_use
	var/var_one, var_two = 2
	var/static/list/var_static = list(list_key = list_use, "list text" = 3)
	plain_call(arg_use, named_arg = 1)
	. = ..() + .(self_arg) + ::global_use + ::global_call(global_arg)
	recv.field_use.method_use(method_arg)[index_use]?.safe_field
	/datum/path_use::static_use
	recv_two::proc_ref()
	x = cond_use ? then_use : else_use
	x += -unary_use
	x = new /datum/new_path{new_var = new_value}(new_arg)
	x = new mini.mini_field(mini_arg)
	x = new(implicit_arg)
	x = /obj/fab_path{fab_var = fab_value}
	x = "interp [interp_use] middle [interp_two] end"
	x = 'icons/res.dmi'
	x = alist(alist_key = alist_use)
	x = input(input_arg) as num in input_list
	x = locate(locate_arg) in locate_list
	x = pick(pick_weight; pick_one, pick_two)
	x = call(call_target, call_name)(call_arg)
	x = call_ext(ext_lib, ext_func)(ext_arg)
	x = __PROC__
	if(if_use)
		if_body()
	else if(elif_use)
		elif_body()
	else
		else_body()
	while(while_use)
		while_body()
	do
		do_body()
	while(do_use)
	for()
		forever_body()
	for(var/loop_var = loop_init, loop_test, loop_inc++)
		loop_body()
	for(var/mob/each_typed in each_list)
		each_body()
	for(var/key_var, value_var in pairs_list)
		pairs_body()
	for(var/range_var in range_from to range_to step range_step)
		range_body()
	spawn(spawn_delay)
		spawn_body()
	switch(switch_use)
		if(case_exact)
			case_body()
		if(case_from to case_to)
			range_case_body()
		else
			default_body()
	try
		try_body()
	catch(var/exception/caught)
		catch_body()
	a_label:
		for(var/labelled in label_list)
			if(labelled)
				continue a_label
			break a_label
	goto a_label
	throw throw_use
	del del_use
	CRASH(crash_use)
	return return_use
"#;

    fn printed_strings(text: &str) -> Vec<String> {
        let mut found = vec![];
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c != '"' {
                continue;
            }
            let mut string = String::from(c);
            loop {
                let c = chars.next().expect("an unfinished string");
                string.push(c);
                match c {
                    '\\' => string.push(chars.next().unwrap()),
                    '"' => break,
                    _ => {}
                }
            }
            found.push(string);
        }
        found
    }

    #[test]
    fn every_name_and_text_the_parser_holds_is_reached() {
        let tree = parsed(EVERYTHING);
        let value = proc_of(&tree, "everything");
        let mut reached: Vec<String> = vec![];
        let mut statements = 0;
        walk_proc(value, &mut |node| match node {
            Node::Name(name) => reached.push(name.to_owned()),
            Node::Term(Term::String(text)) => reached.push(text.to_string()),
            Node::Term(Term::InterpString(first, parts)) => {
                reached.push(first.to_string());
                reached.extend(parts.iter().map(|(_, text)| text.to_string()));
            }
            Node::Term(Term::Resource(name)) => reached.push(name.clone()),
            Node::Statement(_) => statements += 1,
            _ => {}
        });
        let mut printed = printed_strings(&format!("{:?}{:?}", value.code, value.parameters));
        let mut as_printed: Vec<String> = reached.iter().map(|s| format!("{:?}", s)).collect();
        as_printed.sort();
        printed.sort();
        assert_eq!(as_printed, printed);
        for name in [
            "par_typed",
            "living",
            "par_use",
            "set_name",
            "item",
            "var_two",
            "list_key",
            "named_arg",
            "self_arg",
            "global_use",
            "global_call",
            "safe_field",
            "method_arg",
            "index_use",
            "static_use",
            "proc_ref",
            "else_use",
            "unary_use",
            "new_path",
            "new_value",
            "mini_field",
            "implicit_arg",
            "fab_var",
            "interp_two",
            "alist_use",
            "input_list",
            "locate_list",
            "pick_weight",
            "call_name",
            "ext_lib",
            "elif_use",
            "else_body",
            "do_use",
            "forever_body",
            "loop_inc",
            "each_typed",
            "value_var",
            "range_step",
            "spawn_delay",
            "case_to",
            "default_body",
            "exception",
            "caught",
            "a_label",
            "throw_use",
            "del_use",
            "crash_use",
            "return_use",
        ] {
            assert!(reached.iter().any(|n| n == name), "{} isn't reached", name);
        }
        assert!(statements > 50, "{} statements", statements);
    }

    #[test]
    fn a_strings_text_is_not_a_name() {
        let tree = parsed(
            "/proc/f()\n\treturn \"LIMIT\" + \"is [x] LIMIT\" + 'LIMIT' + vars[\"LIMIT\"]\n",
        );
        let mut names = vec![];
        walk_proc(proc_of(&tree, "f"), &mut |node| {
            if let Node::Name(name) = node {
                names.push(name);
            }
        });
        assert_eq!(names, ["x", "vars"]);
    }

    #[test]
    fn a_node_comes_before_what_it_holds() {
        let tree = parsed("/proc/f()\n\treturn a.b(c)\n");
        let mut seen = vec![];
        walk_proc(proc_of(&tree, "f"), &mut |node| {
            seen.push(match node {
                Node::Parameter(_) => "parameter",
                Node::Statement(_) => "statement",
                Node::Expression(_) => "expression",
                Node::Term(_) => "term",
                Node::Follow(_) => "follow",
                Node::Name(name) => name,
            })
        });
        assert_eq!(
            seen,
            [
                "statement",
                "expression",
                "term",
                "a",
                "follow",
                "b",
                "expression",
                "term",
                "c"
            ]
        );
    }

    #[test]
    fn a_static_field_is_found_wherever_it_is() {
        for (code, has) in [
            ("/datum/a::x", true),
            ("f(1, list(g(/datum/a::x)))", true),
            ("a ? b : c[/datum/a::x]", true),
            ("\"is [/datum/a::x]\"", true),
            ("/obj{name = /datum/a::x}", true),
            ("new /obj(/datum/a::x)", true),
            ("pick(/datum/a::x; 2, 3)", true),
            ("a.b(c).d", false),
            ("/datum/a::x()", false),
            ("StaticField + \"StaticField(\\\"x\\\")\"", false),
        ] {
            let tree = parsed(&format!("/proc/f()\n\treturn {code}\n"));
            assert_eq!(has_static_field(returned_by_f(&tree)), has, "{}", code);
        }
    }

    #[test]
    fn constants_within_a_constant_are_reached() {
        let resource = |name: &'static str| Constant::Resource(name.into());
        let pop = |name: &'static str| {
            let mut vars = dreammaker::objtree::Vars::default();
            vars.insert("icon".into(), resource(name));
            Box::new(Pop {
                path: "/obj".into(),
                vars,
            })
        };
        let constant = Constant::List(Box::new([
            (resource("a.dmi"), None),
            (Constant::String("key".into()), Some(resource("b.dmi"))),
            (Constant::Prefab(pop("c.dmi")), None),
            (
                Constant::New {
                    type_: Some(pop("d.dmi")),
                    args: Some(Box::new([(resource("e.dmi"), None)])),
                },
                None,
            ),
            (
                Constant::Call(
                    dreammaker::constants::ConstFn::Icon,
                    Box::new([(resource("f.dmi"), None)]),
                ),
                None,
            ),
        ]));
        let mut names = vec![];
        walk_constant(&constant, &mut |constant| {
            if let Constant::Resource(name) = constant {
                names.push(name.as_str());
            }
        });
        assert_eq!(
            names,
            ["a.dmi", "b.dmi", "c.dmi", "d.dmi", "e.dmi", "f.dmi"]
        );
    }
}
