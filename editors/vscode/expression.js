const NAME = /[A-Za-z_][A-Za-z0-9_]*/y;

function nameAt(line, column) {
	let start = column;
	while (start > 0 && /[A-Za-z0-9_]/.test(line[start - 1])) {
		start--;
	}
	NAME.lastIndex = start;
	const match = NAME.exec(line);
	if (!match || start + match[0].length < column) {
		return undefined;
	}
	return [start, start + match[0].length];
}

function inTextOrComment(line, position) {
	let quoted = false;
	for (let i = 0; i < position; i++) {
		const c = line[i];
		if (quoted) {
			if (c === "\\") {
				i++;
			} else if (c === '"') {
				quoted = false;
			}
		} else if (c === '"') {
			quoted = true;
		} else if (c === "/" && line[i + 1] === "/") {
			return true;
		}
	}
	return quoted;
}

function expressionAt(line, column) {
	const name = nameAt(line, column);
	if (!name || inTextOrComment(line, name[0])) {
		return undefined;
	}
	let [start, end] = name;

	if (/^\s*\(/.test(line.slice(end))) {
		return undefined;
	}
	const word = line.slice(start, end);
	if (/^(var|proc|verb|if|else|for|while|do|switch|return|new|del|in|to|as|set|try|catch|throw|goto|break|continue|spawn|step|null|global|static|const|tmp)$/.test(word)) {
		return undefined;
	}
	if (start > 1 && line[start - 1] === "." && /[0-9]/.test(line[start - 2])) {
		return undefined;
	}

	for (;;) {
		const before = /([A-Za-z_][A-Za-z0-9_]*)\s*(\?\.|\?:|\.|:)\s*$/.exec(line.slice(0, start));
		if (!before || (/:$/.test(before[2]) && /\s:\s*$/.test(line.slice(0, start)))) {
			break;
		}
		start = before.index;
	}
	const through = /(\?\.|\?:|\.|:)\s*$/.exec(line.slice(0, start));
	if (through && !(through[1] === ":" && /\s:\s*$/.test(line.slice(0, start)))) {
		return undefined;
	}
	if (start > 0 && line[start - 1] === "/") {
		return undefined;
	}
	return { start, end };
}

module.exports = { expressionAt };
