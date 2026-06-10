# Wallet management: per-run unique aliases funded by friendbot.
#
# Wallet depletion is the #1 flakiness source across runs (supplies lock XLM
# and friendbot funds an address only once) — every run uses fresh aliases
# namespaced by RUN_TS so each wallet starts with the full 10,000 XLM grant.

# Creates (or resumes) a funded wallet. Sets <VAR> to the alias and
# <VAR>_ADDR to the G-address; persists both into state.env.
#   new_wallet <VAR> <role>
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
        stellar keys generate "$alias" --network "$NETWORK" --fund >/dev/null 2>&1 \
            || stellar keys generate "$alias" --network "$NETWORK" >/dev/null
    fi
    local addr
    addr=$(stellar keys address "$alias")
    # Friendbot is idempotent-safe to retry: a second call on a funded
    # account fails harmlessly.
    curl -s -m 30 "https://friendbot.stellar.org/?addr=$addr" >/dev/null 2>&1 || true
    save_state "$var" "$alias"
    save_state "$addr_var" "$addr"
    record "wallet_$role" ok "friendbot" "" "" "" "" "" "$addr"
    log "wallet $role = $addr"
}
