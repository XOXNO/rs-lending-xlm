#!/usr/bin/env bash
# Find the widest n-debt repay + 10-coll seize liquidation that fits the
# 16,384B contract-events cap (the binding limit; sim does not enforce it,
# so this walks down on-chain: a rejected attempt costs only its fee).
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke wallet assets aggregator oracle protocol report; do source "$INTEG_DIR/lib/$f.sh"; done
source "$INTEG_DIR/flows/stress.sh"
init_run
trap 'write_report; run_summary' EXIT
ACCT="${LIQ20_ACCT:?}"
REPAY_EACH=$((3000 * STRESS_UNIT))

phase liq20_width
for n in 4 3 2; do
    args=""
    for i in $(seq 10 $((9 + n))); do args+=" $(stress_sac $i) $REPAY_EACH"; done
    label="liq20_widest_${n}debt"
    if stellar contract invoke --id "$CONTROLLER" --source "$CAROL" --network "$NETWORK" \
        --instruction-leeway 8000000 -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
        --debt_payments "$(pay_vec $args)" \
        >"$LOG_DIR/$label.out" 2>"$LOG_DIR/$label.err"; then
        hash=$(grep -oE 'Signing transaction: [0-9a-f]{64}' "$LOG_DIR/$label.err" | tail -1 | awk '{print $3}')
        fetch_resources "$hash"
        record "$label" ok liquidate "$hash" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" ""
        log "LANDED: $n debt repays + 10-coll seize ($RES_INSTR insns) tx=$hash"
        save_state LIQ20_FULLREPAY_WIDTH "$n"
        break
    fi
    reason=$(grep -oE 'exceeds network config maximum","[0-9]+' "$LOG_DIR/$label.err" | head -1 | grep -oE '[0-9]+$')
    record "$label" FAIL liquidate "" "" "" "" "" "events ${reason:-?}B > 16384B"
    log "width $n REJECTED: events ${reason:-unknown}B > 16384B cap"
done
phase done
log "width search complete"
