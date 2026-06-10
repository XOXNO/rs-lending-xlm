#!/usr/bin/env bash
# Full release e2e against live testnet.
#
#   RUN_TS=$(date +%Y%m%d-%H%M%S) bash tests/integration/scenarios/full_e2e.sh
#
# Re-running with the SAME RUN_TS resumes: deployed contracts, wallets, and
# completed setup blocks are restored from runs/<RUN_TS>/state.env.
# PHASES selects a subset (space-separated), e.g.:
#   PHASES="deploy lifecycle" RUN_TS=... bash scenarios/full_e2e.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke wallet assets aggregator oracle protocol report; do
    source "$INTEG_DIR/lib/$f.sh"
done
for f in lifecycle strategies liquidation admin stress; do
    source "$INTEG_DIR/flows/$f.sh"
done

init_run
trap 'write_report; run_summary' EXIT

PHASES="${PHASES:-deploy lifecycle strategies liquidation admin stress}"

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
    flow_liq_setup
    flow_liq_single
    flow_liq_bulk
    flow_liq_emode
    flow_liq_isolation
    flow_clean_bad_debt
fi

if want admin; then
    flow_admin
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
