const NAME = /[A-Za-z_][A-Za-z0-9_]*/y;
const KEYWORD = /^(var|proc|verb|if|else|for|while|do|switch|return|new|del|in|to|as|set|try|catch|throw|goto|break|continue|spawn|step|null|global|static|const|tmp)$/;

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

function brackets(line) {
	const opened = [];
	const opens = new Map();
	let quoted = false;
	for (let i = 0; i < line.length; i++) {
		const c = line[i];
		if (quoted) {
			if (c === "\\") {
				i++;
			} else if (c === '"') {
				quoted = false;
			}
		} else if (c === '"') {
			quoted = true;
		} else if (c === "[" || c === "(") {
			opened.push(i);
		} else if (c === "]" || c === ")") {
			const open = opened.pop();
			if (open !== undefined && line[open] === (c === "]" ? "[" : "(")) {
				opens.set(i, open);
			}
		}
	}
	return opens;
}

function operandStart(line, end, opens) {
	let at = end;
	while (line[at - 1] === "]" && opens.has(at - 1)) {
		at = opens.get(at - 1);
		if (line[at - 1] === "?") {
			at--;
		}
	}
	const name = /[A-Za-z_][A-Za-z0-9_]*$/.exec(line.slice(0, at));
	return name ? name.index : undefined;
}

function expressionAt(line, column) {
	const opens = brackets(line);
	let start;
	let end;
	if (line[column] === "]" && opens.has(column)) {
		if (inTextOrComment(line, column)) {
			return undefined;
		}
		end = column + 1;
		start = operandStart(line, end, opens);
		if (start === undefined) {
			return undefined;
		}
	} else {
		const name = nameAt(line, column);
		if (!name || inTextOrComment(line, name[0])) {
			return undefined;
		}
		[start, end] = name;
		if (/^\s*\(/.test(line.slice(end))) {
			return undefined;
		}
		if (KEYWORD.test(line.slice(start, end))) {
			return undefined;
		}
		if (/(^|[^A-Za-z0-9_])[0-9][A-Za-z0-9_]*\.$/.test(line.slice(0, start))) {
			return undefined;
		}
	}

	for (;;) {
		if (/(^|[^A-Za-z0-9_\])"])::$/.test(line.slice(0, start))) {
			start -= 2;
			break;
		}
		const joined = /\s*(\?\.|\?:|\.|:)\s*$/.exec(line.slice(0, start));
		if (!joined || (/:$/.test(joined[1]) && /\s:\s*$/.test(line.slice(0, start)))) {
			break;
		}
		const before = operandStart(line, joined.index, opens);
		if (before === undefined) {
			return undefined;
		}
		start = before;
	}
	if (start > 0 && line[start - 1] === "/") {
		return undefined;
	}
	return { start, end };
}

function evaluatableExpression(vscode, document, position) {
	const found = expressionAt(document.lineAt(position.line).text, position.character);
	if (!found) {
		return undefined;
	}
	return new vscode.EvaluatableExpression(new vscode.Range(position.line, found.start, position.line, found.end));
}

module.exports = { evaluatableExpression, expressionAt };
