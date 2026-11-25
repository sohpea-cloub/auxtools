#!/bin/bash
set -euo pipefail
cargo clippy --message-format=json "$@" | python3 -c '
import collections, json, os, sys

def escaped(text, also=""):
	for char in "%\r\n" + also:
		text = text.replace(char, "%%%02X" % ord(char))
	return text

annotate = os.environ.get("GITHUB_ACTIONS") == "true"
seen, lints = set(), collections.Counter()
for line in sys.stdin:
	try:
		message = json.loads(line)
	except ValueError:
		continue
	if message.get("reason") != "compiler-message":
		continue
	message = message["message"]
	rendered = message.get("rendered") or ""
	if message["level"] not in ("warning", "error") or rendered in seen:
		continue
	seen.add(rendered)
	print(rendered, end="")
	lint = (message.get("code") or {}).get("code") or message["level"]
	lints[lint] += 1
	span = next((span for span in message["spans"] if span["is_primary"]), None)
	if annotate and span:
		print("::%s file=%s,line=%d,endLine=%d,title=%s::%s" % (
			message["level"],
			escaped(span["file_name"], ":,"),
			span["line_start"],
			span["line_end"],
			escaped(lint, ":,"),
			escaped(message["message"]),
		))
for lint, count in sorted(lints.items(), key=lambda item: (-item[1], item[0])):
	print("%4d %s" % (count, lint))
print("%4d in all" % sum(lints.values()))
'
