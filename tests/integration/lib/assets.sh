sac_live() {
    stellar contract invoke --id "$1" --source "$ADMIN" --network "$NETWORK" --send=no \
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
    sac=$(stellar contract id asset --asset "$asset" --network "$NETWORK")
    if sac_live "$sac"; then

        record "issue_sac_$code" ok "asset_id" "" "" "" "" "" "$sac (pre-existing)"
    else

        for attempt in $(seq 1 "$DEPLOY_MAX_ATTEMPTS"); do
            [ "$attempt" -gt 1 ] && backoff_sleep "$attempt" 3 15
            if stellar contract asset deploy --asset "$asset" --source "$ADMIN" \
                --network "$NETWORK" >"$out_f" 2>"$err_f"; then
                hash=$(extract_signing_hash "$err_f")
                break
            fi

            sac_live "$sac" && break
        done
        if ! sac_wait_live "$sac"; then
            die "issue_sac_$code" \
                "SAC $code not live after $DEPLOY_MAX_ATTEMPTS deploy attempts: $(tail_err_note "$err_f" 200)"
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
    local attempt
    for attempt in $(seq 1 "$INV_MAX_ATTEMPTS"); do
        [ "$attempt" -gt 1 ] && backoff_sleep "$attempt"
        if stellar tx new change-trust --source-account "$wallet" --line "$code:$issuer" \
            --network "$NETWORK" >"$LOG_DIR/$label.out" 2>"$err_f"; then
            local hash
            hash=$(grep -oE '[0-9a-f]{64}' "$err_f" | tail -1)
            record "$label" ok "change_trust" "$hash" "" "" "" "" "$code"
            return 0
        fi

        if [ "$attempt" -lt "$INV_MAX_ATTEMPTS" ] && grep -qE "$RPC_TRANSIENT_RE" "$err_f"; then
            record "$label" retry "change_trust" "" "" "" "" "" "transient rpc failure; retrying"
            continue
        fi
        break
    done
    record "$label" FAIL "change_trust" "" "" "" "" "" "$(tail -c 200 "$err_f" | tr '\n\t' '  ')"
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
    local sac="$1" who="$2"
    stellar contract invoke --id "$sac" --source "$ADMIN" --network "$NETWORK" --send=no \
        -- balance --id "$who" 2>/dev/null | tr -d '"'
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
