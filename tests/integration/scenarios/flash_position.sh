#!/usr/bin/env bash

# Focused live-testnet exercise of `flash_position` only: deploy current
# controller/pool wasm plus the mock receiver, list XLM/USDC, seed pool cash,
# then run the success path and the designed reverts.

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke assert wallet assets aggregator oracle protocol report; do
    source "$INTEG_DIR/lib/$f.sh"
done
source "$INTEG_DIR/flows/lifecycle.sh"
source "$INTEG_DIR/flows/flash_position.sh"

init_run
if [ -f "$INTEG_DIR/appendix.md" ]; then
    cp -n "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || cp "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || true
fi

check_tools 2>/dev/null || log "WARNING: some required tools missing (see check_tools)"
check_stellar_version 2>/dev/null || log "WARNING: stellar CLI version check failed or not met"

[ -f "$WASM_DIR/controller.wasm" ] \
    || die preflight "missing $WASM_DIR/controller.wasm (run make integration-wasm)"
[ -f "$WASM_DIR/position_nft.wasm" ] \
    || die preflight "missing $WASM_DIR/position_nft.wasm (run make integration-wasm)"
[ -f "$WASM_DIR/flash_position_receiver.wasm" ] \
    || die preflight "missing $WASM_DIR/flash_position_receiver.wasm (run make integration-wasm)"

trap 'write_report; run_summary' EXIT

phase wallets
new_wallet ADMIN admin
new_wallet ALICE alice
new_wallet BOB bob

phase deploy
deploy_protocol
[ -n "${FLASH_POSITION_RECEIVER:-}" ] \
    || die deploy_flash_position_receiver "FLASH_POSITION_RECEIVER unset after deploy_protocol"

flow_flash_position_markets || die markets "XLM/USDC market listing failed"
flow_flash_position_fund || die funding "USDC funding failed"
flow_seed_liquidity || die seed "pool seed failed"
flow_flash_position || die flash_position "flash_position live coverage failed"
if [ -z "${FP_MATRIX_DONE:-}" ]; then
    flow_flash_position_matrix || die flash_position_matrix "flash_position matrix failed"
else
    log "matrix already recorded; skipping"
fi
flow_flash_position_gaps || die flash_position_gaps "flash_position gap coverage failed"
flow_flash_position_malicious || die flash_position_malicious "malicious receiver coverage failed"

phase done
log "run complete"
log "flash_position account=${ALICE_FP_ACCT:-n/a} controller=$CONTROLLER receiver=${FLASH_POSITION_RECEIVER_V2:-$FLASH_POSITION_RECEIVER}"
bash "$HERE/assert_green.sh" || exit 1
