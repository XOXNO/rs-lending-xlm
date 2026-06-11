#!/usr/bin/env bash
# Re-send the 9-debt + 10-coll liquidation with --instruction-leeway padding
# the declared budget toward (but under) the 400M cap, absorbing the
# sim-vs-apply accrual drift that killed the unpadded attempt.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke wallet assets aggregator oracle protocol report; do source "$INTEG_DIR/lib/$f.sh"; done
source "$INTEG_DIR/flows/stress.sh"
init_run
trap 'write_report; run_summary' EXIT
ACCT="${LIQ20_ACCT:?}"
REPAY_EACH=$((3000 * STRESS_UNIT))

phase liq20_retry9
args=""
for i in $(seq 10 18); do args+=" $(stress_sac $i) $REPAY_EACH"; done
label=liq20_proof_9debt_leeway
# ~391.7M simulated; +8M leeway declares ~399.7M, still under the 400M cap.
if stellar contract invoke --id "$CONTROLLER" --source "$CAROL" --network "$NETWORK" \
    --instruction-leeway 8000000 -- liquidate \
    --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
    --debt_payments "$(pay_vec $args)" \
    >"$LOG_DIR/$label.out" 2>"$LOG_DIR/$label.err"; then
    hash=$(grep -oE 'Signing transaction: [0-9a-f]{64}' "$LOG_DIR/$label.err" | tail -1 | awk '{print $3}')
    fetch_resources "$hash"
    record "$label" ok liquidate "$hash" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" ""
    log "liquidation LANDED: 9 debt repays + 10-coll seize ($RES_INSTR insns) tx=$hash"
else
    record "$label" FAIL liquidate "" "" "" "" "" "$(tail -c 300 "$LOG_DIR/$label.err" | tr '\n\t' '  ')"
    log "9-debt retry FAILED: $(tail -3 "$LOG_DIR/$label.err" | tr '\n' ' ')"
fi
phase done
log "retry complete"
