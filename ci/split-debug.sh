#!/bin/sh
# Moves the debug information of programs built for Linux into FILE.debug next
# to each, which gdb and lldb find by that name. (Windows' is in .pdb files as
# built.)
#
#     ci/split-debug.sh FILE...
set -eu
for file; do
	objcopy --only-keep-debug "$file" "$file.debug"
	chmod -x "$file.debug"
	objcopy --strip-debug --add-gnu-debuglink="$file.debug" "$file"
done
