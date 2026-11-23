#!/bin/bash
# Run BYOND integration tests using prebuilt binaries:
#
#     ci/run-tests.sh PROGRAMS BYOND [TEST...]
#
# PROGRAMS contains the test binaries, the launcher and the debug library. The
# debug library must be built with `--features debug_server/test-commands`,
# because the suites inspect a world with those console commands. BYOND is
# the installation directory that contains bin/. Run from the repository root.
#
# Set WINE=wine to run Windows binaries on Linux.
#
# The tests run in parallel, each in target/run-tests/TEST with its own copy
# of the test worlds. Each test prints its output when it ends. A test that
# still runs after TIMEOUT seconds, 900 by default, is ended and fails. Set
# SERIAL=1 to run them one at a time from the repository root, with no time
# limit.
#
# With no TEST arguments, also run PROGRAMS/unit tests if that directory exists.
set -u
[ $# -ge 2 ] || { sed -n '2,16p' "$0" >&2; exit 2; }
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
report() {
	if [ "$2" -eq 0 ]; then
		echo "passed: $1 ($3 s)"
	else
		echo "FAILED: $1 (exit status $2, $3 s)"
		failed="$failed $1"
	fi
}

run() {
	name=$1
	shift
	echo "::group::$name"
	start=$(date +%s)
	${WINE:+"$WINE"} "$@"
	status=$?
	echo "::endgroup::"
	report "$name" $status $(($(date +%s) - start))
}

if [ -n "${SERIAL:-}" ]; then
	for test in $tests; do
		run "$test" "$programs/$test$exe"
	done
else
	work=$PWD/target/run-tests
	rm -rf "$work"
	[ -z "${WINE:-}" ] || "$WINE" cmd /c exit
	declare -A pids
	trap 'kill "${pids[@]}" 2>/dev/null; exit 130' INT TERM
	start=$(date +%s)
	for test in $tests; do
		mkdir -p "$work/$test/tests"
		cp -r tests/*_host "$work/$test/tests/"
		(cd "$work/$test" && exec ${WINE:+"$WINE"} "$programs/$test$exe") >"$work/$test.log" 2>&1 &
		pids[$test]=$!
	done
	left=$tests
	while [ -n "$left" ]; do
		sleep 1
		seconds=$(($(date +%s) - start))
		running=
		for test in $left; do
			if kill -0 "${pids[$test]}" 2>/dev/null; then
				if [ $seconds -lt "${TIMEOUT:-900}" ]; then
					running="$running $test"
					continue
				fi
				kill "${pids[$test]}"
				echo "$test was still running after $seconds s" >>"$work/$test.log"
			fi
			wait "${pids[$test]}"
			status=$?
			unset "pids[$test]"
			if [ $status -eq 0 ] && grep -q "^thread '[^']*'.* panicked at" "$work/$test.log"; then
				echo "$test panicked, but its exit status was 0" >>"$work/$test.log"
				status=1
			fi
			echo "::group::$test"
			cat "$work/$test.log"
			echo "::endgroup::"
			report "$test" $status $seconds
		done
		left=$running
	done
	trap - INT TERM
fi

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
