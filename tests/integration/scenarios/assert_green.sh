#!/usr/bin/env bash
# Gate for CI: succeeds only when a run has zero unresolved failures.
# A FAIL, UNEXPECTED-OK, or sim-error row whose action later passed (same label,
# status ok/xfail) does not count — retried steps settle themselves.
# sim-error is a probe that failed for a non-budget reason (malformed arg, wrong
# account): a real defect, distinct from the intentional sim-exceeded frontier
# misses. Rows with status=research (20-feed width probes) never fail this gate.
#
#   RUN_TS=<run> bash tests/integration/scenarios/assert_green.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"

[ -f "$ACTIONS_TSV" ] || { echo "no actions.tsv for RUN_TS=$RUN_TS" >&2; exit 1; }

# Phase-completeness: when a lane process log exists (parallel_e2e redirects each
# lane's stderr to runs/<RUN_TS>.log), require the terminal 'run complete' marker.
# A lane killed by timeout/crash leaves a partial actions.tsv with no FAIL row,
# which the unresolved-failure scan below would wrongly pass. Skipped when no such
# log exists (e.g. full_e2e run directly, stderr to the terminal).
LANE_LOG="$INTEG_DIR/runs/$RUN_TS.log"
if [ -f "$LANE_LOG" ] && ! grep -q "run complete" "$LANE_LOG"; then
    echo "FAILED: no 'run complete' marker in $RUN_TS.log — phases did not finish" >&2
    exit 1
fi

unresolved=$(awk -F'\t' '
    NR > 1 { rows[NR] = $0; label[NR] = $3; status[NR] = $4; last = NR }
    END {
        bad = 0
        for (i = 2; i <= last; i++) {
            if (status[i] != "FAIL" && status[i] != "UNEXPECTED-OK" && status[i] != "sim-error") continue
            settled = 0
            for (j = i + 1; j <= last; j++) {
                if (label[j] == label[i] && (status[j] == "ok" || status[j] == "xfail")) {
                    settled = 1
                    break
                }
            }
            if (!settled) { print rows[i] > "/dev/stderr"; bad++ }
        }
        print bad
    }' "$ACTIONS_TSV")

echo "--- run $RUN_TS summary ---"
awk -F'\t' 'NR>1 {c[$4]++} END {for (k in c) printf "  %s: %d\n", k, c[k]}' "$ACTIONS_TSV"

if [ "$unresolved" -gt 0 ]; then
    echo "FAILED: $unresolved unresolved failure(s)" >&2
    exit 1
fi
echo "GREEN: no unresolved failures"
