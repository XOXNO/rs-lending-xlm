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

E2E_LANE="${E2E_LANE:-strategies}"
init_run
if [ -f "$INTEG_DIR/appendix.md" ]; then
    cp -n "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || cp "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || true
fi

check_tools || die preflight "required tool missing"
check_stellar_version || die preflight "wrong stellar CLI version"

[ -f "$WASM_DIR/controller.wasm" ] \
    || die preflight "missing $WASM_DIR/controller.wasm (run make integration-wasm)"
[ -f "$FIXTURE_WASM_DIR/flash_loan_receiver.wasm" ] \
    || die preflight "missing $FIXTURE_WASM_DIR/flash_loan_receiver.wasm (run make integration-wasm)"

trap 'finish_run $?' EXIT
trap 'exit 130' INT TERM

wallets() {
    new_wallet ADMIN admin || return 1
    new_wallet ALICE alice || return 1
    new_wallet BOB bob || return 1
    new_wallet CAROL carol || return 1
}
run_case wallets wallets || die wallets "wallet preflight failed"

phase deploy
run_case deploy_protocol deploy_protocol || die deploy_protocol "required case failed"
[ -n "${FLASH_RECEIVER:-}" ] \
    || die deploy_flash_receiver "FLASH_RECEIVER unset after deploy_protocol"

run_case flow_real_markets flow_real_markets || die flow_real_markets "required case failed"
run_case flow_fund_usdc flow_fund_usdc || die flow_fund_usdc "required case failed"
run_case flow_seed_liquidity flow_seed_liquidity || die flow_seed_liquidity "required case failed"
run_case flow_lifecycle flow_lifecycle || die flow_lifecycle "required case failed"
run_case flow_flash_loans flow_flash_loans || die flow_flash_loans "required case failed"
run_case flow_strategies flow_strategies || die flow_strategies "required case failed"

phase done
python3 "$INTEG_DIR/gate.py" "$RUN_DIR" || exit 1
log "run complete"
log "controller=$CONTROLLER flash_receiver=$FLASH_RECEIVER multiply_acct=${ALICE_MACCT:-n/a}"
bash "$HERE/assert_green.sh" || exit 1
