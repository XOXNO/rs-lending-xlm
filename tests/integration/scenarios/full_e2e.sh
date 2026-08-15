#!/usr/bin/env bash

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke assert wallet assets aggregator oracle protocol report; do
    source "$INTEG_DIR/lib/$f.sh"
done
for f in lifecycle strategies liquidation defindex admin governance stress; do
    source "$INTEG_DIR/flows/$f.sh"
done

init_run
if [ -f "$INTEG_DIR/appendix.md" ]; then
    cp -n "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || cp "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || true
else
    log "NOTE: no $INTEG_DIR/appendix.md (run 'make integration-appendix' to (re)generate)"
fi

check_tools 2>/dev/null || log "WARNING: some required tools missing (see check_tools)"
check_stellar_version 2>/dev/null || log "WARNING: stellar CLI version check failed or not met"

trap 'write_report; run_summary' EXIT

PHASES="${PHASES:-deploy lifecycle strategies liquidation admin governance stress}"

want() { grep -qw "$1" <<<"$PHASES"; }

phase wallets
new_wallet ADMIN admin
new_wallet ALICE alice
new_wallet BOB bob
new_wallet CAROL carol
new_wallet DAVE dave

if want deploy; then
    phase deploy
    deploy_protocol
fi

if want lifecycle; then
    flow_real_markets
    flow_fund_usdc
    flow_seed_liquidity
    flow_lifecycle
fi

if want strategies; then
    flow_flash_loans
    flow_strategies
fi

if want liquidation; then

    INV_TRANSIENT_CONTRACT_RE='Error\(Contract, #'
    flow_liq_setup
    flow_liq_single
    flow_liq_bulk
    flow_liq_spoke
    flow_liq_credit
    flow_liq_credit_rejections
    flow_clean_bad_debt
    # After clean_bad_debt: the owner-override socialization path is only
    # meaningful once the permissionless one has run on its own account.
    flow_force_socialize_and_recap
    # Last in the phase: halting LIQG's seizure leg is irreversible, so nothing
    # that needs to seize LIQG may run after it.
    flow_spoke_flags_and_curve
    unset INV_TRANSIENT_CONTRACT_RE
fi

if want defindex; then
    flow_defindex_strategy
fi

if want admin; then
    flow_admin
fi

if want governance; then
    flow_governance
fi

if want stress; then
    flow_stress_setup
    flow_stress_supply_frontier
    flow_stress_borrow_frontier single
    flow_stress_dualify
    flow_stress_borrow_frontier dual
    flow_stress_liq_frontier
fi

if want admin; then
    flow_admin_upgrade
fi

phase done
log "run complete"
