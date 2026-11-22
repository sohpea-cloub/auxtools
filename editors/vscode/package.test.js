"use strict";

const assert = require("assert");
const fs = require("fs");
const Module = require("module");
const path = require("path");

if (process.argv.length !== 3) {
	console.error("usage: node package.test.js DIRECTORY (an unpacked .vsix)");
	process.exit(2);
}
const unpacked = path.resolve(process.argv[2]);
const root = path.join(unpacked, "extension");
const inPackage = (file) => !path.relative(root, file).startsWith("..");
const packaged = (name) => fs.existsSync(path.join(root, name)) && inPackage(path.join(root, name));
const sorted = (names) => [...new Set(names)].sort();

const files = fs
	.readdirSync(root, { recursive: true, withFileTypes: true })
	.filter((entry) => entry.isFile())
	.map((entry) => path.relative(root, path.join(entry.parentPath || entry.path, entry.name)).split(path.sep).join("/"));
const scripts = files.filter((name) => name.endsWith(".js") && !name.startsWith("bin/") && !name.includes("node_modules/"));
const source = new Map(scripts.map((name) => [name, fs.readFileSync(path.join(root, name), "utf8")]));
const captures = (pattern) => sorted([...source.values()].flatMap((text) => [...text.matchAll(pattern)].map((match) => match[1])));

const vsix = fs.readFileSync(path.join(unpacked, "extension.vsixmanifest"), "utf8");
assert(fs.existsSync(path.join(unpacked, "[Content_Types].xml")), "the package has no [Content_Types].xml");
for (const [, asset] of vsix.matchAll(/<Asset [^>]*Path="([^"]+)"/g)) {
	assert(fs.existsSync(path.join(unpacked, asset)), `extension.vsixmanifest names ${asset}, which is not in the package`);
}
const manifest = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const contributes = manifest.contributes || {};
const each = (list) => list || [];
const icons = (icon) => (typeof icon === "string" ? (icon.startsWith("$(") ? [] : [icon]) : icon ? [icon.light, icon.dark] : []);
const named = [
	["icon", manifest.icon],
	["browser", manifest.browser],
	...each(contributes.commands).flatMap((command) => icons(command.icon).map((icon) => [`the icon of ${command.command}`, icon])),
	...each(contributes.languages).flatMap((language) => [
		[`the configuration of ${language.id}`, language.configuration],
		...icons(language.icon).map((icon) => [`the icon of ${language.id}`, icon]),
	]),
	...each(contributes.grammars).map((grammar) => [`the grammar of ${grammar.scopeName}`, grammar.path]),
	...each(contributes.snippets).map((snippets) => [`the snippets of ${snippets.language}`, snippets.path]),
	...each(contributes.debuggers).map((debuggers) => [`the program of ${debuggers.type}`, debuggers.program]),
	...each(contributes.walkthroughs).flatMap((walkthrough) =>
		each(walkthrough.steps).flatMap((step) =>
			["image", "markdown", "svg"].map((kind) => [`the ${kind} of step ${step.id}`, step.media && typeof step.media[kind] === "string" ? step.media[kind] : undefined])
		)
	),
];
for (const [what, file] of named) {
	assert(file === undefined || packaged(file), `package.json names ${file} as ${what}, which is not in the package`);
}
assert(typeof manifest.main === "string", "package.json has no main");
let main;
try {
	main = require.resolve(path.join(root, manifest.main));
} catch (e) {
	assert.fail(`package.json's main is ${manifest.main}, which is not in the package`);
}
assert(inPackage(main), `package.json's main is ${manifest.main}, which is outside the package`);
console.log("ok: the files the manifests name are packaged");

for (const [name, text] of source) {
	for (const [, wanted] of text.matchAll(/\brequire\(\s*["']([^"']+)["']\s*\)/g)) {
		if (wanted === "vscode" || Module.isBuiltin(wanted)) {
			continue;
		}
		let found;
		try {
			found = Module.createRequire(path.join(root, name)).resolve(wanted);
		} catch (e) {
			assert.fail(`${name} requires ${wanted}, which is not in the package (it has ${files.join(", ")})`);
		}
		assert(inPackage(found), `${name} requires ${wanted}, which is ${found}, outside the package`);
	}
}
console.log(`ok: the scripts require packaged files (${scripts.join(", ")})`);

function standIn(known = {}) {
	return new Proxy(function () {}, {
		get: (target, name) => (name in known ? known[name] : name === "then" || typeof name === "symbol" ? undefined : (known[name] = standIn())),
		set: (target, name, value) => ((known[name] = value), true),
		apply: () => standIn(),
		construct: () => standIn(),
	});
}
const properties = (contributes.configuration || {}).properties || {};
const commands = [];
const adapters = [];
const vscode = standIn({
	commands: standIn({ registerCommand: (name) => commands.push(name) }),
	debug: standIn({ registerDebugAdapterDescriptorFactory: (type, factory) => adapters.push([type, factory]) }),
	DebugAdapterExecutable: class {
		constructor(command, args) {
			Object.assign(this, { command, args });
		}
	},
	extensions: standIn({ getExtension: () => undefined }),
	workspace: standIn({
		textDocuments: [],
		workspaceFolders: undefined,
		getConfiguration: (section) => ({
			get(name) {
				assert(`${section}.${name}` in properties, `the extension reads the setting ${section}.${name}, which package.json does not declare`);
				return properties[`${section}.${name}`].default;
			},
		}),
	}),
});
const load = Module._load;
Module._load = function (request, ...rest) {
	return request === "vscode" ? vscode : load.call(this, request, ...rest);
};

async function activated() {
	const extension = require(main);
	extension.activate({ extensionPath: root, subscriptions: [], environmentVariableCollection: standIn() });
	await new Promise((resolve) => setTimeout(resolve, 50));
	console.log("ok: the packaged extension activates");

	const declared = each(contributes.commands).map((command) => command.command);
	assert.deepStrictEqual(sorted(commands), sorted(declared), "the commands the extension registers (first) are not those package.json declares (second)");
	assert.strictEqual(commands.length, sorted(commands).length, `a command is registered twice: ${commands.join(", ")}`);
	const referred = [
		...Object.values(contributes.menus || {}).flatMap((menu) => menu.map((item) => item.command)),
		...each(manifest.activationEvents).filter((event) => event.startsWith("onCommand:")).map((event) => event.slice("onCommand:".length)),
		...[...JSON.stringify(manifest).matchAll(/\$\{command:(auxtools\.\w+)\}/g)].map((match) => match[1]),
	];
	for (const command of referred) {
		assert(declared.includes(command), `package.json refers to the command ${command}, which it does not declare`);
	}
	console.log(`ok: the declared commands are registered (${declared.length})`);

	const settings = Object.keys(properties);
	const read = captures(/\bsettings\(\)\.get\("([^"]+)"\)/g).map((name) => `auxtools.${name}`);
	assert.deepStrictEqual(read, sorted(settings), "the settings the extension reads (first) are not those package.json declares (second)");
	const kinds = Object.keys(properties["auxtools.notifications"].properties);
	assert.deepStrictEqual(captures(/\bnotifies\("([^"]+)"\)/g), sorted(kinds), "the notification categories the extension asks about (first) are not those of auxtools.notifications (second)");
	for (const name of captures(/\baffectsConfiguration\("([^"]+)"\)/g)) {
		assert(settings.includes(name), `the extension watches the setting ${name}, which package.json does not declare`);
	}
	for (const name of captures(/\b(?:command = |executeCommand\()"(auxtools\.[^"]+)"/g)) {
		assert(declared.includes(name), `the extension uses the command ${name}, which package.json does not declare`);
	}
	console.log(`ok: the declared settings are read (${settings.length})`);

	const [[, factory]] = adapters;
	const platforms = fs.existsSync(path.join(root, "bin")) ? fs.readdirSync(path.join(root, "bin")).sort() : [];
	assert(platforms.length > 0, "the package has no bin/PLATFORM directory");
	const real = Object.getOwnPropertyDescriptor(process, "platform");
	for (const platform of platforms) {
		Object.defineProperty(process, "platform", { ...real, value: platform });
		const adapter = factory.createDebugAdapterDescriptor();
		Object.defineProperty(process, "platform", real);
		assert(adapter && adapter.command, `on ${platform} the extension finds no launcher in the package (bin/${platform} has ${fs.readdirSync(path.join(root, "bin", platform)).join(", ")})`);
		assert(inPackage(adapter.command), `on ${platform} the launcher is ${adapter.command}, outside the package`);
	}
	console.log(`ok: the launchers are packaged (${platforms.join(", ")})`);
}

activated().catch((e) => {
	console.error(e);
	process.exit(1);
});
