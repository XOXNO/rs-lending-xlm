#!/usr/bin/env bash
# Thin wrapper around liq20_send_9debt_leeway (folded into lib/liq20_width.sh).
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

phase liq20_retry9
liq20_send_9debt_leeway
phase done
log "retry complete"