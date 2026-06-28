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

# True if the SAC answers a cheap read — i.e. it is live on-ledger and a mint
# against it won't bounce with "Contract not found".
#   sac_live <sac-id>
sac_live() {
    stellar contract invoke --id "$1" --source "$ADMIN" --network "$NETWORK" --send=no \
        -- decimals >/dev/null 2>&1
}

# Polls until the SAC is live, bounded. A freshly-deployed SAC can lag the RPC
# read replica the next mint simulates against; wait it out before use.
#   sac_wait_live <sac-id>
sac_wait_live() {
    local probe
    for probe in $(seq 1 10); do
        sac_live "$1" && return 0
        sleep 2
    done
    return 1
}

# Deploys a fresh SAC for CODE issued by the admin wallet, confirming it is
# live before returning. Sets <VAR>=<sac-contract-id>; persists into state.env.
#   issue_sac <VAR> <CODE>
issue_sac() {
    local var="$1" code="$2"
    if [ -n "${!var:-}" ]; then return 0; fi
    local asset="$code:$ADMIN_ADDR"
    local out_f="$LOG_DIR/sac_$code.out" err_f="$LOG_DIR/sac_$code.err"
    local sac hash attempt
    sac=$(stellar contract id asset --asset "$asset" --network "$NETWORK")
    if sac_live "$sac"; then
        # Already on-ledger (resume): the deterministic id is enough.
        record "issue_sac_$code" ok "asset_id" "" "" "" "" "" "$sac (pre-existing)"
    else
        # Deploy, retrying transient submission failures (TxBadSeq, RPC 5xx).
        # A failed deploy is NOT a resume — deriving the id and pressing on
        # would leave every later mint hitting "Contract not found".
        for attempt in 1 2 3 4 5; do
            [ "$attempt" -gt 1 ] && sleep $(( (attempt - 1) * 3 ))
            if stellar contract asset deploy --asset "$asset" --source "$ADMIN" \
                --network "$NETWORK" >"$out_f" 2>"$err_f"; then
                hash=$(grep -oE 'Signing transaction: [0-9a-f]{64}' "$err_f" | tail -1 | awk '{print $3}')
                break
            fi
            # A racing duplicate deploy reverts but still lands the SAC.
            sac_live "$sac" && break
        done
        if ! sac_wait_live "$sac"; then
            record "issue_sac_$code" FAIL "asset_deploy" "" "" "" "" "" \
                "SAC not live after deploy: $(tail -c 200 "$err_f" | tr '\n\t' '  ')"
            return 1
        fi
        record "issue_sac_$code" ok "asset_deploy" "${hash:-}" "" "" "" "" "$sac"
    fi
    save_state "$var" "$sac"
    log "SAC $code = $sac"
}

# Adds a classic trustline for CODE:ISSUER to a wallet.
#   trustline <wallet-alias> <CODE> <issuer-G-address>
trustline() {
    local wallet="$1" code="$2" issuer="$3"
    local label="trust_${code}_${wallet%%_e2e*}"
    local err_f="$LOG_DIR/$label.err"
    local attempt
    for attempt in 1 2 3; do
        [ "$attempt" -gt 1 ] && sleep $(( (attempt - 1) * 5 ))
        if stellar tx new change-trust --source-account "$wallet" --line "$code:$issuer" \
            --network "$NETWORK" >"$LOG_DIR/$label.out" 2>"$err_f"; then
            local hash
            hash=$(grep -oE '[0-9a-f]{64}' "$err_f" | tail -1)
            record "$label" ok "change_trust" "$hash" "" "" "" "" "$code"
            return 0
        fi
        # A transient gateway/timeout failure leaves no on-ledger effect, and a
        # change-trust to an already-established line re-submits harmlessly, so
        # resubmitting is always safe. A deterministic failure recurs and falls
        # through to FAIL on the final attempt.
        if [ "$attempt" -lt 3 ] && grep -qE "$RPC_TRANSIENT_RE" "$err_f"; then
            record "$label" retry "change_trust" "" "" "" "" "" "transient rpc failure; retrying"
            continue
        fi
        break
    done
    record "$label" FAIL "change_trust" "" "" "" "" "" "$(tail -c 200 "$err_f" | tr '\n\t' '  ')"
    return 1
}

# Mints self-issued SAC units to a trustline holder (NOT the issuer). The
# recipient's classic trustline is established just before this mint; under RPC
# read-after-write lag the mint's simulate can still see no trustline (SAC #13).
# That is a propagation transient here (the change-trust already committed), so
# let inv re-simulate it with backoff instead of failing the suite.
#   mint_to <sac-id> <CODE> <to-G-address> <amount>
mint_to() {
    local sac="$1" code="$2" to="$3" amount="$4"
    INV_TRANSIENT_CONTRACT_RE='trustline entry is missing' \
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
