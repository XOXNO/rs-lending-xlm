#!/usr/bin/env bash
# Parallel release e2e: independent lanes split along the aggregator boundary,
# run concurrently, then gated together. The harness is network-wait bound, so
# overlapping the lanes' waits cuts wall-clock to ~max(lane) instead of the sum.
#
#   RUN_TS=$(date +%Y%m%d-%H%M%S) bash tests/integration/scenarios/parallel_e2e.sh
#
# Lanes — each a self-contained full_e2e world (own controller / pool / governance
# / wallets / markets, keyed by RUN_TS=<base>-<lane>, so no shared state):
#   agg     lifecycle + strategies + admin + governance  (uses the XOXNO
#           aggregator/DEX venue → serial WITHIN this one lane to avoid swap races)
#   liq     liquidation + defindex strategy               (mock oracles, venue-free)
#   stress  stress                                        (mock oracles, venue-free)
# The two mock lanes are fully independent of agg and of each other, so all three
# run in parallel. Gating is per-lane; the run is green only if every lane is.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"   # requires RUN_TS; provides INTEG_DIR

BASE="$RUN_TS"
LANE_TIMEOUT="${LANE_TIMEOUT:-95m}"
LANES=(agg liq stress)

phases_for() {
    case "$1" in
        agg)    echo "deploy lifecycle strategies admin governance" ;;
        liq)    echo "deploy liquidation defindex" ;;
        stress) echo "deploy stress" ;;
    esac
}

timeout_bin=""
command -v timeout  >/dev/null 2>&1 && timeout_bin="timeout $LANE_TIMEOUT"
command -v gtimeout >/dev/null 2>&1 && timeout_bin="gtimeout $LANE_TIMEOUT"

log_orch() { printf '[%s] [orchestrator] %s\n' "$(date +%H:%M:%S)" "$*" >&2; }

mkdir -p "$INTEG_DIR/runs"

# Fan out: each lane is its own full_e2e process with a distinct RUN_TS.
pids=()
for lane in "${LANES[@]}"; do
    lane_ts="${BASE}-${lane}"
    log_orch "launching lane '$lane' (RUN_TS=$lane_ts) phases: $(phases_for "$lane")"
    (
        export RUN_TS="$lane_ts"
        export PHASES="$(phases_for "$lane")"
        exec $timeout_bin bash "$HERE/full_e2e.sh"
    ) >"$INTEG_DIR/runs/${lane_ts}.log" 2>&1 &
    pids+=("$!")
done

# Capture each lane's process exit. A non-zero exit (crash) or a timeout kill
# means the lane did not finish — regardless of what actions.tsv contains, so
# this must fail the run on its own. full_e2e exits 0 even when actions FAILed,
# so the exit code is necessary but not sufficient; assert_green covers that.
declare -a lane_exit
for i in "${!LANES[@]}"; do
    if wait "${pids[$i]}"; then
        lane_exit[$i]=0
        log_orch "lane '${LANES[$i]}' process exited 0"
    else
        lane_exit[$i]=$?
        log_orch "lane '${LANES[$i]}' process exited NON-ZERO (${lane_exit[$i]}: timeout/crash) — see runs/${BASE}-${LANES[$i]}.log"
    fi
done

# Gate each lane; overall green requires every lane to satisfy ALL of:
#   1. its process exited 0 (not killed by timeout / did not crash),
#   2. its log shows the terminal 'run complete' marker — every phase ran to the
#      end (catches a lane killed after actions.tsv init but before any FAIL row,
#      which assert_green alone would pass), and
#   3. assert_green finds no unresolved failure rows.
overall=0
for i in "${!LANES[@]}"; do
    lane="${LANES[$i]}"
    lane_ts="${BASE}-${lane}"
    lane_log="$INTEG_DIR/runs/${lane_ts}.log"
    log_orch "gating lane '$lane'"
    if [ "${lane_exit[$i]}" -ne 0 ]; then
        log_orch "lane '$lane' FAILED — process did not exit cleanly (${lane_exit[$i]})"
        overall=1
        continue
    fi
    if ! grep -q "run complete" "$lane_log" 2>/dev/null; then
        log_orch "lane '$lane' FAILED — no 'run complete' marker (phases incomplete) in ${lane_ts}.log"
        overall=1
        continue
    fi
    if RUN_TS="$lane_ts" bash "$HERE/assert_green.sh"; then
        log_orch "lane '$lane' GREEN"
    else
        log_orch "lane '$lane' FAILED gate"
        overall=1
    fi
done

# Combined report for the release artifact (per-lane reports concatenated).
combined="$INTEG_DIR/runs/${BASE}-combined.md"
{
    echo "# Parallel testnet e2e — $BASE"
    echo
    [ "$overall" -eq 0 ] && echo "**Result: GREEN (all lanes)**" || echo "**Result: FAILED (one or more lanes)**"
    echo
    for lane in "${LANES[@]}"; do
        echo "## Lane: $lane  (RUN_TS=${BASE}-${lane}, phases: $(phases_for "$lane"))"
        echo
        cat "$INTEG_DIR/runs/${BASE}-${lane}/report.md" 2>/dev/null || echo "_(no report — lane did not produce one)_"
        echo
    done
} > "$combined"
log_orch "combined report: $combined"

if [ "$overall" -eq 0 ]; then log_orch "ALL LANES GREEN"; else log_orch "ONE OR MORE LANES FAILED"; fi
exit "$overall"
