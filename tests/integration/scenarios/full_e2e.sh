#!/usr/bin/env bash

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke assert wallet assets aggregator oracle protocol report; do
    source "$INTEG_DIR/lib/$f.sh"
done
for f in lifecycle strategies liquidation defindex admin governance stress swap_aggregator flash_position xoxno_oracle nft teardown; do
    source "$INTEG_DIR/flows/$f.sh"
done

E2E_LANE="${E2E_LANE:-agg}"
init_run
if [ -f "$INTEG_DIR/appendix.md" ]; then
    cp -n "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || cp "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || true
else
    log "NOTE: no $INTEG_DIR/appendix.md (run 'make integration-appendix' to (re)generate)"
fi

check_tools || die preflight "required tool missing"
check_stellar_version || die preflight "wrong stellar CLI version"

trap 'finish_run $?' EXIT
trap 'exit 130' INT TERM

PHASES="${PHASES:-deploy lifecycle strategies admin governance teardown}"

want() { grep -qw "$1" <<<"$PHASES"; }

wallets() {
    new_wallet ADMIN admin || return 1
    new_wallet ALICE alice || return 1
    new_wallet BOB bob || return 1
    new_wallet CAROL carol || return 1
    new_wallet DAVE dave || return 1
}
run_case wallets wallets || die wallets "wallet preflight failed"

if want deploy; then
    phase deploy
    run_case deploy_protocol deploy_protocol || die deploy_protocol "required case failed"
fi

if want lifecycle; then
    run_case flow_real_markets flow_real_markets || die flow_real_markets "required case failed"
    run_case flow_fund_usdc flow_fund_usdc || die flow_fund_usdc "required case failed"
    run_case flow_seed_liquidity flow_seed_liquidity || die flow_seed_liquidity "required case failed"
    run_case flow_lifecycle flow_lifecycle || die flow_lifecycle "required case failed"
    run_case flow_same_market flow_same_market || die flow_same_market "required case failed"
    run_case flow_nft flow_nft || die flow_nft "required case failed"
fi

if want strategies; then
    run_case flow_flash_loans flow_flash_loans || die flow_flash_loans "required case failed"
    run_case flow_strategies flow_strategies || die flow_strategies "required case failed"
fi

if want flash_position; then
    run_case flow_flash_position flow_flash_position || die flow_flash_position "required case failed"
fi

if want liquidation; then

    INV_TRANSIENT_CONTRACT_RE='Error\(Contract, #'
    run_case flow_liq_setup flow_liq_setup || die flow_liq_setup "required case failed"
    run_case flow_liq_single flow_liq_single || die flow_liq_single "required case failed"
    run_case flow_liq_multi_hub flow_liq_multi_hub || die flow_liq_multi_hub "required case failed"
    run_case flow_liq_bulk flow_liq_bulk || die flow_liq_bulk "required case failed"
    run_case flow_liq_spoke flow_liq_spoke || die flow_liq_spoke "required case failed"
    run_case flow_liq_credit flow_liq_credit || die flow_liq_credit "required case failed"
    run_case flow_liq_credit_rejections flow_liq_credit_rejections || die flow_liq_credit_rejections "required case failed"
    run_case flow_clean_bad_debt flow_clean_bad_debt || die flow_clean_bad_debt "required case failed"
    run_case flow_force_socialize_and_recap flow_force_socialize_and_recap || die flow_force_socialize_and_recap "required case failed"
    # Sets no_seize on LIQG for the rest of the run: no later flow may seize LIQG.
    run_case flow_spoke_flags_and_curve flow_spoke_flags_and_curve || die flow_spoke_flags_and_curve "required case failed"
    run_case flow_liq_deprecated_spoke_credit flow_liq_deprecated_spoke_credit || die flow_liq_deprecated_spoke_credit "required case failed"
    unset INV_TRANSIENT_CONTRACT_RE
fi

if want defindex; then
    run_case flow_defindex_strategy flow_defindex_strategy || die flow_defindex_strategy "required case failed"
fi

if want admin; then
    run_case flow_admin flow_admin || die flow_admin "required case failed"
    run_case flow_risk_refresh flow_risk_refresh || die flow_risk_refresh "required case failed"
    run_case flow_gap_hunt_admin flow_gap_hunt_admin || die flow_gap_hunt_admin "required case failed"
    run_case flow_pool_surface flow_pool_surface || die flow_pool_surface "required case failed"
    run_case flow_swap_aggregator_admin flow_swap_aggregator_admin || die flow_swap_aggregator_admin "required case failed"
fi

# Standalone keyword to rerun the gap-hunt admin checks while the seed account
# is live (the admin phase runs them too).
if want gap_hunt; then
    run_case flow_gap_hunt_admin flow_gap_hunt_admin || die flow_gap_hunt_admin "required case failed"
fi

if want governance; then
    run_case flow_governance flow_governance || die flow_governance "required case failed"
fi

if want stress; then
    run_case flow_stress_setup flow_stress_setup || die flow_stress_setup "required case failed"
    run_case flow_stress_supply_frontier flow_stress_supply_frontier || die flow_stress_supply_frontier "required case failed"
    run_case flow_stress_borrow_frontier:single flow_stress_borrow_frontier single || die flow_stress_borrow_frontier "required case failed"
    run_case flow_stress_dualify flow_stress_dualify || die flow_stress_dualify "required case failed"
    run_case flow_stress_borrow_frontier:dual flow_stress_borrow_frontier dual || die flow_stress_borrow_frontier "required case failed"
    run_case flow_stress_composed flow_stress_composed || die flow_stress_composed "required case failed"
    run_case flow_stress_delayed flow_stress_delayed || die flow_stress_delayed "required case failed"
    run_case flow_stress_liq_frontier flow_stress_liq_frontier || die flow_stress_liq_frontier "required case failed"
fi

if want admin; then
    run_case flow_admin_upgrade flow_admin_upgrade || die flow_admin_upgrade "required case failed"
fi

if want oracle; then
    run_case flow_xoxno_oracle flow_xoxno_oracle || die flow_xoxno_oracle "required case failed"
fi

# Runs last: teardown closes every position, so no flow that needs one may
# follow it.
if want teardown; then
    run_case flow_teardown flow_teardown || die flow_teardown "required case failed"
fi

phase done
python3 "$INTEG_DIR/gate.py" "$RUN_DIR" || exit 1
log "run complete"
