"use strict";

const path = require("path");

const EXTENSION = "platymuus.dm-langclient";

function installed(vscode) {
	return Boolean(vscode.extensions.getExtension(EXTENSION));
}

async function environment(vscode, folder) {
	const extension = vscode.extensions.getExtension(EXTENSION);
	folder = folder || (vscode.workspace.workspaceFolders || [])[0];
	if (!extension || !folder) {
		return undefined;
	}
	try {
		await extension.activate();
		const dme = await vscode.commands.executeCommand("dreammaker.getFilenameDme");
		if (typeof dme !== "string" || !dme.endsWith(".dme")) {
			return undefined;
		}
		const full = path.join(folder.uri.fsPath, dme);
		return { dme: full, dmb: full.replace(/\.dme$/, ".dmb") };
	} catch {
		return undefined;
	}
}

module.exports = { installed, environment };
