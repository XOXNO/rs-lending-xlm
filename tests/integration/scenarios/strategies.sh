#!/usr/bin/env bash

# Focused live-testnet exercise of cash `flash_loan` and the strategy
# entrypoints (multiply, swap_debt, swap_collateral, repay_debt_with_collateral).
# Deploys current controller/pool wasm plus the adversarial flash-loan receiver.

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke assert wallet assets aggregator oracle protocol report; do
    source "$INTEG_DIR/lib/$f.sh"
done
source "$INTEG_DIR/flows/lifecycle.sh"
source "$INTEG_DIR/flows/strategies.sh"

init_run
if [ -f "$INTEG_DIR/appendix.md" ]; then
    cp -n "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || cp "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || true
fi

check_tools 2>/dev/null || log "WARNING: some required tools missing (see check_tools)"
check_stellar_version 2>/dev/null || log "WARNING: stellar CLI version check failed or not met"

[ -f "$WASM_DIR/controller.wasm" ] \
    || die preflight "missing $WASM_DIR/controller.wasm (run make integration-wasm)"
[ -f "$WASM_DIR/flash_loan_receiver.wasm" ] \
    || die preflight "missing $WASM_DIR/flash_loan_receiver.wasm (run make integration-wasm)"

trap 'write_report; run_summary' EXIT

phase wallets
new_wallet ADMIN admin
new_wallet ALICE alice
new_wallet BOB bob
new_wallet CAROL carol

phase deploy
deploy_protocol
[ -n "${FLASH_RECEIVER:-}" ] \
    || die deploy_flash_receiver "FLASH_RECEIVER unset after deploy_protocol"

flow_real_markets || die markets "XLM/USDC/EURC market listing failed"
flow_fund_usdc || die funding "USDC funding failed"
flow_seed_liquidity || die seed "pool seed failed"
flow_lifecycle || die lifecycle "lifecycle coverage failed"
flow_flash_loans || die flash_loans "cash flash_loan live coverage failed"
flow_strategies || die strategies "strategy live coverage failed"

phase done
log "run complete"
log "controller=$CONTROLLER flash_receiver=$FLASH_RECEIVER multiply_acct=${ALICE_MACCT:-n/a}"
bash "$HERE/assert_green.sh" || exit 1
