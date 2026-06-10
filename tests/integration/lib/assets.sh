# Token plumbing: self-issued SACs, trustlines, minting, balances, and
# funding wallets with real USDC/EURC by swapping XLM through the aggregator.
#
# Encoded testnet facts:
# - Classic-asset SAC balances on G-accounts need a change-trust trustline
#   first; `mint` to the issuer itself reverts (#2) — mint to trustline
#   holders only.
# - i128 inside a Vec<(Address,i128)> JSON arg must be a QUOTED string;
#   scalar i128 flags take bare numbers.
# - Back-to-back aggregator swaps trip min-out (#5) on the stale pool
#   snapshot — fund with ONE swap then SAC-transfer to other wallets.

# Deploys a fresh SAC for CODE issued by the admin wallet.
# Sets <VAR>=<sac-contract-id>; persists into state.env.
#   issue_sac <VAR> <CODE>
issue_sac() {
    local var="$1" code="$2"
    if [ -n "${!var:-}" ]; then return 0; fi
    local asset="$code:$ADMIN_ADDR"
    local out_f="$LOG_DIR/sac_$code.out" err_f="$LOG_DIR/sac_$code.err"
    if stellar contract asset deploy --asset "$asset" --source "$ADMIN" --network "$NETWORK" \
        >"$out_f" 2>"$err_f"; then
        local sac hash
        sac=$(tr -d '"\n' < "$out_f")
        hash=$(grep -oE 'Signing transaction: [0-9a-f]{64}' "$err_f" | tail -1 | awk '{print $3}')
        record "issue_sac_$code" ok "asset_deploy" "$hash" "" "" "" "" "$sac"
    else
        # Already deployed (resume): derive the deterministic id.
        local sac
        sac=$(stellar contract id asset --asset "$asset" --network "$NETWORK")
        record "issue_sac_$code" ok "asset_id" "" "" "" "" "" "$sac (pre-existing)"
    fi
    sac=$(stellar contract id asset --asset "$asset" --network "$NETWORK")
    save_state "$var" "$sac"
    log "SAC $code = $sac"
}

# Adds a classic trustline for CODE:ISSUER to a wallet.
#   trustline <wallet-alias> <CODE> <issuer-G-address>
trustline() {
    local wallet="$1" code="$2" issuer="$3"
    local label="trust_${code}_${wallet%%_e2e*}"
    local err_f="$LOG_DIR/$label.err"
    if stellar tx new change-trust --source-account "$wallet" --line "$code:$issuer" \
        --network "$NETWORK" >"$LOG_DIR/$label.out" 2>"$err_f"; then
        local hash
        hash=$(grep -oE '[0-9a-f]{64}' "$err_f" | tail -1)
        record "$label" ok "change_trust" "$hash" "" "" "" "" "$code"
    else
        record "$label" FAIL "change_trust" "" "" "" "" "" "$(tail -c 200 "$err_f" | tr '\n\t' '  ')"
        return 1
    fi
}

# Mints self-issued SAC units to a trustline holder (NOT the issuer).
#   mint_to <sac-id> <CODE> <to-G-address> <amount>
mint_to() {
    local sac="$1" code="$2" to="$3" amount="$4"
    inv "mint_${code}_to_${to:0:6}" "$ADMIN" "$sac" -- mint --to "$to" --amount "$amount" >/dev/null
}

#   balance <sac-id> <G-or-C-address>  → prints i128 balance
balance() {
    local sac="$1" who="$2"
    stellar contract invoke --id "$sac" --source "$ADMIN" --network "$NETWORK" --send=no \
        -- balance --id "$who" 2>/dev/null | tr -d '"'
}

#   sac_transfer <signer-alias> <sac-id> <from-addr> <to-addr> <amount> <label>
sac_transfer() {
    local signer="$1" sac="$2" from="$3" to="$4" amount="$5" label="$6"
    inv "$label" "$signer" "$sac" -- transfer --from "$from" --to "$to" --amount "$amount" >/dev/null
}

# Swaps XLM → <to-sac> through the aggregator for a wallet.
#   swap_xlm_to <wallet-alias> <wallet-addr> <to-sac> <xlm-stroops> <label>
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
