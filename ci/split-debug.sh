#!/bin/sh
set -eu
for file; do
	objcopy --only-keep-debug "$file" "$file.debug"
	chmod -x "$file.debug"
	objcopy --strip-debug --add-gnu-debuglink="$file.debug" "$file"
done
