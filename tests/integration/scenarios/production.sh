#!/usr/bin/env bash
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke assert wallet assets aggregator oracle protocol report; do source "$INTEG_DIR/lib/$f.sh"; done
for f in lifecycle blend sdk production teardown; do source "$INTEG_DIR/flows/$f.sh"; done
E2E_LANE=production
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
run_case flow_production_fixtures flow_production_fixtures || die flow_production_fixtures "required case failed"
run_case flow_production_operator flow_production_operator || die flow_production_operator "required case failed"
run_case flow_production_lending flow_production_lending || die flow_production_lending "required case failed"
run_case flow_production_caller flow_production_caller || die flow_production_caller "required case failed"
run_case flow_teardown flow_teardown || die flow_teardown "required case failed"
python3 "$INTEG_DIR/gate.py" "$RUN_DIR" || exit 1
log "run complete"
