# Zero-state teardown. Runs last in a lane: repays every live account's
# debts (owner wallet, minted on harness-issued mock markets when short),
# withdraws every position, drains the DeFindex strategy when one exists,
# claims all pool revenue, then proves the world is empty on chain:
#
#   - position NFT total_supply == 0 (every emptied account was burned)
#   - per market: pool borrowed == 0 and revenue == 0, supplied == 0
#   - pool + controller token balances down to rounding dust
#
# What survives those asserts is recorded per market as the storage-residue
# report — the exact value the protocol retains after everyone leaves.

TEARDOWN_POOL_DUST="${TEARDOWN_POOL_DUST:-10000}"
TEARDOWN_CTRL_DUST="${TEARDOWN_CTRL_DUST:-1000}"

# Maps an NFT owner address to the run wallet alias that signs for it.
_td_wallet_alias() {
    local addr="$1" role av
    for role in ADMIN ALICE BOB CAROL DAVE EVE FRANK; do
        av="${role}_ADDR"
        if [ "${!av:-}" = "$addr" ]; then printf '%s' "${!role}"; return 0; fi
    done
    return 1
}

# Prints the asset code when `sac` is a harness-issued mock (state SAC_<code>),
# which makes it mintable by ADMIN for repay shortfalls.
_td_mock_code_for() {
    local sac="$1" v
    for v in $(compgen -A variable | grep '^SAC_'); do
        if [ "${!v:-}" = "$sac" ]; then printf '%s' "${v#SAC_}"; return 0; fi
    done
    return 1
}

# Makes sure `owner` can pay `pay` of `sac`: mock assets are minted by ADMIN,
# real classic assets are topped up from ADMIN's balance (trustline first) —
# borrowers whose proceeds went to a receiver or a swap never held the debt
# asset, so the SAC would otherwise reject the repay's transfer.
_td_ensure_funds() {
    local id="$1" alias="$2" owner="$3" sac="$4" pay="$5"
    local bal code line
    bal=$(balance "$sac" "$owner"); [[ "$bal" =~ ^[0-9]+$ ]] || bal=0
    _uint_ge "$bal" "$pay" && return 0
    if code=$(_td_mock_code_for "$sac"); then
        mint_to "$sac" "$code" "$owner" "$pay"
        return 0
    fi
    [ "$sac" = "$XLM_SAC" ] && return 0
    local need=$((pay - bal + 1000))
    local line
    line=$(classic_line "$sac")
    # ADMIN may itself be dry (receiver funding drains it in the flash lane):
    # buy the asset with XLM through the aggregator before handing it on.
    local admin_bal
    admin_bal=$(balance "$sac" "$ADMIN_ADDR"); [[ "$admin_bal" =~ ^[0-9]+$ ]] || admin_bal=0
    if ! _uint_ge "$admin_bal" "$need"; then
        trustline "$ADMIN" "${line%%:*}" "${line##*:}"
        swap_xlm_to "$ADMIN" "$ADMIN_ADDR" "$sac" "${TEARDOWN_SWAP_XLM:-20000000000}" \
            "td_swap_${id}_${sac:0:6}" || true
    fi
    if [ "$owner" != "$ADMIN_ADDR" ]; then
        trustline "$alias" "${line%%:*}" "${line##*:}"
        sac_transfer "$ADMIN" "$sac" "$ADMIN_ADDR" "$owner" "$need" \
            "td_topup_${id}_${sac:0:6}"
    fi
}

# Pass 1: repay every debt of `id`. Debts must clear before any withdrawals so
# pool cash is whole and no lender's exit gets blocked by open utilization.
_td_repay_account() {
    local id="$1"
    local exists owner alias pos k hub sac debt pay bal code
    exists=$(view "td_exists_$id" "$CONTROLLER" -- account_exists --account_id "$id" | tr -d '"')
    [ "$exists" = "true" ] || return 0
    owner=$(view "td_owner_$id" "$POSITION_NFT" -- owner_of --token_id "$id" | tr -d '"')
    alias=$(_td_wallet_alias "$owner") || {
        record "td_owner_unknown_$id" ok owner_of "" "" "" "" "" \
            "account $id owner $owner is not a run wallet; left as residue"
        return 0
    }
    pos=$(view "td_positions_$id" "$CONTROLLER" -- get_account_positions --account_id "$id")
    local debt_keys=()
    while IFS= read -r k; do [ -n "$k" ] && debt_keys+=("$k"); done \
        < <(jq -r '.[1] | keys[]' <<<"$pos" 2>/dev/null)
    # The map key printed by `keys[]` is itself a JSON object literal.
    for k in ${debt_keys[@]+"${debt_keys[@]}"}; do
        hub=$(jq -r '.hub_id' <<<"$k")
        sac=$(jq -r '.asset' <<<"$k")
        debt=$(_view_int "td_debt_${id}_${sac:0:6}" get_borrow_amount \
            --account_id "$id" --hub_asset "$(hub_key "$hub" "$sac")")
        [[ "$debt" =~ ^[0-9]+$ ]] && [ "$debt" -gt 0 ] || continue
        # Interest accrues between the read and the transaction; overpay is
        # capped at the live debt by the controller, so the buffer is free.
        pay=$((debt + debt / 50 + 100))
        _td_ensure_funds "$id" "$alias" "$owner" "$sac" "$pay"
        inv "td_repay_${id}_${sac:0:6}" "$alias" "$CONTROLLER" -- repay \
            --caller "$owner" --account_id "$id" \
            --payments "$(pay_vec "$hub" "$sac" "$pay")" >/dev/null
        assert_int_view_eq "td_debt_cleared_${id}_${sac:0:6}" 0 get_borrow_amount \
            --account_id "$id" --hub_asset "$(hub_key "$hub" "$sac")"
    done
}

# Pass 2: withdraw every position of `id`; amount 0 means "everything", and the
# controller burns the account once its last position is gone.
_td_withdraw_account() {
    local id="$1"
    local exists owner alias pos k hub sac
    exists=$(view "td_exists2_$id" "$CONTROLLER" -- account_exists --account_id "$id" | tr -d '"')
    [ "$exists" = "true" ] || return 0
    owner=$(view "td_owner2_$id" "$POSITION_NFT" -- owner_of --token_id "$id" | tr -d '"')
    alias=$(_td_wallet_alias "$owner") || return 0
    pos=$(view "td_positions2_$id" "$CONTROLLER" -- get_account_positions --account_id "$id")
    local supply_keys=()
    while IFS= read -r k; do [ -n "$k" ] && supply_keys+=("$k"); done \
        < <(jq -r '.[0] | keys[]' <<<"$pos" 2>/dev/null)
    for k in ${supply_keys[@]+"${supply_keys[@]}"}; do
        hub=$(jq -r '.hub_id' <<<"$k")
        sac=$(jq -r '.asset' <<<"$k")
        inv "td_withdraw_${id}_${sac:0:6}" "$alias" "$CONTROLLER" -- withdraw \
            --caller "$owner" --account_id "$id" \
            --withdrawals "$(pay_vec "$hub" "$sac" 0)" --to null >/dev/null
    done
    assert_bool_view "td_burned_$id" false account_exists --account_id "$id"
}

flow_teardown() {
    phase teardown
    require_var MARKETS
    require_var POSITION_NFT

    # The DeFindex strategy owns its controller account, so it can only be
    # emptied through its own withdraw. Drain it first; its account burns with
    # everyone else's below.
    if [ -n "${STRATEGY:-}" ]; then
        local sbal
        sbal=$(view td_dfx_balance "$STRATEGY" -- balance --from "$DAVE_ADDR" | tr -d '"')
        if [[ "$sbal" =~ ^[1-9][0-9]*$ ]]; then
            inv td_dfx_withdraw_all "$DAVE" "$STRATEGY" -- withdraw \
                --amount "$sbal" --from "$DAVE_ADDR" --to "$DAVE_ADDR" >/dev/null
        fi
    fi

    # Snapshot live token ids before any burn reshuffles the enumeration.
    local total ids=() i id
    total=$(view td_nft_total "$POSITION_NFT" -- total_supply | tr -d '"')
    [[ "$total" =~ ^[0-9]+$ ]] || die td_nft_total "total_supply unreadable: '$total'"
    log "teardown: $total live account(s)"
    i=0
    while [ "$i" -lt "$total" ]; do
        id=$(view "td_token_$i" "$POSITION_NFT" -- get_token_id --index "$i" | tr -d '"')
        [[ "$id" =~ ^[0-9]+$ ]] && ids+=("$id")
        i=$((i + 1))
    done

    for id in ${ids[@]+"${ids[@]}"}; do _td_repay_account "$id"; done
    for id in ${ids[@]+"${ids[@]}"}; do _td_withdraw_account "$id"; done

    # Revenue last: with borrowed at zero nothing accrues after the claim, so
    # the post-claim zero read is stable.
    local m hub sac rev
    for m in $MARKETS; do
        hub="${m%%:*}"; sac="${m##*:}"
        rev=$(_view_pool_int "td_rev_${hub}_${sac:0:6}" get_revenue \
            --hub_asset "$(hub_key "$hub" "$sac")")
        if [[ "$rev" =~ ^[0-9]+$ ]] && [ "$rev" -gt 0 ]; then
            inv "td_claim_${hub}_${sac:0:6}" "$ADMIN" "$CONTROLLER" -- claim_revenue \
                --caller "$ADMIN_ADDR" --assets "$(hub_vec "$hub" "$sac")" >/dev/null
        fi
    done

    # The zero-state proof, then the residue report: what the protocol still
    # holds per market after every participant has left.
    assert_view_eq_at "$POSITION_NFT" td_nft_zero 0 total_supply
    local borrowed revenue supplied reserves pool_bal ctrl_bal
    for m in $MARKETS; do
        hub="${m%%:*}"; sac="${m##*:}"
        borrowed=$(_view_pool_int "td_borrowed_${hub}_${sac:0:6}" get_borrowed_amount \
            --hub_asset "$(hub_key "$hub" "$sac")")
        supplied=$(_view_pool_int "td_supplied_${hub}_${sac:0:6}" get_supplied_amount \
            --hub_asset "$(hub_key "$hub" "$sac")")
        revenue=$(_view_pool_int "td_revenue_${hub}_${sac:0:6}" get_revenue \
            --hub_asset "$(hub_key "$hub" "$sac")")
        reserves=$(_view_pool_int "td_reserves_${hub}_${sac:0:6}" get_reserves \
            --hub_asset "$(hub_key "$hub" "$sac")")
        pool_bal=$(balance "$sac" "$POOL"); [[ "$pool_bal" =~ ^[0-9]+$ ]] || pool_bal=0
        ctrl_bal=$(balance "$sac" "$CONTROLLER"); [[ "$ctrl_bal" =~ ^[0-9]+$ ]] || ctrl_bal=0

        [ "$borrowed" = "0" ] \
            || _assert_fail "td_zero_borrowed_${hub}_${sac:0:6}" "borrowed=$borrowed want 0"
        [ "$supplied" = "0" ] \
            || _assert_fail "td_zero_supplied_${hub}_${sac:0:6}" "supplied=$supplied want 0"
        [ "$revenue" = "0" ] \
            || _assert_fail "td_zero_revenue_${hub}_${sac:0:6}" "revenue=$revenue want 0 after claim"
        _uint_le "$pool_bal" "$TEARDOWN_POOL_DUST" \
            || _assert_fail "td_pool_dust_${hub}_${sac:0:6}" "pool balance $pool_bal > dust cap $TEARDOWN_POOL_DUST"
        _uint_le "$ctrl_bal" "$TEARDOWN_CTRL_DUST" \
            || _assert_fail "td_ctrl_dust_${hub}_${sac:0:6}" "controller balance $ctrl_bal > dust cap $TEARDOWN_CTRL_DUST"
        record "td_residue_${hub}_${sac:0:6}" ok residue "" "" "" "" "" \
            "supplied=$supplied borrowed=$borrowed revenue=$revenue reserves=$reserves pool_bal=$pool_bal ctrl_bal=$ctrl_bal"
    done
    log "teardown: zero state proven for markets: $MARKETS"
}
