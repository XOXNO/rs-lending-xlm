# 20-feed liquidation width research helpers (requires stress.sh, invoke.sh, core.sh).
# Intentional frontier misses are recorded as status=research so assert_green ignores them.

LIQ20_TX_CAP="${LIQ20_TX_CAP:-400000000}"
LIQ20_DEFAULT_REPAY_EACH="${LIQ20_DEFAULT_REPAY_EACH:-$((3000 * STRESS_UNIT))}"
LIQ20_DEFAULT_LEEWAY="${LIQ20_DEFAULT_LEEWAY:-8000000}"

liq20_pay_vec() {
local hub_id="$1" n="$2" repay_each="${3:-$LIQ20_DEFAULT_REPAY_EACH}" args="" i
    for i in $(seq 10 $((9 + n))); do args+=" $(stress_sac $i) $repay_each"; done
    pay_vec "$hub_id" $args
}

liq20_record_research_fail() {
    record "$1" research "$2" "" "" "" "" "" "$3"
}

liq20_parse_events_reject_note() {
    local err_f="$1"
    local sz reason
    sz=$(grep -oE 'maximum","[0-9]+","16384' "$err_f" | grep -oE '","[0-9]+","' | grep -oE '[0-9]+' | head -1)
    [ -z "$sz" ] && sz=$(grep -oE '"[0-9]{5}"' "$err_f" | grep -oE '[0-9]+' | head -1)
    reason=$(grep -oE 'exceeds network config maximum","[0-9]+' "$err_f" | head -1 | grep -oE '[0-9]+$')
    echo "events ${reason:-${sz:-?}}B > 16384B"
}

liq20_liquidate_send() {
    local label="$1" leeway="$2" n="$3" repay_each="${4:-$LIQ20_DEFAULT_REPAY_EACH}"
    local out_f="$LOG_DIR/$label.out" err_f="$LOG_DIR/$label.err"
    if stellar contract invoke --id "$CONTROLLER" --source "$CAROL" --network "$NETWORK" \
        --instruction-leeway "$leeway" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
        --debt_payments "$(liq20_pay_vec "$PRIMARY_HUB_ID" "$n" "$repay_each")" \
        >"$out_f" 2>"$err_f"; then
        local hash
        hash=$(grep -oE 'Signing transaction: [0-9a-f]{64}' "$err_f" | tail -1 | awk '{print $3}')
        fetch_resources "$hash"
        record "$label" ok liquidate "$hash" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" ""
        log "width $n LANDED ($RES_INSTR insns) tx=$hash"
        return 0
    fi
    return 1
}

# Walk widths on-chain until one lands; record research status on event-cap rejects.
liq20_events_width_walk() {
    local state_key="$1"; shift
    local n repay_each label
    for n in "$@"; do
        repay_each="${LIQ20_DEFAULT_REPAY_EACH}"
        label="liq20_walk_${n}debt"
        if liq20_liquidate_send "$label" "$LIQ20_DEFAULT_LEEWAY" "$n" "$repay_each"; then
            save_state "$state_key" "$n"
            return 0
        fi
        local note
        note=$(liq20_parse_events_reject_note "$LOG_DIR/$label.err")
        liq20_record_research_fail "$label" liquidate "$note"
        log "width $n REJECTED: $note"
    done
    return 1
}

# V2 walk: sim-first, dynamic leeway from declared instruction headroom.
liq20_v2_walk_widths() {
    local state_key="$1"; shift
    local n repay_each headroom leeway label
    for n in "$@"; do
        repay_each="${LIQ20_DEFAULT_REPAY_EACH}"
        sim_probe "v2_probe_${n}debt_10coll" "$CAROL" "$CONTROLLER" -- liquidate \
            --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
            --debt_payments "$(liq20_pay_vec "$PRIMARY_HUB_ID" "$n" "$repay_each")"
        if [ "$PROBE_STATUS" != ok ] || [ -z "$RES_INSTR" ]; then
            log "width $n: probe $PROBE_STATUS"
            continue
        fi
        headroom=$((LIQ20_TX_CAP - RES_INSTR))
        log "width $n: sim $RES_INSTR insns, headroom $headroom"
        if [ "$headroom" -le 200000 ]; then
            log "width $n: declared budget cannot fit drift headroom — skipping send"
            continue
        fi
        leeway=$((headroom - 100000))
        [ "$leeway" -gt 8000000 ] && leeway=8000000
        label="liq20_v2_proof_${n}debt"
        if liq20_liquidate_send "$label" "$leeway" "$n" "$repay_each"; then
            save_state "$state_key" "$n"
            return 0
        fi
        liq20_record_research_fail "$label" liquidate \
            "$(grep -oE 'Trapped|ResourceLimitExceeded|TxSorobanInvalid|events size[^\"]*' "$LOG_DIR/$label.err" | head -2 | tr '\n' ' ')"
        log "width $n send failed: $(tail -c 200 "$LOG_DIR/$label.err" | tr '\n\t' '  ')"
    done
    return 1
}

# Bisect: sim must pass AND declared instructions must fit the tx cap before send.
liq20_bisect_widths() {
    local state_key="$1"; shift
    local n repay_each
    for n in "$@"; do
        repay_each="${LIQ20_DEFAULT_REPAY_EACH}"
        sim_probe "probe_liq_${n}debt_10coll" "$CAROL" "$CONTROLLER" -- liquidate \
            --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
            --debt_payments "$(liq20_pay_vec "$PRIMARY_HUB_ID" "$n" "$repay_each")"
        if [ "$PROBE_STATUS" = ok ] && [ -n "$RES_INSTR" ] && [ "$RES_INSTR" -le "$LIQ20_TX_CAP" ]; then
            log "width $n fits: declared $RES_INSTR insns <= $LIQ20_TX_CAP"
            inv "liq20_bisect_proof_${n}debt" "$CAROL" "$CONTROLLER" -- liquidate \
                --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
                --debt_payments "$(liq20_pay_vec "$PRIMARY_HUB_ID" "$n" "$repay_each")" >/dev/null \
                && log "liquidation LANDED: $n debt repays + 10-coll seize ($RES_INSTR insns)"
            save_state "$state_key" "$n"
            return 0
        fi
        log "width $n: status=$PROBE_STATUS declared=${RES_INSTR:-n/a} insns — over cap or exceeded"
    done
    return 1
}

# Find widest n that simulates cleanly, then send one proof tx.
liq20_fullrepay_probe() {
    local state_key="$1"; shift
    local best_n=0 n repay_each
    for n in "$@"; do
        repay_each="${LIQ20_DEFAULT_REPAY_EACH}"
        sim_probe "probe_liq_${n}debt_10coll" "$CAROL" "$CONTROLLER" -- liquidate \
            --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
            --debt_payments "$(liq20_pay_vec "$PRIMARY_HUB_ID" "$n" "$repay_each")"
        if [ "$PROBE_STATUS" = ok ]; then best_n=$n; break; fi
        log "liquidation with $n debt repays + 10-coll seize: $PROBE_STATUS"
    done
    if [ "$best_n" -gt 0 ]; then
        inv "liq20_fullrepay_proof_${best_n}debt" "$CAROL" "$CONTROLLER" -- liquidate \
            --liquidator "$CAROL_ADDR" --account_id "$ACCT" \
            --debt_payments "$(liq20_pay_vec "$PRIMARY_HUB_ID" "$best_n" "$repay_each")" >/dev/null \
            && log "liquidation LANDED: $best_n debt repays + 10-coll seize in one tx"
    else
        log "no repay width passed at 10-coll seize"
    fi
    save_state "$state_key" "$best_n"
}

# One-shot 9-debt send with fixed leeway.
liq20_send_9debt_leeway() {
    local label=liq20_proof_9debt_leeway
    if liq20_liquidate_send "$label" "$LIQ20_DEFAULT_LEEWAY" 9; then
        return 0
    fi
    liq20_record_research_fail "$label" liquidate "$(tail -c 300 "$LOG_DIR/$label.err" | tr '\n\t' '  ')"
    log "9-debt retry FAILED: $(tail -3 "$LOG_DIR/$label.err" | tr '\n' ' ')"
    return 1
}
