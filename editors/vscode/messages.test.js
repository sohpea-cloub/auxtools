const assert = require("assert");
const fs = require("fs");
const path = require("path");

const cli = path.join(__dirname, "../../auxtools-cli/src");
const read = (directory, wanted) =>
	fs
		.readdirSync(directory)
		.filter(wanted)
		.map((name) => fs.readFileSync(path.join(directory, name), "utf8"));
const adapter = [fs.readFileSync(path.join(cli, "adapter.rs"), "utf8"), ...read(path.join(cli, "adapter"), (name) => name.endsWith(".rs"))].join("\n");
const status = fs.readFileSync(path.join(cli, "adapter/status.rs"), "utf8");
const extension = read(__dirname, (name) => name.endsWith(".js") && !name.endsWith(".test.js")).join("\n");

function names(text, pattern, what, where) {
	const found = [...new Set([...text.matchAll(pattern)].map((match) => match[1]))].sort();
	assert(found.length > 0, `found no ${what} in ${where}: this test looks for ${pattern}`);
	return found;
}
function same(what, launcher, editor) {
	const missing = (from, all) => all.filter((name) => !from.includes(name)).join(", ") || "none";
	assert.deepStrictEqual(
		editor,
		launcher,
		`${what} differ. Only in the adapter (auxtools-cli/src/adapter): ${missing(editor, launcher)}. Only in the extension (editors/vscode): ${missing(launcher, editor)}`
	);
	console.log(`ok: ${what} (${launcher.join(", ")})`);
}

same(
	"the custom requests",
	names(adapter, /"(auxtools\w+)"\s*=>/g, "request names", "the adapter"),
	names(extension, /\bcustomRequest\("(auxtools\w+)"/g, "request names", "the extension")
);

same(
	"the custom events",
	names(adapter, /\bconst EVENT: &'static str = "(auxtools\w+)"/g, "event names", "the adapter"),
	names(extension, /\bevent\.event === "(auxtools\w+)"/g, "event names", "the extension")
);

const [reported] = /\blet shown = [^;]*;/.exec(status) || [""];
const states = [...names(adapter, /\breload_state\("(\w+)"/g, "reload states", "the adapter"), ...names(reported, /"(\w+)"/g, "reload states", "reload_state's `let shown`")];
const [table] = /^const STATES = \{$.*?^\};$/ms.exec(extension) || [""];
const labelled = names(table, /^\t(\w+): \{/gm, "reload states", "the extension's STATES");
const derived = names(extension, /\bstate = "(\w+)"/g, "states of its own", "the extension");
for (const state of derived) {
	assert(labelled.includes(state), `the extension sets the reload state ${state}, which its STATES has no label for (it has ${labelled.join(", ")})`);
}
same(
	"the reload states",
	[...new Set(states)].sort(),
	labelled.filter((state) => !derived.includes(state))
);
for (const state of names(extension, /\bstate === "(\w+)"/g, "compared reload states", "the extension")) {
	assert(states.includes(state), `the extension compares a reload state with ${state}, which the adapter never sends (it sends ${states.join(", ")})`);
}

const waits = names(status, /\bUntil::\w+ => "(\w+)"/g, "`until` names", "adapter/status.rs");
for (const until of names(extension, /\buntil === "(\w+)"/g, "compared `until` names", "the extension")) {
	assert(waits.includes(until), `the extension compares a held change's \`until\` with ${until}, which the adapter never sends (it sends ${waits.join(", ")})`);
}
console.log(`ok: what a held change waits for (${waits.join(", ")})`);
