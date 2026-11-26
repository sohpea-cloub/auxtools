#!/bin/sh
# Lets cargo reuse what an earlier run built from files that are unchanged:
#
#     ci/source-times.sh LIST
#
# cargo rebuilds a crate when one of its files or directories is newer than
# what was built from it, and a fresh checkout makes them all new. LIST, which
# is cached with cargo's output, has the contents and time of each as they
# were in the run that cached it. Run in a checkout of HEAD before cargo: a
# file or directory whose contents are as LIST has them gets the time LIST
# has, the others keep theirs, and LIST is written again for this checkout.
set -eu
[ $# -eq 1 ] || { sed -n '2,11p' "$0" >&2; exit 2; }
list=$1
tab=$(printf '\t')
mkdir -p "$(dirname "$list")"
git ls-tree -r -t HEAD >"$list.files"
if [ -f "$list" ]; then
	awk -F '\t' 'NR == FNR { time[$2 FS $3] = $1; next } $0 in time { print time[$0] FS $2 }' "$list" "$list.files" |
		while IFS=$tab read -r time file; do
			touch -c -d "@$time" -- "$file"
		done
fi
git ls-tree -r -t -z --name-only HEAD | xargs -0 stat -c "%Y$tab%n" >"$list.times"
awk -F '\t' 'NR == FNR { time[$2] = $1; next } { print time[$2] FS $0 }' "$list.times" "$list.files" >"$list"
rm "$list.files" "$list.times"
