// The auxtools debugger for VS Code. The debugging is all the launcher's
// (`tools/auxtools-cli`): this tells VS Code where it is, fills in launch and
// attach configurations, and adds commands and a build task for DreamMaker
// worlds.
//
// Plain JavaScript with no dependencies: VS Code provides the `vscode` module.

"use strict";

const childProcess = require("child_process");
const fs = require("fs");
const path = require("path");
const vscode = require("vscode");

const DEBUG_TYPE = "auxtools";
const TASK_TYPE = "dreammaker";

function settings() {
	return vscode.workspace.getConfiguration("auxtools");
}

// The launcher for this platform, from the settings or the extension's own
function launcherPath(context) {
	const configured = settings().get("launcherPath");
	if (configured) {
		return configured;
	}
	const exe = process.platform === "win32" ? "auxtools.exe" : "auxtools";
	return path.join(context.extensionPath, "bin", process.platform, exe);
}

// The launcher's arguments for building `dme`
function buildArgs(dme) {
	const byondPath = settings().get("byondPath");
	return byondPath ? ["--build", dme, byondPath] : ["--build", dme];
}

// A packaged extension's files can lose their executable bit on Linux
function ensureExecutable(file) {
	if (process.platform === "win32") {
		return;
	}
	try {
		fs.accessSync(file, fs.constants.X_OK);
	} catch {
		try {
			fs.chmodSync(file, 0o755);
		} catch {
			// Running it says what's wrong
		}
	}
}

// The workspace's files with `extension` (".dme" or ".dmb"), most likely
// first: one named after its folder, then shallower ones
async function findFiles(extension, folder) {
	const glob = `**/*${extension}`;
	const pattern = folder ? new vscode.RelativePattern(folder, glob) : glob;
	const found = await vscode.workspace.findFiles(pattern, "**/node_modules/**", 50);
	const score = (uri) => {
		const owner = vscode.workspace.getWorkspaceFolder(uri);
		const named = owner && path.basename(uri.fsPath, extension) === owner.name ? 0 : 1;
		return [named, uri.fsPath.split(/[\\/]/).length];
	};
	return found.sort((a, b) => {
		const [x, y] = [score(a), score(b)];
		return x[0] - y[0] || x[1] - y[1] || a.fsPath.localeCompare(b.fsPath);
	});
}

// One built world (.dmb) to run, asking when there's a choice
async function pickWorldFile(folder) {
	const found = await findFiles(".dmb", folder);
	if (found.length === 0) {
		vscode.window.showErrorMessage("There's no .dmb in the workspace to run. Build the world first (Tasks: Run Build Task).");
		return undefined;
	}
	if (found.length === 1) {
		return found[0];
	}
	const picked = await vscode.window.showQuickPick(
		found.map((uri) => ({ label: vscode.workspace.asRelativePath(uri), uri })),
		{ placeHolder: "Which world?" }
	);
	return picked && picked.uri;
}

// The running worlds that can be attached to, from `auxtools --list`
function listWorlds(context) {
	const launcher = launcherPath(context);
	ensureExecutable(launcher);
	return new Promise((resolve, reject) => {
		childProcess.execFile(launcher, ["--list"], { timeout: 30000 }, (error, stdout) => {
			if (error) {
				reject(error);
				return;
			}
			try {
				resolve(JSON.parse(stdout));
			} catch (e) {
				reject(e);
			}
		});
	});
}

// One running world's process id, asking when there's a choice
async function pickWorld(context) {
	let worlds;
	try {
		worlds = await listWorlds(context);
	} catch (e) {
		vscode.window.showErrorMessage(`Couldn't list the running worlds: ${e.message}`);
		return undefined;
	}
	if (worlds.length === 0) {
		vscode.window.showErrorMessage("There's no world running auxtools' debugger. Worlds started by its launcher can be attached to.");
		return undefined;
	}
	const items = worlds.map((world) => ({
		label: world.dmb ? path.basename(world.dmb) : world.program,
		description: `process ${world.pid}`,
		detail: world.dmb || world.args.join(" "),
		pid: String(world.pid),
	}));
	if (items.length === 1) {
		return items[0].pid;
	}
	const picked = await vscode.window.showQuickPick(items, { placeHolder: "Which world?" });
	return picked && picked.pid;
}

function defaultServer() {
	return process.platform === "win32" ? "dreamseeker" : "dreamdaemon";
}

// A launch configuration for a .dmb. Its .dme is the one next to it.
function launchConfiguration(dmb, server, name) {
	return {
		type: DEBUG_TYPE,
		request: "launch",
		name: name || `Debug ${path.basename(dmb)}`,
		dmb,
		server: server || defaultServer(),
	};
}

class ConfigurationProvider {
	constructor(context) {
		this.context = context;
	}

	async provideDebugConfigurations(folder) {
		const found = await findFiles(".dmb", folder);
		const dmb = found.length > 0 ? found[0].fsPath : "${workspaceFolder}/${workspaceFolderBasename}.dmb";
		const servers = process.platform === "win32" ? ["dreamseeker", "dreamdaemon"] : ["dreamdaemon"];
		return servers.map((server) => launchConfiguration(dmb, server, `Debug in ${server === "dreamseeker" ? "DreamSeeker" : "DreamDaemon"}`));
	}

	// Fills in what a configuration leaves out, including all of it when
	// there's no launch.json
	async resolveDebugConfiguration(folder, config) {
		if (!config.type && !config.request && !config.name) {
			const dmb = await pickWorldFile(folder);
			if (!dmb) {
				return undefined;
			}
			config = launchConfiguration(dmb.fsPath);
		}
		if (config.request === "attach" && !config.processId) {
			config.processId = await pickWorld(this.context);
			if (!config.processId) {
				return undefined;
			}
		}
		if (!config.byondPath && settings().get("byondPath")) {
			config.byondPath = settings().get("byondPath");
		}
		if (!config.debugLibrary && settings().get("debugLibrary")) {
			config.debugLibrary = settings().get("debugLibrary");
		}
		return config;
	}
}

class AdapterFactory {
	constructor(context) {
		this.context = context;
	}

	createDebugAdapterDescriptor() {
		const launcher = launcherPath(this.context);
		if (!fs.existsSync(launcher)) {
			vscode.window.showErrorMessage(`There's no auxtools launcher at ${launcher}.`);
			return undefined;
		}
		ensureExecutable(launcher);
		return new vscode.DebugAdapterExecutable(launcher, []);
	}
}

class BuildTaskProvider {
	constructor(context) {
		this.context = context;
	}

	async provideTasks() {
		const found = await findFiles(".dme");
		return found.map((uri) => this.task({ type: TASK_TYPE, dme: uri.fsPath }, vscode.workspace.getWorkspaceFolder(uri)));
	}

	resolveTask(task) {
		if (!task.definition.dme) {
			return undefined;
		}
		return this.task(task.definition, task.scope);
	}

	task(definition, folder) {
		ensureExecutable(launcherPath(this.context));
		const task = new vscode.Task(
			definition,
			folder || vscode.TaskScope.Workspace,
			`build ${path.basename(definition.dme)}`,
			TASK_TYPE,
			new vscode.ProcessExecution(launcherPath(this.context), buildArgs(definition.dme), {
				cwd: path.dirname(definition.dme),
			}),
			"$dreammaker"
		);
		task.group = vscode.TaskGroup.Build;
		return task;
	}
}

// Attaches to a running world, picked from a list
async function attach() {
	await vscode.debug.startDebugging(undefined, {
		type: DEBUG_TYPE,
		request: "attach",
		name: "Attach to a world",
	});
}

// A command that starts a world from one of the workspace's .dmb files
function startCommand(server, noDebug) {
	return async () => {
		const editor = vscode.window.activeTextEditor;
		const folder = editor ? vscode.workspace.getWorkspaceFolder(editor.document.uri) : undefined;
		const dmb = await pickWorldFile(folder);
		if (!dmb) {
			return;
		}
		const name = `${noDebug ? "Run" : "Debug"} ${path.basename(dmb.fsPath)}`;
		await vscode.debug.startDebugging(vscode.workspace.getWorkspaceFolder(dmb), launchConfiguration(dmb.fsPath, server, name), { noDebug });
	};
}

function activate(context) {
	const provider = new ConfigurationProvider(context);
	context.subscriptions.push(
		vscode.debug.registerDebugAdapterDescriptorFactory(DEBUG_TYPE, new AdapterFactory(context)),
		vscode.debug.registerDebugConfigurationProvider(DEBUG_TYPE, provider),
		vscode.debug.registerDebugConfigurationProvider(DEBUG_TYPE, provider, vscode.DebugConfigurationProviderTriggerKind.Dynamic),
		vscode.tasks.registerTaskProvider(TASK_TYPE, new BuildTaskProvider(context)),
		vscode.commands.registerCommand("auxtools.debugDreamSeeker", startCommand("dreamseeker", false)),
		vscode.commands.registerCommand("auxtools.debugDreamDaemon", startCommand("dreamdaemon", false)),
		vscode.commands.registerCommand("auxtools.runDreamSeeker", startCommand("dreamseeker", true)),
		vscode.commands.registerCommand("auxtools.runDreamDaemon", startCommand("dreamdaemon", true)),
		vscode.commands.registerCommand("auxtools.attach", attach),
		// For `${command:auxtools.pickWorld}` in attach configurations
		vscode.commands.registerCommand("auxtools.pickWorld", () => pickWorld(context))
	);
}

function deactivate() {}

module.exports = { activate, deactivate };
