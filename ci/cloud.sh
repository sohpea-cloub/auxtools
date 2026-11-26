#!/bin/bash
# Runs the tests on Linux and under Wine for some BYOND builds, in containers
# of the project's test service, and waits for the result:
#
#     ci/cloud.sh LINUX_BUNDLE WINDOWS_BUNDLE SOURCE BUILD...
#
# The bundles are the tar files of programs that CI's build jobs make, SOURCE
# is `git archive --format=tar.gz` of the commit they were built from, and
# each BUILD is a BYOND build, such as 516.1689.
#
# The job needs `permissions: id-token: write`: its GitHub identity token
# shows the service that the job runs in this repository. The environment
# gives REPOSITORY and COMMIT, which name the run. It may give RERUNS, how
# often a failed job is run again [1], KNOWN_BROKEN, builds whose failures are
# expected and don't fail the run, and CLOUD, the service's address.
#
# Prints a line for each build, in version order, then the logs of the jobs
# that failed unexpectedly. Exits with 1 if one did.
set -euo pipefail
[ $# -ge 4 ] || { sed -n '2,18p' "$0" >&2; exit 2; }
linux_bundle=$1 windows_bundle=$2 source_archive=$3
shift 3
cloud=${CLOUD:-https://auxtools-cloud.sohpea.workers.dev}
[ -n "${ACTIONS_ID_TOKEN_REQUEST_URL:-}" ] || { echo "::error::The job has no GitHub identity token: it needs 'permissions: id-token: write'."; exit 1; }
token=$(curl -fsS --retry 3 -H "Authorization: bearer $ACTIONS_ID_TOKEN_REQUEST_TOKEN" "$ACTIONS_ID_TOKEN_REQUEST_URL&audience=$cloud" | jq -r .value)
auth=(-H "Authorization: Bearer $token")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

upload() {
	local sha
	sha=$(sha256sum "$1" | cut -c 1-64)
	if [ "$(curl -sS -o /dev/null -w '%{http_code}' -I "${auth[@]}" "$cloud/uploads/$sha")" != 200 ]; then
		curl -fsS --retry 3 "${auth[@]}" -T "$1" "$cloud/uploads/$sha" >/dev/null
	fi
	echo "$sha"
}

gzip -c "$linux_bundle" >"$work/bundle-linux.tar.gz"
gzip -c "$windows_bundle" >"$work/bundle-windows.tar.gz"
linux=$(upload "$work/bundle-linux.tar.gz")
windows=$(upload "$work/bundle-windows.tar.gz")
source=$(upload "$source_archive")
request=$(jq -n --arg repository "$REPOSITORY" --arg commit "$COMMIT" --arg linux "$linux" --arg windows "$windows" \
	--arg source "$source" --argjson reruns "${RERUNS:-1}" --arg broken "${KNOWN_BROKEN:-}" '$ARGS.positional as $builds |
	{repository: $repository, commit: $commit, uploads: {linux: $linux, windows: $windows, source: $source},
	 builds: $builds, platforms: ["linux", "wine"], short: ["check_recipes", "compare_dreammaker"], reruns: $reruns,
	 knownBroken: ($broken | split(" ") | map(select(. != ""))), label: "\($repository) at \($commit[0:8])"}' --args "$@")
run=$(curl -sS --fail-with-body "${auth[@]}" -H "content-type: application/json" --data "$request" "$cloud/runs") ||
	{ echo "::error::The tests didn't start: $run"; exit 1; }
id=$(jq -r .id <<<"$run")
poll=$(jq -r .poll <<<"$run")
page=$(jq -r .page <<<"$run")
echo "Test results: $page"

while :; do
	sleep 10
	status=$(curl -fsS --retry 5 --retry-all-errors -H "x-poll-secret: $poll" "$cloud/runs/$id")
	[ "$(jq -r .done <<<"$status")" != true ] || break
done

lines=$(jq -r '
	{passed: "Passed", known: "Known failure", failed: "FAILED", running: "Not finished"} as $words |
	(.jobs | map({key: .name, value: .}) | from_entries) as $jobs |
	def results: map(select(.state != "passed") | "\(.title) — " +
		(if .state == "flaky" then "passed on retry"
		 elif (.failed | length) > 0 then "\(.failed | join(", ")) failed"
		 else .error // "failed" end)) | join("; ");
	.versions | sort_by(.version | split(".") | map(tonumber)) | .[] |
	["BYOND \(.version)", $words[.state], (.jobs | map($jobs[.]) | results)] | @tsv' <<<"$status")
echo
while IFS=$'\t' read -r name state results; do
	printf '%-20s %-14s %s\n' "$name" "$state" "$results"
done <<<"$lines"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
	{
		echo "[View test results]($page)"
		echo
		echo "| BYOND version | Status | Results |"
		echo "|---|---|---|"
		while IFS=$'\t' read -r name state results; do
			echo "| ${name#BYOND } | $state | $results |"
		done <<<"$lines"
	} >>"$GITHUB_STEP_SUMMARY"
fi

echo
while IFS=$'\t' read -r name title version; do
	echo "::group::BYOND $version — $title"
	curl -sS -H "x-poll-secret: $poll" "$cloud/runs/$id/jobs/$(jq -rn --arg name "$name" '$name | @uri')/log" || true
	echo "::endgroup::"
done < <(jq -r '.jobs[] | select(.state == "failed") | [.name, .title, .version] | @tsv' <<<"$status")

[ "$(jq -r .workflow <<<"$status")" = complete ] || { echo "::error::The tests ended early: $(jq -r .workflow <<<"$status")"; exit 1; }
[ "$(jq -r '[.jobs[] | select(.state != "passed" and .state != "flaky" and .state != "known")] | length' <<<"$status")" = 0 ]
