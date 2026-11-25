const assert = require("assert");
const { evaluatableExpression, expressionAt } = require("./expression");

function at(marked) {
	const column = marked.indexOf("|");
	const line = marked.replace("|", "");
	const found = expressionAt(line, column);
	return found && line.slice(found.start, found.end);
}

assert.strictEqual(at("\tvar/y = |x + 1"), "x");
assert.strictEqual(at("\tvar/y = x| + 1"), "x");
assert.strictEqual(at("\treturn he|alth * 2"), "health");
assert.strictEqual(at("\tsrc.na|me = called"), "src.name");
assert.strictEqual(at("\ts|rc.name = called"), "src");
assert.strictEqual(at("\tmob.loc.conte|nts.len"), "mob.loc.contents");
assert.strictEqual(at("\tmob.loc.contents.l|en"), "mob.loc.contents.len");
assert.strictEqual(at("\tif (target?.cli|ent)"), "target?.client");
assert.strictEqual(at("\tthing:hid|den = 1"), "thing:hidden");
assert.strictEqual(at("\tthing?:hid|den = 1"), "thing?:hidden");
assert.strictEqual(at("\ta . b . |c"), "a . b . c");
assert.strictEqual(at("\tget_mob().na|me"), undefined);
assert.strictEqual(at("\titems[1].na|me"), "items[1].name");
assert.strictEqual(at("\tx = src.ite|ms[i]"), "src.items");
assert.strictEqual(at("\tx = src.items[|i]"), "i");
assert.strictEqual(at("\tx = src.items[i|]"), "src.items[i]");
assert.strictEqual(at("\tx = items[i].na|me"), "items[i].name");
assert.strictEqual(at("\tx = items?[1]?.na|me"), "items?[1]?.name");
assert.strictEqual(at('\tx = items["slot (left)"].na|me'), 'items["slot (left)"].name');
assert.strictEqual(at("\tx = a[b[1]][2].|c"), "a[b[1]][2].c");
assert.strictEqual(at("\tx = a[b[1]|]"), "a[b[1]]");
assert.strictEqual(at("\tx = list(1)[1].na|me"), undefined);
assert.strictEqual(at('\tworld.log << "a[1|]"'), undefined);
assert.strictEqual(at("\tsrc.bu|mp(2)"), undefined);
assert.strictEqual(at("\tle|af(x)"), undefined);
assert.strictEqual(at("\tleaf(|x)"), "x");
assert.strictEqual(at("/proc/le|af(x)"), undefined);
assert.strictEqual(at('\tworld.log << "dap: wor|ld"'), undefined);
assert.strictEqual(at('\tworld.log << "a \\" b|" + c'), undefined);
assert.strictEqual(at('\tworld.log << "a" + |c'), "c");
assert.strictEqual(at("\tx = 1 // then |y"), undefined);
assert.strictEqual(at("\tret|urn x"), undefined);
assert.strictEqual(at("\tvar/|y = 1"), undefined);
assert.strictEqual(at("\tx = 1e|3"), undefined);
assert.strictEqual(at("\tnew /datum/th|ing(x)"), undefined);
assert.strictEqual(at("\tvar/datum/thing/|t = x"), undefined);
assert.strictEqual(at("\treturn a ? b : |c"), "c");
assert.strictEqual(at("\treturn a ? b :|c"), "c");
assert.strictEqual(at("\tx = | 1"), undefined);
assert.strictEqual(at("|"), undefined);
const vscode = {
	Range: class {
		constructor(line, start, endLine, end) {
			Object.assign(this, { line, start, endLine, end });
		}
	},
	EvaluatableExpression: class {
		constructor(range) {
			this.range = range;
		}
	},
};
const document = { lineAt: (line) => ({ text: ["", "\tvar/n = src.items[i].na"][line] }) };
assert.deepStrictEqual({ ...evaluatableExpression(vscode, document, { line: 1, character: 23 }).range }, {
	line: 1,
	start: 9,
	endLine: 1,
	end: 24,
});
assert.deepStrictEqual({ ...evaluatableExpression(vscode, document, { line: 1, character: 20 }).range }, {
	line: 1,
	start: 9,
	endLine: 1,
	end: 21,
});
assert.strictEqual(evaluatableExpression(vscode, document, { line: 1, character: 3 }), undefined);
console.log("ok: 43 hover expressions");
