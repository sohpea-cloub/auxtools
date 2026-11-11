// Which files have edits that aren't saved, for the launcher. Nothing in the
// Debug Adapter Protocol says: VS Code gives a breakpoint added to such a file
// on a line of the editor's text, which the file, and the world's code,
// don't have yet. The launcher keeps those until the file is saved.

// The files a session is told of: those on disk
function pathOf(document) {
	return document.uri && document.uri.scheme === "file" ? document.uri.fsPath : undefined;
}

class UnsavedFiles {
	constructor() {
		this.paths = new Set();
	}

	// What to tell the sessions of a document whose state may have changed,
	// as `{ path, unsaved }`, or undefined when there's nothing new
	changed(document) {
		const path = pathOf(document);
		if (path === undefined) {
			return undefined;
		}
		const unsaved = document.isDirty && !document.isClosed;
		if (unsaved === this.paths.has(path)) {
			return undefined;
		}
		if (unsaved) {
			this.paths.add(path);
		} else {
			this.paths.delete(path);
		}
		return { path, unsaved };
	}

	// What a session that starts is told: every file that's unsaved
	all() {
		return [...this.paths].map((path) => ({ path, unsaved: true }));
	}
}

module.exports = { UnsavedFiles };
