#!/usr/bin/env bash
# Event-cap width walk 8..5. Prefer liq20_v2_walk.sh for instruction-cap research.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke wallet assets aggregator oracle protocol report liq20_width; do
    source "$INTEG_DIR/lib/$f.sh"
done
source "$INTEG_DIR/flows/stress.sh"
init_run
trap 'write_report; run_summary' EXIT
ACCT="${LIQ20_ACCT:?}"

phase liq20_walk
liq20_events_width_walk LIQ20_FULLREPAY_WIDTH 8 7 6 5
phase done
log "walk complete"