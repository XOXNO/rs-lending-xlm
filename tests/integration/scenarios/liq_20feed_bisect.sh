#!/usr/bin/env bash
# Bisect the repay width: widest n-debt repay + 10-coll seize whose DECLARED
# instructions fit the 400M per-tx cap (sim-ok alone is insufficient: the
# 10-debt shape declares 400.5M, passes simulation, and is rejected by core).
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke wallet assets aggregator oracle protocol report; do source "$INTEG_DIR/lib/$f.sh"; done
source "$INTEG_DIR/flows/stress.sh"
init_run
trap 'write_report; run_summary' EXIT
ACCT="${LIQ20_ACCT:?}"
TX_CAP=400000000
REPAY_EACH=$((3000 * STRESS_UNIT))

phase liq20_bisect
for n in 9 8 7; do
    args=""
    for i in $(seq 10 $((9 + n))); do args+=" $(stress_sac $i) $REPAY_EACH"; done
    sim_probe "probe_liq_${n}debt_10coll" "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
        --debt_payments "$(pay_vec $args)"
    if [ "$PROBE_STATUS" = ok ] && [ -n "$RES_INSTR" ] && [ "$RES_INSTR" -le "$TX_CAP" ]; then
        log "width $n fits: declared $RES_INSTR insns <= $TX_CAP"
        inv "liq20_bisect_proof_${n}debt" "$CAROL" "$CONTROLLER" -- liquidate \
            --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
            --debt_payments "$(pay_vec $args)" >/dev/null \
            && log "liquidation LANDED: $n debt repays + 10-coll seize ($RES_INSTR insns)"
        save_state LIQ20_FULLREPAY_WIDTH "$n"
        break
    fi
    log "width $n: status=$PROBE_STATUS declared=${RES_INSTR:-n/a} insns — over cap or exceeded"
done
phase done
log "bisect complete"
