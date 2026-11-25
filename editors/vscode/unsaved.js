function pathOf(document) {
	return document.uri && document.uri.scheme === "file" ? document.uri.fsPath : undefined;
}

class UnsavedFiles {
	constructor() {
		this.paths = new Set();
	}

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

	all() {
		return [...this.paths].map((path) => ({ path, unsaved: true }));
	}
}

module.exports = { UnsavedFiles };
