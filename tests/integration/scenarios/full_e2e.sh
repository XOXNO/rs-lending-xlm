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

# Preflight guards (non-fatal; CI may enforce).
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
    # Under heavy testnet read-after-write lag a flow op can revert on state the
    # immediately-prior op wrote but the replica hasn't synced yet — a borrow
    # racing the account its supply just opened (#24 AccountNotFound), or a
    # liquidate reading a price crash that hasn't landed (#101 HealthFactorTooHigh).
    # These flows are validated and must-succeed, so let inv re-simulate contract
    # errors with backoff. The xfail revert guards use a separate path (they do
    # not read INV_TRANSIENT_CONTRACT_RE), so their expected reverts are unaffected.
    INV_TRANSIENT_CONTRACT_RE='Error\(Contract, #'
    flow_liq_setup
    flow_liq_single
    flow_liq_bulk
    flow_liq_spoke
    flow_clean_bad_debt
    unset INV_TRANSIENT_CONTRACT_RE
fi

# DeFindex strategy adapter on its own dedicated mock market; venue-free, so it
# rides the mock liquidation lane.
if want defindex; then
    flow_defindex_strategy
fi

if want admin; then
    flow_admin
fi

# Governance timelock e2e on the governance-owned controller (independent of the
# EOA controller state); runs before admin_upgrade pauses the EOA controller.
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
