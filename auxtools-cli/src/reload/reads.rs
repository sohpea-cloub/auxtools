use auxtools_protocol::ParsedReads;
use dmasm::syntax::{walk_expression, Node};
use dreammaker::ast::{Case, Expression, Follow, Parameter, Spanned, Statement, Term, VarStatement};

pub fn parsed_reads(params: &[Parameter], block: &[Spanned<Statement>], name: &str) -> Option<ParsedReads> {
	let mut scan = Scan {
		name,
		visible: vec![],
		statics: 0,
		seen: None
	};
	for param in params {
		if let Some(default) = &param.default {
			scan.expression(default);
		}
		if param.name.as_str() == name {
			scan.visible.push(Visible::Other);
		}
	}
	scan.block(block);
	scan.seen
}

#[derive(Clone, Copy)]
enum Visible {
	Local(u32),
	Other
}

struct Scan<'n> {
	name: &'n str,
	visible: Vec<Visible>,
	statics: u32,
	seen: Option<ParsedReads>
}

impl Scan<'_> {
	const fn note(&mut self, read: Option<u32>) {
		self.seen = Some(match (self.seen, read) {
			(None, Some(nth)) => ParsedReads::Only(nth),
			(Some(ParsedReads::Only(had)), Some(nth)) if had == nth => ParsedReads::Only(nth),
			_ => ParsedReads::Unknown
		});
	}

	fn bare(&mut self) {
		match self.visible.last() {
			Some(Visible::Local(nth)) => self.note(Some(*nth)),
			Some(Visible::Other) => {}
			None => self.note(None)
		}
	}

	fn block(&mut self, block: &[Spanned<Statement>]) {
		let mark = self.visible.len();
		for statement in block {
			self.statement(&statement.elem);
		}
		self.visible.truncate(mark);
	}

	fn declare(&mut self, var: &VarStatement) {
		if let Some(value) = &var.value {
			self.expression(value);
		}
		if var.name.as_str() != self.name {
			return;
		}
		if var.var_type.flags.is_static() || var.var_type.flags.is_const() {
			self.visible.push(Visible::Local(self.statics));
			self.statics += 1;
		} else {
			self.visible.push(Visible::Other);
		}
	}

	fn loop_var(&mut self, name: &str, declared: bool) {
		if name != self.name {
			return;
		}
		if declared {
			self.visible.push(Visible::Other);
		} else {
			self.bare();
		}
	}

	fn statement(&mut self, statement: &Statement) {
		match statement {
			Statement::Expr(value) | Statement::Throw(value) | Statement::Del(value) => self.expression(value),
			Statement::Return(value) | Statement::Crash(value) => {
				if let Some(value) = value {
					self.expression(value);
				}
			}
			Statement::While { condition, block } => {
				self.expression(&condition.elem);
				self.block(block);
			}
			Statement::DoWhile { block, condition } => {
				self.block(block);
				self.expression(&condition.elem);
			}
			Statement::If { arms, else_arm } => {
				for (condition, block) in arms {
					self.expression(&condition.elem);
					self.block(block);
				}
				if let Some(block) = else_arm {
					self.block(block);
				}
			}
			Statement::ForInfinite { block } | Statement::Spawn { block, delay: None } | Statement::Label { block, .. } => self.block(block),
			Statement::Spawn { delay: Some(delay), block } => {
				self.expression(delay);
				self.block(block);
			}
			Statement::ForLoop { init, test, inc, block } => {
				let mark = self.visible.len();
				if let Some(init) = init {
					self.statement(init);
				}
				if let Some(test) = test {
					self.expression(test);
				}
				if let Some(inc) = inc {
					self.statement(inc);
				}
				self.block(block);
				self.visible.truncate(mark);
			}
			Statement::ForList(each) => {
				if let Some(list) = &each.in_list {
					self.expression(list);
				}
				let mark = self.visible.len();
				self.loop_var(each.name.as_str(), each.var_type.is_some());
				self.block(&each.block);
				self.visible.truncate(mark);
			}
			Statement::ForKeyValue(each) => {
				if let Some(list) = &each.in_list {
					self.expression(list);
				}
				let mark = self.visible.len();
				self.loop_var(each.key.as_str(), each.var_type.is_some());
				self.loop_var(each.value.as_str(), each.var_type.is_some());
				self.block(&each.block);
				self.visible.truncate(mark);
			}
			Statement::ForRange(each) => {
				self.expression(&each.start);
				self.expression(&each.end);
				if let Some(step) = &each.step {
					self.expression(step);
				}
				let mark = self.visible.len();
				self.loop_var(each.name.as_str(), each.var_type.is_some());
				self.block(&each.block);
				self.visible.truncate(mark);
			}
			Statement::Var(var) => self.declare(var),
			Statement::Vars(vars) => vars.iter().for_each(|var| self.declare(var)),
			Statement::Setting { value, .. } => self.expression(value),
			Statement::Switch { input, cases, default } => {
				self.expression(input);
				for (values, block) in cases.iter() {
					for case in &values.elem {
						match case {
							Case::Exact(value) => self.expression(value),
							Case::Range(from, to) => {
								self.expression(from);
								self.expression(to);
							}
						}
					}
					self.block(block);
				}
				if let Some(block) = default {
					self.block(block);
				}
			}
			Statement::TryCatch {
				try_block,
				catch_params,
				catch_block
			} => {
				self.block(try_block);
				let mark = self.visible.len();
				if catch_params
					.iter()
					.any(|path| path.iter().last().is_some_and(|last| last.as_str() == self.name))
				{
					self.visible.push(Visible::Other);
				}
				self.block(catch_block);
				self.visible.truncate(mark);
			}
			Statement::Continue(_) | Statement::Break(_) | Statement::Goto(_) => {}
		}
	}

	fn expression(&mut self, expression: &Expression) {
		let name = self.name;
		walk_expression(expression, &mut |node| match node {
			Node::Term(Term::Ident(used)) if used.as_str() == name => self.bare(),
			Node::Term(Term::GlobalIdent(used)) if used.as_str() == name => self.note(None),
			Node::Term(Term::NewMiniExpr { expr, .. }) => {
				if expr.ident.as_str() == name {
					self.bare();
				}
				if expr.fields.iter().any(|field| field.ident.as_str() == name) {
					self.note(None);
				}
			}
			Node::Follow(Follow::Field(_, used) | Follow::StaticField(used)) if used.as_str() == name => self.note(None),
			_ => {}
		});
	}
}

#[cfg(test)]
mod tests {
	use std::sync::atomic::{AtomicU32, Ordering};

	use auxtools_protocol::ParsedReads;
	use dmasm::reload::{ParseOptions, Snapshot};

	use super::parsed_reads;

	fn reads(body: &str, name: &str) -> Option<ParsedReads> {
		reads_with("x", body, name)
	}

	fn reads_with(params: &str, body: &str, name: &str) -> Option<ParsedReads> {
		static PARSED: AtomicU32 = AtomicU32::new(0);
		let dir = std::env::temp_dir().join(format!(
			"auxtools-locals-{}-{}",
			std::process::id(),
			PARSED.fetch_add(1, Ordering::Relaxed)
		));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("world.dme"), "#include \"code.dm\"\n").unwrap();
		std::fs::write(
			dir.join("code.dm"),
			format!("var/{name} = 1\n/datum/a/var/{name} = 2\n/datum/a/proc/f({params})\n{body}")
		)
		.unwrap();
		let snapshot = Snapshot::parse(&dir.join("world.dme"), &ParseOptions::default()).unwrap();
		let _ = std::fs::remove_dir_all(&dir);
		let key = snapshot.procs().find(|key| key.path.ends_with("/f")).unwrap().clone();
		parsed_reads(snapshot.proc_parameters(&key).unwrap(), snapshot.proc_code(&key).unwrap(), name)
	}

	#[test]
	fn a_body_that_reads_a_name_only_as_its_local_reads_that_local() {
		assert_eq!(reads("\tvar/static/n = 0\n\tn++\n\treturn n\n", "n"), Some(ParsedReads::Only(0)));
		let branches = "\tif (x)\n\t\tvar/static/n = 0\n\t\treturn n\n\tvar/const/n = 2\n\treturn \"[n]\"\n";
		assert_eq!(reads(branches, "n"), Some(ParsedReads::Unknown));
		let second = "\tif (x)\n\t\tvar/static/n = 0\n\tvar/const/n = 2\n\treturn n\n";
		assert_eq!(reads(second, "n"), Some(ParsedReads::Only(1)));
		assert_eq!(reads("\tvar/static/n = 0\n\treturn 1\n", "n"), None);
	}

	#[test]
	fn a_body_that_may_read_another_var_of_the_name_reads_it_unknown() {
		for body in [
			"\tvar/static/n = 0\n\treturn global.n\n",
			"\tvar/static/n = 0\n\treturn src.n\n",
			"\tvar/static/n = 0\n\treturn x:n\n",
			"\tvar/static/n = 0\n\treturn /datum/a::n\n",
			"\treturn n\n\tvar/static/n = 0\n",
			"\tif (x)\n\t\tvar/static/n = 0\n\treturn n\n"
		] {
			assert_eq!(reads(body, "n"), Some(ParsedReads::Unknown), "{}", body);
		}
	}

	#[test]
	fn a_parameter_default_reads_a_name_outside_the_body() {
		let body = "\tvar/const/n = 5\n\treturn n + 1\n";
		assert_eq!(reads_with("x = global.n", body, "n"), Some(ParsedReads::Unknown));
		assert_eq!(reads_with("x = n", body, "n"), Some(ParsedReads::Unknown));
		assert_eq!(reads_with("x = 2", body, "n"), Some(ParsedReads::Only(0)));
	}

	#[test]
	fn an_ordinary_local_hides_a_static_of_its_name() {
		let body = "\tvar/static/n = 0\n\tn++\n\tfor (var/n in list(1))\n\t\tx += n\n\tif (x)\n\t\tvar/n = 3\n\t\treturn n\n\treturn n\n";
		assert_eq!(reads(body, "n"), Some(ParsedReads::Only(0)));
	}
}
