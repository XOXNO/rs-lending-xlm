wallet_funded() {
    curl --fail-with-body -sS -m 30 "https://horizon-testnet.stellar.org/accounts/$1" > "$2" \
        && jq -e '.balances | any(.asset_type == "native" and (.balance | tonumber) >= 100)' "$2" >/dev/null
}

fund_wallet() {
    local alias="$1"
    stellar keys address "$alias" >/dev/null 2>&1 && return 0
    stellar keys generate "$alias" "${NET_ARGS[@]}" --fund >/dev/null 2>&1 && return 0
    stellar keys generate "$alias" "${NET_ARGS[@]}" >/dev/null 2>&1 || return 1
    curl -s -m 30 "https://friendbot.stellar.org/?addr=$(stellar keys address "$alias")" >/dev/null 2>&1
}

prefund_wallets() {
    local role pid pids=()
    for role in "$@"; do
        fund_wallet "e2e_${role}_${RUN_TS}" & pids+=("$!")
    done
    for pid in "${pids[@]}"; do wait "$pid" || true; done
}

new_wallet() {
    local var="$1" role="$2"
    local alias="e2e_${role}_${RUN_TS}"
    local addr_var="${var}_ADDR"
    if [ -n "${!addr_var:-}" ]; then
        log "wallet $role resumed: ${!addr_var}"
        return 0
    fi
    if ! stellar keys address "$alias" >/dev/null 2>&1; then
        log "generating + funding wallet $alias"
        stellar keys generate "$alias" "${NET_ARGS[@]}" --fund >/dev/null 2>&1 \
            || stellar keys generate "$alias" "${NET_ARGS[@]}" >/dev/null
    fi
    local addr
    addr=$(stellar keys address "$alias")

    save_state "$var" "$alias"
    save_state "$addr_var" "$addr"
    local funding="$LOG_DIR/wallet_${role}_funding.json"
    wallet_funded "$addr" "$funding" \
        || { curl -s -m 30 "https://friendbot.stellar.org/?addr=$addr" >/dev/null 2>&1; wallet_funded "$addr" "$funding"; } \
        || die "wallet_$role" "funding not confirmed (minimum 100 XLM)"
    record "wallet_$role" ok "friendbot" "" "" "" "" "" "$addr"
    log "wallet $role = $addr"
}
