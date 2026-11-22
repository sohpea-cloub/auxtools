const assert = require("assert");
const fs = require("fs");
const path = require("path");
const { confined } = require("./report");

const page = fs.readFileSync(path.join(__dirname, "../../auxtools-cli/src/profile.html"), "utf8");
const shown = confined(page.replace("/*DATA*/", '{"procs":[{"proc":"/a \\u003cscript>"}]}'));

const policy = /<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-([^']+)'; style-src 'nonce-\1'">/.exec(shown);
assert(policy, `the page has no policy:\n${shown.slice(0, 300)}`);
const [, nonce] = policy;
assert(policy.index < shown.search(/<(script|style)\b/), "a script or a style sheet comes before the policy");
assert(!confined(page).includes(nonce), "two panels share a nonce");

const elements = [...shown.matchAll(/<(script|style)\b([^>]*)>/g)].map(([, tag, attributes]) => `${tag}${attributes}`);
assert.deepStrictEqual(elements, [`style nonce="${nonce}"`, 'script type="application/json" id="data"', `script nonce="${nonce}"`]);

const markup = shown.replace(/<(script|style)\b[^>]*>.*?<\/\1>/gs, "");
const refused = markup.match(/<[^>]*\s(?:style|src|srcset|on\w+)\s*=[^>]*>|<(?:link|iframe|object|embed|base)\b[^>]*>/g);
assert(!refused, `the policy refuses ${refused}`);
const [, styles] = /<style\b[^>]*>(.*?)<\/style>/s.exec(shown);
assert(!/url\(|@import/.test(styles), "the style sheet loads something from elsewhere");

const other = confined("<html><script>1</script></html>");
assert(other.startsWith('<meta http-equiv="Content-Security-Policy"') && other.endsWith("<html><script>1</script></html>"), other);

console.log("ok: the profile report's content security policy");
