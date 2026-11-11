#!/bin/bash
# Runs the tests that need BYOND, with programs that are built already:
#
#     ci/run-tests.sh PROGRAMS BYOND [TEST...]
#
# PROGRAMS is the directory the test programs, the launcher and the debug
# library are in, as cargo leaves them in target/TARGET/PROFILE. BYOND is a
# BYOND build's directory, the one with bin/ in it. Run it from the
# repository's root: the worlds the tests run are in tests/.
#
# Programs built for Windows run as they are on Windows, and on Linux under
# Wine when WINE names it (WINE=wine).
#
# Unit tests built for another machine (cargo test --no-run) run too, when
# they're in PROGRAMS/unit.
set -u
[ $# -ge 2 ] || { sed -n '2,15p' "$0" >&2; exit 2; }
programs=$(cd "$1" && pwd) || exit 2
byond=$(cd "$2" && pwd) || exit 2
shift 2
tests=${*:-test_runner debug_test hot_reload_test launcher_test dap_test}

exe=
case "$(uname -s)" in
	MINGW* | MSYS* | CYGWIN*)
		exe=.exe
		BYOND_PATH=$(cygpath -w "$byond")
		;;
	*)
		if [ -n "${WINE:-}" ]; then
			exe=.exe
			# Wine has the whole filesystem as drive Z
			BYOND_PATH="Z:$(printf '%s' "$byond" | tr / '\\')"
			export WINEDEBUG="${WINEDEBUG:--all}"
		else
			BYOND_PATH=$byond
			export LD_LIBRARY_PATH="$byond/bin${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
		fi
		;;
esac
export BYOND_PATH

failed=
run() {
	name=$1
	shift
	echo "::group::$name"
	start=$(date +%s)
	${WINE:+"$WINE"} "$@"
	status=$?
	echo "::endgroup::"
	if [ $status -eq 0 ]; then
		echo "passed: $name ($(($(date +%s) - start)) s)"
	else
		echo "FAILED: $name (exit status $status, $(($(date +%s) - start)) s)"
		failed="$failed $name"
	fi
}

for test in $tests; do
	run "$test" "$programs/$test$exe"
done

# cargo runs a package's unit tests from the package's directory
if [ -d "$programs/unit" ] && [ $# -eq 0 ]; then
	root=$PWD
	for test in $(cd "$programs/unit" && find . -type f | sort); do
		cd "$(dirname "$test")" || exit 2
		run "unit tests: ${test#./}" "$programs/unit/$test"
		cd "$root" || exit 2
	done
fi

if [ -n "$failed" ]; then
	echo "failed:$failed"
	exit 1
fi
echo "all passed"
