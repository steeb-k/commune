#!/usr/bin/env bash
# Wait for a workflow run to finish, then say what broke and where.
#
# This exists because of a failure of process rather than of code. Runs were
# being started and then left: the turn ended, nobody looked, and the person
# who noticed the build was broken was the one who had asked for it. A run
# that nobody is waiting on is a run whose result arrives by complaint.
#
# So: start a run, then start this, and do not call the work done until this
# has printed a conclusion. It blocks, which is the point. On failure it
# prints the failing step's log tail, because "the Windows job failed" is not
# a fact anybody can act on and the last forty lines usually are.
#
#   bash build-aux/ci/watch-run.sh                # the newest run
#   bash build-aux/ci/watch-run.sh 34665639379    # a particular one
#
# Exits 0 if every job succeeded, 1 otherwise — so it can gate a `&&`.
#
# Every question it asks GitHub is asked up to five times, thirty seconds
# apart, before it gives up. On 15 October 2026 one `failed to get run` — a
# blip in the API, the run itself perfectly healthy — ended the watch partway
# through a release, and the person waiting on it lost the thing this script
# is for. A watch has to outlast the API's bad minutes, and a run that
# genuinely cannot be read still ends it, two minutes later.
set -euo pipefail

# `gh` with patience. Prints what `gh` printed on the attempt that worked;
# what it said on the ones that did not goes to stderr, so a retried blip is
# visible without ending up inside the answer.
gh_retry() {
    local attempt out err
    err="$(mktemp)"
    for attempt in 1 2 3 4 5; do
        if out="$(gh "$@" 2>"$err")"; then
            rm -f "$err"
            printf '%s\n' "$out"
            return 0
        fi
        printf 'watch-run: gh %s failed (attempt %d of 5): %s\n' "$1" "$attempt" "$(cat "$err")" >&2
        [ "$attempt" = 5 ] || sleep 30
    done
    rm -f "$err"
    return 1
}

run="${1:-}"
if [ -z "$run" ]; then
    run="$(gh_retry run list --limit 1 --json databaseId --jq '.[0].databaseId')"
fi

# How long to keep asking before giving up. The longest real build is the
# five-platform one at around two hours; three is enough headroom that a hit
# here means something is wedged, not slow.
deadline=$(( $(date +%s) + 3 * 60 * 60 ))

echo "watching run $run"
last=""
while :; do
    state="$(gh_retry run view "$run" --json status,conclusion \
        --jq '[.status, (.conclusion // "-")] | @tsv')"
    read -r status conclusion <<<"$state"

    # One line per job, and only when it changes: a poll loop that reprints
    # the same table every fifteen seconds buries the transition that matters.
    now="$(gh_retry run view "$run" --json jobs \
        --jq '.jobs[] | [.name, .status, (.conclusion // "-")] | @tsv')"
    if [ "$now" != "$last" ]; then
        printf '\n--- %s ---\n%s\n' "$(date -u +%H:%M:%S)" "$now"
        last="$now"
    fi

    [ "$status" = "completed" ] && break

    if [ "$(date +%s)" -gt "$deadline" ]; then
        echo "::error::run $run has not finished in three hours; something is wedged"
        exit 1
    fi
    sleep 15
done

echo
echo "=== run $run: $conclusion ==="

if [ "$conclusion" = "success" ]; then
    gh_retry run view "$run" --json jobs \
        --jq '.jobs[] | "\(.conclusion)\t\(.name)"'
    exit 0
fi

# The whole reason to wait: land on the step that broke rather than on the
# job that contained it.
gh_retry run view "$run" --json jobs --jq '
    .jobs[] | select(.conclusion != "success" and .conclusion != "skipped" and .conclusion != null) |
    "\nJOB \(.name) -> \(.conclusion)",
    (.steps[] | select(.conclusion == "failure") | "  failed at step: \(.name)")'

echo
log="$(gh run view "$run" --log-failed 2>/dev/null || true)"
if [ -z "$log" ]; then
    echo "(no failed-step log; the job was probably cancelled before it ran)"
    exit 1
fi

# The lines that say what went wrong, rather than the last eighty lines of
# whatever the step happened to be printing. A failing macOS build spends
# forty minutes printing `Compiling <crate>`, so a blind tail showed a wall
# of crate names and nothing about the failure — which meant grepping the
# log by hand, which is the work this script exists to save.
echo "--- what the log says went wrong ---"
printf '%s\n' "$log" \
    | grep -aiE 'error|fatal|fail|cannot|not found|no such|refused|denied|abort' \
    | grep -avE 'pipefail|Compiling|Downloaded|Fresh |warning: unused|--retry|extraheader|safe\.directory|CACHE_ON_FAILURE|fail-on-cache-miss|Cache not found|spurious network|Cleaning up orphan|Node.js 20 is deprecated|continue-on-error|ContinueOnError|errors.py|quick-error|thiserror|gix-error' \
    | tail -30

echo
echo "--- and the last lines before it stopped ---"
printf '%s\n' "$log" | tail -15

exit 1
