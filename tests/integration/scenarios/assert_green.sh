#!/usr/bin/env bash
# Gate for CI: succeeds only when a run has zero unresolved failures.
# A FAIL or UNEXPECTED-OK row whose action later passed (same label, status
# ok/xfail) does not count — retried steps settle themselves.
# Rows with status=research (20-feed width probes) are intentional frontier
# misses and never fail this gate.
#
#   RUN_TS=<run> bash tests/integration/scenarios/assert_green.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"

[ -f "$ACTIONS_TSV" ] || { echo "no actions.tsv for RUN_TS=$RUN_TS" >&2; exit 1; }

unresolved=$(awk -F'\t' '
    NR > 1 { rows[NR] = $0; label[NR] = $3; status[NR] = $4; last = NR }
    END {
        bad = 0
        for (i = 2; i <= last; i++) {
            if (status[i] != "FAIL" && status[i] != "UNEXPECTED-OK") continue
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
