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

    curl -s -m 30 "https://friendbot.stellar.org/?addr=$addr" >/dev/null 2>&1 || true
    save_state "$var" "$alias"
    save_state "$addr_var" "$addr"
    local funding="$LOG_DIR/wallet_${role}_funding.json"
    curl --fail-with-body -sS -m 30 "https://horizon-testnet.stellar.org/accounts/$addr" > "$funding" \
        && jq -e '.balances | any(.asset_type == "native" and (.balance | tonumber) >= 100)' "$funding" >/dev/null \
        || die "wallet_$role" "funding not confirmed (minimum 100 XLM)"
    record "wallet_$role" ok "friendbot" "" "" "" "" "" "$addr"
    log "wallet $role = $addr"
}
