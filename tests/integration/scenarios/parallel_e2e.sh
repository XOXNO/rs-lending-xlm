#!/usr/bin/env bash

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"

BASE="$RUN_TS"
LANE_TIMEOUT="${LANE_TIMEOUT:-95m}"

# E2E_LANES selects the lanes, so a caller can run only the lane a change
# affects (for example `liq` after a liquidation change). Unset runs all five.
#
# `-`, not `:-`: an explicitly empty E2E_LANES must reach the zero-lane check
# below and abort, not expand to the default and run every lane.
read -r -a LANES <<<"${E2E_LANES-agg liq stress flash blend}"

phases_for() {
    case "$1" in
        agg)    echo "deploy lifecycle strategies admin governance teardown" ;;
        liq)    echo "deploy liquidation defindex teardown" ;;
        stress) echo "deploy stress oracle teardown" ;;
    esac
}

# Lanes backed by a dedicated scenario instead of full_e2e.sh phases. The
# scenarios carry their own wallet sets, wasm preflights, and green gate, so
# the orchestrator only maps lane -> script and applies the same outer gate.
script_for() {
    case "$1" in
        flash) echo "flash_position.sh" ;;
        blend) echo "blend.sh" ;;
        *)     echo "full_e2e.sh" ;;
    esac
}

describe_lane() {
    local script
    script="$(script_for "$1")"
    if [ "$script" = "full_e2e.sh" ]; then
        echo "phases: $(phases_for "$1")"
    else
        echo "scenario: $script"
    fi
}

timeout_bin=""
command -v timeout  >/dev/null 2>&1 && timeout_bin="timeout $LANE_TIMEOUT"
command -v gtimeout >/dev/null 2>&1 && timeout_bin="gtimeout $LANE_TIMEOUT"

log_orch() { printf '[%s] [orchestrator] %s\n' "$(date +%H:%M:%S)" "$*" >&2; }

# Rejects an unknown lane up front. `phases_for` prints nothing for one, and
# full_e2e.sh's `${PHASES:-...}` treats empty as unset, so a typo would run the
# default phases under the wrong lane name.
[ "${#LANES[@]}" -gt 0 ] || { log_orch "E2E_LANES resolved to no lanes"; exit 2; }
for lane in "${LANES[@]}"; do
    if [ -z "$(phases_for "$lane")" ] && [ "$(script_for "$lane")" = "full_e2e.sh" ]; then
        log_orch "unknown lane '$lane' (known: agg liq stress flash blend)"
        exit 2
    fi
done

mkdir -p "$INTEG_DIR/runs"

pids=()
for lane in "${LANES[@]}"; do
    lane_ts="${BASE}-${lane}"
    log_orch "launching lane '$lane' (RUN_TS=$lane_ts) $(describe_lane "$lane")"
    (
        export RUN_TS="$lane_ts"
        script="$(script_for "$lane")"
        if [ "$script" = "full_e2e.sh" ]; then
            export PHASES="$(phases_for "$lane")"
        fi
        exec $timeout_bin bash "$HERE/$script"
    ) >"$INTEG_DIR/runs/${lane_ts}.log" 2>&1 &
    pids+=("$!")
done

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

combined="$INTEG_DIR/runs/${BASE}-combined.md"
{
    echo "# Parallel testnet e2e — $BASE"
    echo
    # Names the lanes that ran, so a partial run does not read as full coverage.
    if [ "$overall" -eq 0 ]; then
        echo "**Result: GREEN (lanes run: ${LANES[*]})**"
    else
        echo "**Result: FAILED (lanes run: ${LANES[*]})**"
    fi
    if [ "${#LANES[@]}" -lt 5 ]; then
        echo
        echo "> Partial run — only ${#LANES[@]} of 5 lanes. Phases not covered here were not executed."
    fi
    echo
    for lane in "${LANES[@]}"; do
        echo "## Lane: $lane  (RUN_TS=${BASE}-${lane}, $(describe_lane "$lane"))"
        echo
        cat "$INTEG_DIR/runs/${BASE}-${lane}/report.md" 2>/dev/null || echo "_(no report — lane did not produce one)_"
        echo
    done
} > "$combined"
log_orch "combined report: $combined"

if [ "$overall" -eq 0 ]; then log_orch "ALL LANES GREEN"; else log_orch "ONE OR MORE LANES FAILED"; fi
exit "$overall"
