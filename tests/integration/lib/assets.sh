sac_live() {
    stellar contract invoke --instruction-leeway "${INSTRUCTION_LEEWAY:-2000000}" --id "$1" --source "$ADMIN" "${NET_ARGS[@]}" --send=no \
        -- decimals >/dev/null 2>&1
}

sac_wait_live() {
    local probe
    for probe in $(seq 1 10); do
        sac_live "$1" && return 0
        sleep 2
    done
    return 1
}

issue_sac() {
    local var="$1" code="$2"
    if [ -n "${!var:-}" ]; then return 0; fi
    local asset="$code:$ADMIN_ADDR"
    local out_f="$LOG_DIR/sac_$code.out" err_f="$LOG_DIR/sac_$code.err"
    local sac hash attempt
    sac=$(stellar contract id asset --asset "$asset" "${NET_ARGS[@]}")
    if sac_live "$sac"; then

        record "issue_sac_$code" ok "asset_id" "" "" "" "" "" "$sac (pre-existing)"
    else

        local rc=0
        stellar contract asset deploy --asset "$asset" --source "$ADMIN" \
            "${NET_ARGS[@]}" >"$out_f" 2>"$err_f" || rc=$?
        hash=$(extract_signing_hash "$err_f")
        if [ -z "$hash" ] || [ "$(tx_status "$hash")" != SUCCESS ] || [ "$rc" -ne 0 ]; then
            record "issue_sac_$code" FAIL asset_deploy "$hash" "" "" "" "" "SAC deployment unconfirmed; never resubmit"
            return 1
        fi
        if ! sac_wait_live "$sac"; then
            die "issue_sac_$code" \
                "SAC $code not live after ${attempt:-0} deploy attempt(s): $(tail_err_note "$err_f" 200)"
        fi
        record "issue_sac_$code" ok "asset_deploy" "${hash:-}" "" "" "" "" "$sac"
    fi
    save_state "$var" "$sac"
    log "SAC $code = $sac"
}

trustline() {
    local wallet="$1" code="$2" issuer="$3"
    local label="trust_${code}_${wallet%%_e2e*}"
    local err_f="$LOG_DIR/$label.err"
    local rc=0 hash
    stellar tx new change-trust --source-account "$wallet" --line "$code:$issuer" \
        "${NET_ARGS[@]}" >"$LOG_DIR/$label.out" 2>"$err_f" || rc=$?
    hash=$(extract_signing_hash "$err_f")
    if [ -n "$hash" ] && [ "$(tx_status "$hash")" = SUCCESS ] && [ "$rc" -eq 0 ]; then
        record "$label" ok change_trust "$hash" "" "" "" "" "$code" classic_transaction
        return 0
    fi
    record "$label" FAIL change_trust "$hash" "" "" "" "" "unconfirmed trustline; never resubmit: $(tail_err_note "$err_f")"
    return 1
}

mint_to() {
    local sac="$1" code="$2" to="$3" amount="$4"

    local bal
    bal=$(balance "$sac" "$to" 2>/dev/null)
    if [[ "$bal" =~ ^[0-9]+$ ]] && _uint_ge "$bal" "$amount"; then
        record "mint_${code}_to_${to:0:6}" ok mint "" "" "" "" "" "holder already funded (resume); skipping mint"
        return 0
    fi
    INV_TRANSIENT_CONTRACT_RE='trustline entry is missing' \
        inv "mint_${code}_to_${to:0:6}" "$ADMIN" "$sac" -- mint --to "$to" --amount "$amount" >/dev/null
}

balance() {
    local sac="$1" who="$2" value label="balance_${1:0:8}_${2:0:8}"
    value=$(view "$label" "$sac" -- balance --id "$who") || return 1
    value=$(tr -d '\"[:space:]' <<<"$value")
    [[ "$value" =~ ^[0-9]+$ ]] || { _assert_fail "$label" "invalid balance '$value'"; return 1; }
    printf '%s\n' "$value"
}

sac_transfer() {
    local signer="$1" sac="$2" from="$3" to="$4" amount="$5" label="$6"
    inv "$label" "$signer" "$sac" -- transfer --from "$from" --to "$to" --amount "$amount" >/dev/null
}

swap_xlm_to() {
    local wallet="$1" addr="$2" to_sac="$3" amount_in="$4" label="$5"
    local swap_hex
    swap_hex=$(agg_route_hex "$XLM_SAC" "$to_sac" "$amount_in") || {
        record "$label" FAIL execute_strategy "" "" "" "" "" "no aggregator route"
        return 1
    }
    inv "$label" "$wallet" "$AGGREGATOR" -- execute_strategy \
        --sender "$addr" --total_in "$amount_in" --swap_xdr "$swap_hex" >/dev/null
}

# Compare economic token movement separately from Stellar transaction fees.
# Raw balance() remains unchanged; fee evidence is retained beside receipts.
financial_balance() {
    local token="$1" address="$2" raw
    raw=$(balance "$token" "$address") || return 1
    if [ "$token" = "$XLM_SAC" ] && [[ "$address" = G* ]]; then
        python3 "$INTEG_DIR/network_fees.py" "$LOG_DIR" "$address" "$raw"
    else
        printf '%s\n' "$raw"
    fi
}
