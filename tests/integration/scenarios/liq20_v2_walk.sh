#!/usr/bin/env bash

set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke wallet assets aggregator oracle protocol report liq20_width; do
    source "$INTEG_DIR/lib/$f.sh"
done
source "$INTEG_DIR/flows/stress.sh"
init_run
trap 'write_report; run_summary' EXIT
ACCT="${LIQ20_ACCT:?run liq_20feed.sh first}"

phase liq20_v2_walk
liq20_v2_walk_widths LIQ20_V2_WIDTH 10 9 8
phase done
log "v2 walk complete"
