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
set -euo pipefail

run="${1:-}"
if [ -z "$run" ]; then
    run="$(gh run list --limit 1 --json databaseId --jq '.[0].databaseId')"
fi

# How long to keep asking before giving up. The longest real build is the
# five-platform one at around two hours; three is enough headroom that a hit
# here means something is wedged, not slow.
deadline=$(( $(date +%s) + 3 * 60 * 60 ))

echo "watching run $run"
last=""
while :; do
    read -r status conclusion < <(
        gh run view "$run" --json status,conclusion \
            --jq '[.status, (.conclusion // "-")] | @tsv'
    )

    # One line per job, and only when it changes: a poll loop that reprints
    # the same table every fifteen seconds buries the transition that matters.
    now="$(gh run view "$run" --json jobs \
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
    gh run view "$run" --json jobs \
        --jq '.jobs[] | "\(.conclusion)\t\(.name)"'
    exit 0
fi

# The whole reason to wait: land on the step that broke rather than on the
# job that contained it.
gh run view "$run" --json jobs --jq '
    .jobs[] | select(.conclusion != "success" and .conclusion != "skipped" and .conclusion != null) |
    "\nJOB \(.name) -> \(.conclusion)",
    (.steps[] | select(.conclusion == "failure") | "  failed at step: \(.name)")'

echo
echo "--- log tail of the failing jobs ---"
gh run view "$run" --log-failed 2>/dev/null | tail -80 || \
    echo "(no failed-step log; the job was probably cancelled before it ran)"

exit 1
