#!/usr/bin/env bash
# Instruction-cap bisect (9..7 debts). Research-only.
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

phase liq20_bisect
liq20_bisect_widths LIQ20_FULLREPAY_WIDTH 9 8 7
phase done
log "bisect complete"