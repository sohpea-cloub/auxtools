"use strict";

const crypto = require("crypto");

function confined(html) {
	const nonce = crypto.randomBytes(16).toString("base64");
	const policy = `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-${nonce}'; style-src 'nonce-${nonce}'">`;
	if (!html.includes("<head>")) {
		return policy + html;
	}
	return html.replace("<head>", `<head>\n${policy}`).replace(/<(script|style)>/g, `<$1 nonce="${nonce}">`);
}

module.exports = { confined };
