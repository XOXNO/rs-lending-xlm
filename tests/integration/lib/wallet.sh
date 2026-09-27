wallet_funded() {
    local n=0 code
    while ! code=$(curl --fail-with-body -sS -m 30 -o "$2" -w '%{http_code}' "https://horizon-testnet.stellar.org/accounts/$1"); do
        n=$((n + 1))
        [ "$code" = 429 ] && throttle_sleep "$n" || return 1
    done
    jq -e '.balances | any(.asset_type == "native" and (.balance | tonumber) >= 100)' "$2" >/dev/null
}

friendbot_fund() {
    local alias="$1" addr code slot rc=1 attempt=0 deadline base="$LOG_DIR/friendbot_$1"
    stellar keys address "$alias" >/dev/null 2>&1 \
        || stellar keys generate "$alias" "${NET_ARGS[@]}" >/dev/null 2>&1 || return 1
    addr=$(stellar keys address "$alias") || return 1
    slot_take "$INTEG_DIR/runs/.slots/friendbot" "${E2E_FRIENDBOT_SLOTS:-6}" 30 300 \
        || { printf 'slot-unavailable\n' >> "$base.codes"; return 2; }
    slot="$SLOT_FD"
    deadline=$(( $(date +%s) + 90 ))
    while :; do
        attempt=$((attempt + 1))
        code=$(curl -sS -m 30 -o "$base.json" -w '%{http_code}' "https://friendbot.stellar.org/?addr=$addr" 2>>"$base.err") || :
        code="${code:-000}"
        printf '%s\n' "$code" >> "$base.codes"
        if wallet_funded "$addr" "$base.balance.json"; then rc=0; break; fi
        case "$code" in 429|5[0-9][0-9]|000) ;; *) break;; esac
        [ "$(date +%s)" -lt "$deadline" ] || break
        backoff_sleep "$((attempt + 1))"
    done
    eval "exec $slot>&-"
    return "$rc"
}

fund_wallet() {
    stellar keys address "$1" >/dev/null 2>&1 && return 0
    friendbot_fund "$1"
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
    local addr funding="$LOG_DIR/wallet_${role}_funding.json"
    if ! addr=$(stellar keys address "$alias" 2>/dev/null) || ! wallet_funded "$addr" "$funding"; then
        log "funding wallet $alias"
        friendbot_fund "$alias" || {
            [ $? -ne 2 ] || die "wallet_$role" "no friendbot slot free within 300 s (E2E_FRIENDBOT_SLOTS=${E2E_FRIENDBOT_SLOTS:-6})"
            die "wallet_$role" "funding not confirmed (minimum 100 XLM)"
        }
        addr=$(stellar keys address "$alias") || die "wallet_$role" "funded key $alias is missing"
    fi
    save_state "$var" "$alias"
    save_state "$addr_var" "$addr"
    record "wallet_$role" ok "friendbot" "" "" "" "" "" "$addr"
    log "wallet $role = $addr"
}

lane_channels() {
    local n="$1" i alias addr chans='' pids=() pid
    for i in $(seq 1 "$n"); do fund_wallet "e2e_chan${i}_${RUN_TS}" & pids+=("$!"); done
    for pid in "${pids[@]}"; do wait "$pid" || true; done
    for i in $(seq 1 "$n"); do
        alias="e2e_chan${i}_${RUN_TS}"
        addr=$(stellar keys address "$alias") || { _assert_fail "lane_channel_$i" 'channel key missing'; return 1; }
        wallet_funded "$addr" "$LOG_DIR/channel_${i}_funding.json" || friendbot_fund "$alias" || {
            [ $? -ne 2 ] || { _assert_fail "lane_channel_$i" 'no friendbot slot free within 300 s'; return 1; }
            _assert_fail "lane_channel_$i" 'channel funding not confirmed (minimum 100 XLM)'; return 1
        }
        chans="$chans $alias"
    done
    save_state CHANNELS "${chans# }"
}
