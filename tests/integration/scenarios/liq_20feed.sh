#!/usr/bin/env bash
# 20-feed liquidation: 10 unique collaterals + 10 unique debts on ONE account,
# then liquidate. Resumes a completed stress run (needs DAVE_ACCT with 10
# collaterals ST00..ST09 and dualified stress markets):
#
#   RUN_TS=<existing> bash tests/integration/scenarios/liq_20feed.sh
#
# Build path A: borrow all 10 debt assets in one tx (20-feed dual borrow —
# itself a frontier data point). Fallback path B if A exceeds the budget:
# fresh account, 5 colls -> 10 borrows (15 feeds) -> 5 more colls (the
# supply leg only prices mutated assets, so the account still ends 10+10).
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke assert wallet assets aggregator oracle protocol report; do
    source "$INTEG_DIR/lib/$f.sh"
done
source "$INTEG_DIR/flows/stress.sh"

init_run
trap 'write_report; run_summary' EXIT

[ -n "${DAVE_ACCT:-}" ] || { log "DAVE_ACCT missing — run the stress phase first"; exit 1; }

phase liq20_reprice
# Fresh dual-source $1 prices on all 20 markets: restores the crashed
# collateral side and renews timestamps so nothing reads stale (#206).
if [ -z "${LIQ20_REPRICED:-}" ]; then
    for i in $(seq 0 $((STRESS_N - 1))); do
        dual_px "$(stress_sac $i)" "$(stress_code $i)" "$WAD" "liq20_px_$(stress_code $i)"
    done
    save_state LIQ20_REPRICED 1
fi

phase liq20_build
# Collaterals: DAVE_ACCT holds 100,000 units x ST00..ST09 = $1M at $1.
# Debts: 50,000 units x ST10..ST19 = $500k (LTV cap $700k; pool has 200k each).
BORROW_ARGS=""
for i in $(seq 10 19); do BORROW_ARGS+=" $(stress_sac $i) $((50000 * STRESS_UNIT))"; done
ACCT=""
if [ -z "${LIQ20_ACCT:-}" ]; then
    sim_probe probe_borrow_20feed_dual "$DAVE" "$CONTROLLER" -- borrow \
        --caller "$DAVE_ADDR" --account_id "$DAVE_ACCT" \
        --borrows "$(pay_vec $BORROW_ARGS)" --to null
    if [ "$PROBE_STATUS" = ok ]; then
        inv liq20_borrow_10debts "$DAVE" "$CONTROLLER" -- borrow \
            --caller "$DAVE_ADDR" --account_id "$DAVE_ACCT" \
            --borrows "$(pay_vec $BORROW_ARGS)" --to null >/dev/null || exit 1
        save_state LIQ20_ACCT "$DAVE_ACCT"
        save_state LIQ20_CRASH_WAD "$((WAD / 10 * 6))"   # HF = 0.6*0.75/0.5 = 0.9
    else
        log "20-feed dual borrow exceeds budget — building via path B"
        args=""
        for i in $(seq 0 4); do args+=" $(stress_sac $i) $((1000 * STRESS_UNIT))"; done
        acct=$(inv liq20b_supply_5coll "$DAVE" "$CONTROLLER" -- supply \
            --caller "$DAVE_ADDR" --account_id 0 --spoke_id 0 \
            --assets "$(pay_vec $args)" | tr -d '"') || exit 1
        args=""
        for i in $(seq 10 19); do args+=" $(stress_sac $i) $((300 * STRESS_UNIT))"; done
        inv liq20b_borrow_10debts "$DAVE" "$CONTROLLER" -- borrow \
            --caller "$DAVE_ADDR" --account_id "$acct" \
            --borrows "$(pay_vec $args)" --to null >/dev/null || exit 1
        args=""
        for i in $(seq 5 9); do args+=" $(stress_sac $i) $((1000 * STRESS_UNIT))"; done
        inv liq20b_supply_5more "$DAVE" "$CONTROLLER" -- supply \
            --caller "$DAVE_ADDR" --account_id "$acct" --spoke_id 0 \
            --assets "$(pay_vec $args)" >/dev/null || exit 1
        save_state LIQ20_ACCT "$acct"
        save_state LIQ20_CRASH_WAD "$((WAD / 10 * 4))"   # HF = 0.4*0.75*10000/3000 = 1.0- -> 0.4 gives exactly 1.0; use deeper crash below
    fi
fi
ACCT="${LIQ20_ACCT}"
view liq20_positions "$CONTROLLER" -- get_account_positions --account_id "$ACCT" >/dev/null

phase liq20_crash
# Crash all 10 collateral feeds (primary + anchor in lock-step).
if [ -z "${LIQ20_CRASHED:-}" ]; then
    crash="${LIQ20_CRASH_WAD:-$((WAD / 10 * 6))}"
    [ "$crash" = "$((WAD / 10 * 4))" ] && crash=$((WAD / 100 * 35))
    for i in $(seq 0 9); do
        dual_px "$(stress_sac $i)" "$(stress_code $i)" "$crash" "liq20_crash_$(stress_code $i)"
    done
    save_state LIQ20_CRASHED 1
fi
assert_can_liquidated liq20_can_liq "$ACCT" true
assert_hf_below_wad liq20_hf "$ACCT"

phase liq20_liquidate
# Repay-1 liquidation: reads all 20 position feeds dual-source, seizes
# proportionally across all 10 collaterals.
REPAY=$((10000 * STRESS_UNIT))
[ -n "${LIQ20_CRASH_WAD:-}" ] && [ "$LIQ20_CRASH_WAD" = "$((WAD / 10 * 4))" ] && REPAY=$((100 * STRESS_UNIT))
sim_probe probe_liquidate_10coll_10debt "$CAROL" "$CONTROLLER" -- liquidate \
    --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
    --debt_payments "$(pay_vec "$(stress_sac 19)" "$REPAY")"
if [ "$PROBE_STATUS" = ok ]; then
    # Top-level (not a function): `local` would error, so use a plain assignment.
    liq20_debt_pre=$(_view_int liq20_debt_pre get_borrow_amount \
        --account_id "$ACCT" --asset "$(stress_sac 19)")
    if inv liq20_liquidate_proof "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
        --debt_payments "$(pay_vec "$(stress_sac 19)" "$REPAY")" >/dev/null; then
        log "20-feed liquidation LANDED on-chain (10 colls seized, 1 of 10 debts repaid)"
        assert_borrow_decreased liq20_debt_post "$ACCT" "$(stress_sac 19)" "$liq20_debt_pre"
    fi
else
    log "20-feed liquidation probe: $PROBE_STATUS"
fi

phase done
log "liq20 scenario complete"
