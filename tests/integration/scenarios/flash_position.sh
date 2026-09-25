#!/usr/bin/env bash

# Focused live-testnet exercise of `flash_position` only: deploy current
# controller/pool wasm plus the mock receiver, list XLM/USDC, seed pool cash,
# then run the success path, the designed reverts and the zero-state teardown.

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke assert wallet assets aggregator oracle protocol report; do
    source "$INTEG_DIR/lib/$f.sh"
done
source "$INTEG_DIR/flows/lifecycle.sh"
source "$INTEG_DIR/flows/flash_position.sh"
source "$INTEG_DIR/flows/teardown.sh"

E2E_LANE="${E2E_LANE:-flash}"
init_run
if [ -f "$INTEG_DIR/appendix.md" ]; then
    cp -n "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || cp "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || true
fi

check_tools || die preflight "required tool missing"
check_stellar_version || die preflight "wrong stellar CLI version"

[ -f "$WASM_DIR/controller.wasm" ] \
    || die preflight "missing $WASM_DIR/controller.wasm (run make integration-wasm)"
[ -f "$WASM_DIR/position_nft.wasm" ] \
    || die preflight "missing $WASM_DIR/position_nft.wasm (run make integration-wasm)"
[ -f "$FIXTURE_WASM_DIR/flash_position_receiver.wasm" ] \
    || die preflight "missing $FIXTURE_WASM_DIR/flash_position_receiver.wasm (run make integration-wasm)"

trap 'finish_run $?' EXIT
trap 'exit 130' INT TERM

wallets() {
    new_wallet ADMIN admin || return 1
    new_wallet ALICE alice || return 1
    new_wallet BOB bob || return 1
}
run_case wallets wallets || die wallets "wallet preflight failed"

phase deploy
run_case deploy_protocol deploy_protocol || die deploy_protocol "required case failed"
[ -n "${FLASH_POSITION_RECEIVER:-}" ] \
    || die deploy_flash_position_receiver "FLASH_POSITION_RECEIVER unset after deploy_protocol"

run_case flow_flash_position_markets flow_flash_position_markets || die flow_flash_position_markets "required case failed"
run_case flow_flash_position_fund flow_flash_position_fund || die flow_flash_position_fund "required case failed"
run_case flow_seed_liquidity flow_seed_liquidity || die flow_seed_liquidity "required case failed"
run_case flow_flash_position flow_flash_position || die flow_flash_position "required case failed"
if [ -z "${FP_MATRIX_DONE:-}" ]; then
    run_case flow_flash_position_matrix flow_flash_position_matrix || die flow_flash_position_matrix "required case failed"
else
    log "matrix already recorded; skipping"
fi
run_case flow_flash_position_gaps flow_flash_position_gaps || die flow_flash_position_gaps "required case failed"
run_case flow_flash_position_gates flow_flash_position_gates || die flow_flash_position_gates "required case failed"
run_case flow_flash_position_malicious flow_flash_position_malicious || die flow_flash_position_malicious "required case failed"
run_case flow_teardown flow_teardown || die flow_teardown "required case failed"

phase done
python3 "$INTEG_DIR/gate.py" "$RUN_DIR" || exit 1
log "run complete"
log "flash_position account=${ALICE_FP_ACCT:-n/a} controller=$CONTROLLER receiver=${FLASH_POSITION_RECEIVER_V2:-$FLASH_POSITION_RECEIVER}"
bash "$HERE/assert_green.sh" || exit 1
