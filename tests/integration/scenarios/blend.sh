#!/usr/bin/env bash

# Focused live-testnet migrate_from_blend against the real Blend TestnetV2
# pool. Covers allowlist, input rejects, market-flag rejects, collateral /
# supply / debt migrates, zero-liability refund, existing-account merge,
# delegate migrate, remigrate-empty, and Blend-side health/cap/min-borrow
# failures that a website happy-path does not hit.

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke assert wallet assets aggregator oracle protocol report; do
    source "$INTEG_DIR/lib/$f.sh"
done
source "$INTEG_DIR/flows/lifecycle.sh"
source "$INTEG_DIR/flows/blend.sh"
source "$INTEG_DIR/flows/teardown.sh"

init_run
if [ -f "$INTEG_DIR/appendix.md" ]; then
    cp -n "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || cp "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || true
fi

check_tools 2>/dev/null || log "WARNING: some required tools missing (see check_tools)"
check_stellar_version 2>/dev/null || log "WARNING: stellar CLI version check failed or not met"

[ -f "$WASM_DIR/controller.wasm" ] \
    || die preflight "missing $WASM_DIR/controller.wasm (run make integration-wasm)"

trap 'write_report; run_summary' EXIT

phase wallets
new_wallet ADMIN admin
new_wallet ALICE alice
new_wallet BOB bob
new_wallet CAROL carol
new_wallet DAVE dave
new_wallet EVE eve
new_wallet FRANK frank

phase deploy
deploy_protocol

flow_real_markets || die markets "XLM/USDC/EURC market listing failed"
# Native XLM only: Blend TestnetV2 USDC is not the protocol USDC SAC, and the
# aggregator swap is unrelated to migrate_from_blend accounting.
flow_blend_hub_liquidity || die seed "XLM hub seed failed"

flow_blend_allowlist || die blend_allowlist "Blend allowlist failed"
flow_blend_rejects || die blend_rejects "Blend reject coverage failed"
flow_blend_migrate || die blend_migrate "Blend migrate coverage failed"
flow_teardown || die teardown "zero-state teardown failed"

phase done
log "run complete"
log "controller=$CONTROLLER blend_pool=$BLEND_POOL alice_acct=${ALICE_BLEND_ACCT:-n/a}"
bash "$HERE/assert_green.sh" || exit 1
