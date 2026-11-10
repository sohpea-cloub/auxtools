// Which text under the mouse is an expression to show the value of, while
// the world is paused: a name, with what it's reached through before it
// (`a.b.c`, `a?.b`, `a:b`), up to the name under the mouse and no further.
// A name that's called (`b` in `a.b()`) isn't one: a hover doesn't call procs.

const NAME = /[A-Za-z_][A-Za-z0-9_]*/y;

// The name that `column` (from 0) is in or just after, as [start, end]
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

// Whether `position` is inside a text ("...") or a comment on this line
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

// The expression at `column` of `line`, as { start, end }, or undefined
function expressionAt(line, column) {
	const name = nameAt(line, column);
	if (!name || inTextOrComment(line, name[0])) {
		return undefined;
	}
	let [start, end] = name;

	// A proc's name, where it's called or declared
	if (/^\s*\(/.test(line.slice(end))) {
		return undefined;
	}
	// A keyword, or a number's letters
	const word = line.slice(start, end);
	if (/^(var|proc|verb|if|else|for|while|do|switch|return|new|del|in|to|as|set|try|catch|throw|goto|break|continue|spawn|step|null|global|static|const|tmp)$/.test(word)) {
		return undefined;
	}
	if (start > 1 && line[start - 1] === "." && /[0-9]/.test(line[start - 2])) {
		return undefined;
	}

	// What it's reached through: names joined by `.`, `?.`, `:` and `?:`
	for (;;) {
		const before = /([A-Za-z_][A-Za-z0-9_]*)\s*(\?\.|\?:|\.|:)\s*$/.exec(line.slice(0, start));
		// `a ? b : c` has a colon too: a space before it isn't a member's
		if (!before || (/:$/.test(before[2]) && /\s:\s*$/.test(line.slice(0, start)))) {
			break;
		}
		start = before.index;
	}
	// Still a member of something: of a call's result or a list's entry,
	// which a hover doesn't work out
	const through = /(\?\.|\?:|\.|:)\s*$/.exec(line.slice(0, start));
	if (through && !(through[1] === ":" && /\s:\s*$/.test(line.slice(0, start)))) {
		return undefined;
	}
	// A path (`/obj/item`) or a declaration (`var/x`) isn't a value to read
	if (start > 0 && line[start - 1] === "/") {
		return undefined;
	}
	return { start, end };
}

module.exports = { expressionAt };
