"use strict";

const EXTENSION = "ss13.ss13buildflags";

function defines(text) {
	const found = [];
	let token = "";
	let quoted = false;
	for (const char of text) {
		if (char === '"') {
			quoted = !quoted;
		}
		if (!quoted && /\s/.test(char)) {
			if (token) {
				found.push(token);
			}
			token = "";
			continue;
		}
		token += char;
	}
	if (token) {
		found.push(token);
	}
	return found.filter((arg) => arg.startsWith("-D") && arg.length > 2).map((arg) => arg.slice(2));
}

async function selected(vscode) {
	if (!vscode.extensions.getExtension(EXTENSION)) {
		return undefined;
	}
	if (vscode.workspace.getConfiguration("ss13BuildFlags").get("injectionMode", "cli-args") !== "cli-args") {
		return undefined;
	}
	try {
		const current = await vscode.commands.executeCommand("ss13BuildFlags.current");
		return typeof current === "string" ? defines(current) : undefined;
	} catch {
		return undefined;
	}
}

async function api(vscode) {
	const extension = vscode.extensions.getExtension(EXTENSION);
	if (!extension) {
		return undefined;
	}
	try {
		const exports = extension.isActive ? extension.exports : await extension.activate();
		return exports && typeof exports.buildsWithFlags === "function" ? exports : undefined;
	} catch {
		return undefined;
	}
}

function launchFlags(flags, config) {
	return flags.buildsWithFlags(config.preLaunchTask) ? flags.current() : [];
}

module.exports = { api, defines, launchFlags, selected };
