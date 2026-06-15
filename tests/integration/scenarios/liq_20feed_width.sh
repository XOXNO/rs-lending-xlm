#!/usr/bin/env bash
# Narrow event-cap probe (4..2 debts). Research-only — failures are status=research.
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

phase liq20_width
liq20_events_width_walk LIQ20_FULLREPAY_WIDTH 4 3 2
phase done
log "width search complete"