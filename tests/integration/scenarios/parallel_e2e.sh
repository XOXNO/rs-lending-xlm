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

# A lane process exiting non-zero (crash/timeout) is only logged — pass/fail is
# decided by assert_green below, since full_e2e exits 0 even when actions FAILed.
for i in "${!LANES[@]}"; do
    if wait "${pids[$i]}"; then
        log_orch "lane '${LANES[$i]}' process exited 0"
    else
        log_orch "lane '${LANES[$i]}' process exited NON-ZERO (timeout/crash) — see runs/${BASE}-${LANES[$i]}.log"
    fi
done

# Gate each lane; overall green requires all lanes green.
overall=0
for lane in "${LANES[@]}"; do
    log_orch "gating lane '$lane'"
    if RUN_TS="${BASE}-${lane}" bash "$HERE/assert_green.sh"; then
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
