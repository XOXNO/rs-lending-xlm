#!/usr/bin/env bash

# Focused live-testnet migrate_from_blend against the real Blend TestnetV2
# pool. Covers allowlist, input rejects, market-flag rejects, collateral /
# supply / debt migrates, the zero-liability reject, existing-account merge,
# delegate migrate, remigrate-empty, and the health, debt-cap and min-borrow
# failures that a web-app happy path does not reach.

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke assert wallet assets aggregator oracle protocol report; do
    source "$INTEG_DIR/lib/$f.sh"
done
source "$INTEG_DIR/flows/lifecycle.sh"
source "$INTEG_DIR/flows/blend.sh"
source "$INTEG_DIR/flows/teardown.sh"

E2E_LANE="${E2E_LANE:-blend}"
init_run
if [ -f "$INTEG_DIR/appendix.md" ]; then
    cp -n "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || cp "$INTEG_DIR/appendix.md" "$RUN_DIR/appendix.md" 2>/dev/null || true
fi

check_tools || die preflight "required tool missing"
check_stellar_version || die preflight "wrong stellar CLI version"

[ -f "$WASM_DIR/controller.wasm" ] \
    || die preflight "missing $WASM_DIR/controller.wasm (run make integration-wasm)"

trap 'finish_run $?' EXIT
trap 'exit 130' INT TERM

wallets() {
    new_wallet ADMIN admin || return 1
    new_wallet ALICE alice || return 1
    new_wallet BOB bob || return 1
    new_wallet CAROL carol || return 1
    new_wallet DAVE dave || return 1
    new_wallet EVE eve || return 1
    new_wallet FRANK frank || return 1
}
run_case wallets wallets || die wallets "wallet preflight failed"

phase deploy
run_case deploy_protocol deploy_protocol || die deploy_protocol "required case failed"

run_case flow_real_markets flow_real_markets || die flow_real_markets "required case failed"
# Native XLM only: Blend TestnetV2 USDC is not the protocol USDC SAC, and the
# aggregator swap is unrelated to migrate_from_blend accounting.
run_case flow_blend_hub_liquidity flow_blend_hub_liquidity || die flow_blend_hub_liquidity "required case failed"

run_case flow_blend_allowlist flow_blend_allowlist || die flow_blend_allowlist "required case failed"
run_case flow_blend_rejects flow_blend_rejects || die flow_blend_rejects "required case failed"
run_case flow_blend_migrate flow_blend_migrate || die flow_blend_migrate "required case failed"
run_case flow_blend_multireserve flow_blend_multireserve || die flow_blend_multireserve "required case failed"
run_case flow_teardown flow_teardown || die flow_teardown "required case failed"

phase done
python3 "$INTEG_DIR/gate.py" "$RUN_DIR" || exit 1
log "run complete"
log "controller=$CONTROLLER blend_pool=$BLEND_POOL alice_acct=${ALICE_BLEND_ACCT:-n/a}"
bash "$HERE/assert_green.sh" || exit 1
