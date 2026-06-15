#!/usr/bin/env bash
# Maximal liquidation shape: sim-probe widest n-debt repay at full 10-coll seize.
#
#   RUN_TS=<existing> bash tests/integration/scenarios/liq_20feed_fullrepay.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke assert wallet assets aggregator oracle protocol report liq20_width; do
    source "$INTEG_DIR/lib/$f.sh"
done
source "$INTEG_DIR/flows/stress.sh"

init_run
trap 'write_report; run_summary' EXIT

ACCT="${LIQ20_ACCT:?run liq_20feed.sh first}"

phase liq20_fullrepay
assert_can_liquidated liq20f_can_liq "$ACCT" true
liq20_fullrepay_probe LIQ20_FULLREPAY_WIDTH 10 8 6 4 2
phase done
log "fullrepay scenario complete"