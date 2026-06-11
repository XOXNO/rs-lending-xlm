#!/usr/bin/env bash
# V2-events width walk: widest n-debt repay + 10-coll seize. Events no longer
# bind (~12KB at n=10); the 400M instruction cap decides. Leeway is computed
# from the simulated count so the declared budget hugs the cap.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/../env.sh"
for f in core invoke wallet assets aggregator oracle protocol report; do source "$INTEG_DIR/lib/$f.sh"; done
source "$INTEG_DIR/flows/stress.sh"
init_run
trap 'write_report; run_summary' EXIT
ACCT="${LIQ20_ACCT:?}"
TX_CAP=400000000
REPAY_EACH=$((3000 * STRESS_UNIT))

phase liq20_v2_walk
for n in 10 9 8; do
    args=""
    for i in $(seq 10 $((9 + n))); do args+=" $(stress_sac $i) $REPAY_EACH"; done
    sim_probe "v2_probe_${n}debt_10coll" "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
        --debt_payments "$(pay_vec $args)"
    if [ "$PROBE_STATUS" != ok ] || [ -z "$RES_INSTR" ]; then
        log "width $n: probe $PROBE_STATUS"
        continue
    fi
    headroom=$((TX_CAP - RES_INSTR))
    log "width $n: sim $RES_INSTR insns, headroom $headroom"
    if [ "$headroom" -le 200000 ]; then
        log "width $n: declared budget cannot fit drift headroom — skipping send"
        continue
    fi
    leeway=$((headroom - 100000))
    [ "$leeway" -gt 8000000 ] && leeway=8000000
    label="liq20_v2_proof_${n}debt"
    if stellar contract invoke --id "$CONTROLLER" --source "$CAROL" --network "$NETWORK" \
        --instruction-leeway "$leeway" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
        --debt_payments "$(pay_vec $args)" \
        >"$LOG_DIR/$label.out" 2>"$LOG_DIR/$label.err"; then
        hash=$(grep -oE 'Signing transaction: [0-9a-f]{64}' "$LOG_DIR/$label.err" | tail -1 | awk '{print $3}')
        fetch_resources "$hash"
        record "$label" ok liquidate "$hash" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" ""
        log "width $n LANDED ($RES_INSTR insns) tx=$hash"
        save_state LIQ20_V2_WIDTH "$n"
        break
    fi
    record "$label" FAIL liquidate "" "" "" "" "" "$(tail -c 200 "$LOG_DIR/$label.err" | tr '\n\t' '  ')"
    log "width $n send failed: $(grep -oE 'Trapped|ResourceLimitExceeded|TxSorobanInvalid|events size[^\"]*' "$LOG_DIR/$label.err" | head -2 | tr '\n' ' ')"
done
phase done
log "v2 walk complete"
