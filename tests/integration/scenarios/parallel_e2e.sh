#!/usr/bin/env bash

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"

BASE="$RUN_TS"
LANE_TIMEOUT="${LANE_TIMEOUT:-95m}"
[[ "$LANE_TIMEOUT" =~ ^[1-9][0-9]*[smh]$ ]] || { echo 'invalid LANE_TIMEOUT' >&2; exit 2; }

# E2E_LANES selects the lanes, so a caller can run only the lane a change
# affects (for example `liq` after a liquidation change). Unset runs all seven.
#
# `-`, not `:-`: an explicitly empty E2E_LANES must reach the zero-lane check
# below and abort, not expand to the default and run every lane.
read -r -a LANES <<<"${E2E_LANES-agg liq stress flash blend production sdk}"

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
        production) echo "production.sh" ;;
        sdk) echo "sdk.sh" ;;
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
command -v timeout  >/dev/null 2>&1 && timeout_bin="timeout"
command -v gtimeout >/dev/null 2>&1 && timeout_bin="gtimeout"

log_orch() { printf '[%s] [orchestrator] %s\n' "$(date +%H:%M:%S)" "$*" >&2; }

# Rejects an unknown lane up front. `phases_for` prints nothing for one, and
# full_e2e.sh's `${PHASES:-...}` treats empty as unset, so a typo would run the
# default phases under the wrong lane name.
[ "${#LANES[@]}" -gt 0 ] || { log_orch "E2E_LANES resolved to no lanes"; exit 2; }
for lane in "${LANES[@]}"; do
    if [ -z "$(phases_for "$lane")" ] && [ "$(script_for "$lane")" = "full_e2e.sh" ]; then
        log_orch "unknown lane '$lane' (known: agg liq stress flash blend production sdk)"
        exit 2
    fi
done

[ -n "$timeout_bin" ] || { log_orch "timeout utility required"; exit 2; }
seen=" "
for lane in "${LANES[@]}"; do
    case "$seen" in *" $lane "*) log_orch "duplicate lane $lane"; exit 2;; esac
    seen+="$lane "
done
mkdir -p "$INTEG_DIR/runs"

export E2E_LIMITS_FILE="$INTEG_DIR/runs/$BASE-network-limits.json"
"${NODE_BIN:-node}" "$INTEG_DIR/sdk/limits.mjs" "$NETWORKS_FILE" "$E2E_LIMITS_FILE" || exit 1

install_wasms() {
    local dir="$INTEG_DIR/runs/$BASE-wasm-install/logs" alias="e2e_installer_$BASE"
    local names name wasm want got rc attempt addr
    local -a wasms=()
    mkdir -p "$dir"
    names=$(jq -r '.artifacts | keys[]' "$WASM_DIR/candidate.json") && [ -n "$names" ] \
        || { log_orch "no candidate artifacts in $WASM_DIR/candidate.json"; return 1; }
    for name in $names; do wasms+=("$WASM_DIR/$name"); done
    for wasm in "$FIXTURE_WASM_DIR"/*.wasm; do [ ! -f "$wasm" ] || wasms+=("$wasm"); done
    stellar keys address "$alias" >/dev/null 2>&1 || stellar keys generate "$alias" "${NET_ARGS[@]}" >"$dir/installer.log" 2>&1
    addr=$(stellar keys address "$alias") || { log_orch "installer key $alias missing — see $dir/installer.log"; return 1; }
    for attempt in 1 2 3 4 5; do
        curl -fsS -m 30 "https://horizon-testnet.stellar.org/accounts/$addr" >/dev/null 2>>"$dir/installer.log" && break
        [ "$attempt" -lt 5 ] || { log_orch "installer wallet $addr was not funded — see $dir/installer.log"; return 1; }
        curl -sS -m 30 "https://friendbot.stellar.org/?addr=$addr" >>"$dir/installer.log" 2>&1
        sleep "$attempt"
    done
    for wasm in "${wasms[@]}"; do
        name=$(basename "$wasm" .wasm)
        want=$(python3 -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$wasm") \
            || { log_orch "cannot read $wasm"; return 1; }
        for attempt in 1 2 3 4 5; do
            rc=0
            got=$(stellar contract upload --wasm "$wasm" --source "$alias" "${NET_ARGS[@]}" \
                --instruction-leeway "${INSTRUCTION_LEEWAY:-20000000}" 2>"$dir/$name.$attempt.err") || rc=$?
            got="${got//[[:space:]\"]/}"
            [ "$rc" -eq 0 ] || [ "$attempt" -eq 5 ] || { sleep $((attempt * 5)); continue; }
            break
        done
        [ "$rc" -eq 0 ] && [ "$got" = "$want" ] \
            || { log_orch "install of $name.wasm failed (rc=$rc, hash '$got') — see $dir/$name.$attempt.err"; return 1; }
        log_orch "installed $name.wasm $want"
    done
}
install_wasms || exit 1

pids=()
stop_children() {
    trap - INT TERM
    # GNU timeout owns a process group; kill that group, including CLI/RPC children.
    for pid in "${pids[@]}"; do [ -z "$pid" ] || kill -TERM -- "-$pid" 2>/dev/null || true; done
    sleep 2
    for pid in "${pids[@]}"; do [ -z "$pid" ] || kill -KILL -- "-$pid" 2>/dev/null || true; done
    for pid in "${pids[@]}"; do [ -z "$pid" ] || wait "$pid" 2>/dev/null || true; done
    for lane in "${LANES[@]}"; do
        [ ! -f "$INTEG_DIR/runs/$BASE-$lane/metadata.json" ] || python3 "$INTEG_DIR/gate.py" mark-incomplete "$INTEG_DIR/runs/$BASE-$lane" cancelled
    done
    exit 130
}
trap stop_children INT TERM
for lane in "${LANES[@]}"; do
    lane_ts="${BASE}-${lane}"
    log_orch "launching lane '$lane' (RUN_TS=$lane_ts) $(describe_lane "$lane")"
    (
        export RUN_TS="$lane_ts"
        export E2E_LANE="$lane"
        script="$(script_for "$lane")"
        if [ "$script" = "full_e2e.sh" ]; then
            export PHASES="$(phases_for "$lane")"
        fi
        exec "$timeout_bin" --kill-after=10s "$LANE_TIMEOUT" bash "$HERE/$script"
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
        python3 "$INTEG_DIR/gate.py" mark-incomplete "$INTEG_DIR/runs/$BASE-${LANES[$i]}" "lane exit ${lane_exit[$i]}" || true
        log_orch "lane '${LANES[$i]}' process exited NON-ZERO (${lane_exit[$i]}: timeout/crash) — see runs/${BASE}-${LANES[$i]}.log"
    fi
    pids[$i]=""
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
    if [ "${#LANES[@]}" -lt 7 ]; then
        echo
        echo "> Partial run — only ${#LANES[@]} of 7 lanes. Phases not covered here were not executed."
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
