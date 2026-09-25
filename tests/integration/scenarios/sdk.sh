#!/usr/bin/env bash
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke assert wallet assets aggregator oracle protocol report; do source "$INTEG_DIR/lib/$f.sh"; done
for f in lifecycle blend strategies sdk production teardown; do source "$INTEG_DIR/flows/$f.sh"; done
E2E_LANE=sdk
init_run
trap 'finish_run $?' EXIT
trap 'exit 130' INT TERM
check_tools || die preflight "required tools missing"
check_stellar_version || die preflight "CLI version mismatch"
wallets() {
    new_wallet ADMIN admin || return 1
    new_wallet ALICE alice || return 1
    new_wallet BOB bob || return 1
    new_wallet CAROL carol || return 1
    new_wallet DAVE dave || return 1
}
run_case wallets wallets || die wallets "funding failed"
run_case deploy_protocol deploy_protocol || die deploy_protocol "required case failed"
run_case flow_real_markets flow_real_markets || die flow_real_markets "required case failed"
run_case flow_fund_usdc flow_fund_usdc || die flow_fund_usdc "required case failed"
run_case flow_seed_liquidity flow_seed_liquidity || die flow_seed_liquidity "required case failed"
run_case flow_blend_allowlist flow_blend_allowlist || die flow_blend_allowlist "required case failed"
run_case flow_sdk_lifecycle flow_sdk_lifecycle || die flow_sdk_lifecycle "required case failed"
run_case flow_sdk_strategy flow_sdk_strategy || die flow_sdk_strategy "required case failed"
run_case flow_sdk_blend flow_sdk_blend || die flow_sdk_blend "required case failed"
run_case flow_sdk_errors flow_sdk_errors || die flow_sdk_errors "required case failed"
run_case flow_teardown flow_teardown || die flow_teardown "required case failed"
python3 "$INTEG_DIR/gate.py" "$RUN_DIR" || exit 1
log "run complete"
