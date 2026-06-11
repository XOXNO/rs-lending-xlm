#!/usr/bin/env bash
# Maximal liquidation shape: repay ALL 10 debt assets in ONE liquidate tx
# while the seize leg spans ALL 10 collaterals (20 dual-source feeds).
# Probes 10 debts first, then bisects down (8/6/4/2) to find the repay-width
# frontier at full 10-collateral seize; sends the widest passing probe.
#
#   RUN_TS=<existing> bash tests/integration/scenarios/liq_20feed_fullrepay.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke wallet assets aggregator oracle protocol report; do
    source "$INTEG_DIR/lib/$f.sh"
done
source "$INTEG_DIR/flows/stress.sh"

init_run
trap 'write_report; run_summary' EXIT

ACCT="${LIQ20_ACCT:?run liq_20feed.sh first}"
REPAY_EACH=$((3000 * STRESS_UNIT))   # $3k per debt — far below any close-factor cap

phase liq20_fullrepay
view liq20f_can_liq "$CONTROLLER" -- can_be_liquidated --account_id "$ACCT" >/dev/null

best_n=0
for n in 10 8 6 4 2; do
    args=""
    for i in $(seq 10 $((9 + n))); do args+=" $(stress_sac $i) $REPAY_EACH"; done
    sim_probe "probe_liq_${n}debt_10coll" "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
        --debt_payments "$(pay_vec $args)"
    if [ "$PROBE_STATUS" = ok ]; then best_n=$n; break; fi
    log "liquidation with $n debt repays + 10-coll seize: $PROBE_STATUS"
done

if [ "$best_n" -gt 0 ]; then
    args=""
    for i in $(seq 10 $((9 + best_n))); do args+=" $(stress_sac $i) $REPAY_EACH"; done
    inv "liq20_fullrepay_proof_${best_n}debt" "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
        --debt_payments "$(pay_vec $args)" >/dev/null \
        && log "liquidation LANDED: $best_n debt repays + 10-coll seize in one tx"
else
    log "no repay width passed at 10-coll seize"
fi
save_state LIQ20_FULLREPAY_WIDTH "$best_n"

phase done
log "fullrepay scenario complete"
