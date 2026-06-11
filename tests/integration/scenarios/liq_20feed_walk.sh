#!/usr/bin/env bash
# Walk repay widths 8..5 at full 10-collateral seize, recording each
# attempt's ACTUAL emitted-events size from the rejection (or landing it).
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke wallet assets aggregator oracle protocol report; do source "$INTEG_DIR/lib/$f.sh"; done
source "$INTEG_DIR/flows/stress.sh"
init_run
trap 'write_report; run_summary' EXIT
ACCT="${LIQ20_ACCT:?}"
REPAY_EACH=$((3000 * STRESS_UNIT))

phase liq20_walk
for n in 8 7 6 5; do
    args=""
    for i in $(seq 10 $((9 + n))); do args+=" $(stress_sac $i) $REPAY_EACH"; done
    label="liq20_walk_${n}debt"
    if stellar contract invoke --id "$CONTROLLER" --source "$CAROL" --network "$NETWORK" \
        --instruction-leeway 8000000 -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
        --debt_payments "$(pay_vec $args)" \
        >"$LOG_DIR/$label.out" 2>"$LOG_DIR/$label.err"; then
        hash=$(grep -oE 'Signing transaction: [0-9a-f]{64}' "$LOG_DIR/$label.err" | tail -1 | awk '{print $3}')
        fetch_resources "$hash"
        record "$label" ok liquidate "$hash" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" ""
        log "width $n LANDED ($RES_INSTR insns) tx=$hash"
        save_state LIQ20_FULLREPAY_WIDTH "$n"
        break
    fi
    sz=$(grep -oE 'maximum","[0-9]+","16384' "$LOG_DIR/$label.err" | grep -oE '","[0-9]+","' | grep -oE '[0-9]+' | head -1)
    [ -z "$sz" ] && sz=$(grep -oE '"[0-9]{5}"' "$LOG_DIR/$label.err" | grep -oE '[0-9]+' | head -1)
    record "$label" FAIL liquidate "" "" "" "" "" "events ${sz:-?}B > 16384B"
    log "width $n REJECTED: events ${sz:-unknown}B vs 16384B cap"
done
phase done
log "walk complete"
