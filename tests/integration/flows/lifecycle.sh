flow_real_markets() {
    phase real_markets
    local xlm_band
    xlm_band=$(reflector_band XLM) || { log "XLM live price unavailable; cannot calibrate sanity band"; return 1; }
    create_market XLM "$PRIMARY_HUB_ID" "$XLM_SAC" 7 \
        "$(oracle_cfg_reflector XLM $xlm_band)" \
        "$(asset_config_json 7000 7500 1000)"
    create_market USDC "$PRIMARY_HUB_ID" "$USDC_SAC" 7 \
        "$(oracle_cfg_reflector USDC 900000000000000000 1100000000000000000)" \
        "$(asset_config_json 7500 8000 500)"
    create_market EURC "$PRIMARY_HUB_ID" "$EURC_SAC" 7 \
        "$(oracle_cfg_reflector EURC 980000000000000000 1180000000000000000)" \
        "$(asset_config_json 7500 8000 500)"
}

classic_line() {
    local sac="$1"
    stellar contract invoke --id "$sac" --source "$ADMIN" "${NET_ARGS[@]}" --send=no \
        -- name 2>/dev/null | tr -d '"'
}

flow_fund_usdc() {
    phase funding
    [ -n "${FUNDED_USDC:-}" ] && return 0
    local line code issuer
    line=$(classic_line "$USDC_SAC")
    code="${line%%:*}"; issuer="${line##*:}"
    trustline "$ADMIN" "$code" "$issuer"
    trustline "$ALICE" "$code" "$issuer"
    trustline "$BOB" "$code" "$issuer"
    trustline "$CAROL" "$code" "$issuer"

    swap_xlm_to "$ADMIN" "$ADMIN_ADDR" "$USDC_SAC" 50000000000 fund_swap_usdc || return 1
    local got
    got=$(balance "$USDC_SAC" "$ADMIN_ADDR")
    [ -z "$got" ] || [ "$got" -le 0 ] && { log "funding swap produced no USDC"; return 1; }
    log "admin USDC balance: $got"
    local share=$((got / 4))
    sac_transfer "$ADMIN" "$USDC_SAC" "$ADMIN_ADDR" "$ALICE_ADDR" "$share" fund_alice_usdc
    sac_transfer "$ADMIN" "$USDC_SAC" "$ADMIN_ADDR" "$BOB_ADDR" "$share" fund_bob_usdc
    sac_transfer "$ADMIN" "$USDC_SAC" "$ADMIN_ADDR" "$CAROL_ADDR" "$share" fund_carol_usdc

    line=$(classic_line "$EURC_SAC")
    trustline "$ALICE" "${line%%:*}" "${line##*:}"
    swap_xlm_to "$ALICE" "$ALICE_ADDR" "$EURC_SAC" 5000000000 fund_alice_eurc
    save_state FUNDED_USDC 1
}

flow_seed_liquidity() {
    phase seed_liquidity
    [ -n "${SEEDED:-}" ] && return 0
    local usdc_left acct
    usdc_left=$(balance "$USDC_SAC" "$ADMIN_ADDR")
    [ -z "$usdc_left" ] || [ "$usdc_left" -le 0 ] && { log "no USDC to seed"; return 1; }
    acct=$(inv_create seed_supply "$ADMIN" "$CONTROLLER" -- supply \
        --caller "$ADMIN_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 20000000000 "$USDC_SAC" "$usdc_left")" | tr -d '"') || return 1
    save_state ADMIN_ACCT "$acct"
    save_state SEEDED 1
}

flow_lifecycle() {
    phase lifecycle
    local acct
    acct=$(inv_create supply_create "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 10000000000)" | tr -d '"')
    save_state ALICE_ACCT "$acct"
    log "alice account = $acct"

    # balance() sends stderr to /dev/null, so a failed read is an empty string.
    # Expanding that directly inside $(( )) aborts the whole run with a bash
    # syntax error, before any report is written.
    local usdc_bal usdc_half
    usdc_bal=$(balance "$USDC_SAC" "$ALICE_ADDR")
    if [[ "$usdc_bal" =~ ^[0-9]+$ ]] && [ "$usdc_bal" -gt 0 ]; then
        usdc_half=$(( usdc_bal / 2 ))
    else
        usdc_half=0
        record supply_bulk FAIL supply "" "" "" "" "" \
            "alice USDC balance unreadable or zero: '${usdc_bal}'"
    fi
    if [ "$usdc_half" -gt 0 ]; then
        inv supply_bulk "$ALICE" "$CONTROLLER" -- supply \
            --caller "$ALICE_ADDR" --account_id "$acct" --spoke_id "$PRIMARY_SPOKE_ID" \
            --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 5000000000 "$USDC_SAC" "$usdc_half")" >/dev/null
    fi

    view hf_alice "$CONTROLLER" -- get_health_factor --account_id "$acct" >/dev/null
    view coll_usd_alice "$CONTROLLER" -- get_total_collateral_usd --account_id "$acct" >/dev/null
    view ltv_usd_alice "$CONTROLLER" -- get_ltv_collateral_usd --account_id "$acct" >/dev/null
    view attrs_alice "$CONTROLLER" -- get_account_attributes --account_id "$acct" >/dev/null
view positions_alice "$CONTROLLER" -- get_account_positions --account_id "$acct" >/dev/null
view indexes_view "$CONTROLLER" -- get_market_indexes_detailed \
--hub_assets "$(hub_vec "$PRIMARY_HUB_ID" "$XLM_SAC" "$USDC_SAC" "$EURC_SAC")" >/dev/null

    inv borrow_single "$ALICE" "$CONTROLLER" -- borrow \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 200000000)" --to null >/dev/null
    inv borrow_bulk "$ALICE" "$CONTROLLER" -- borrow \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 150000000 "$XLM_SAC" 1000000000)" --to null >/dev/null
    local borrow_usd
    borrow_usd=$(_view_int borrow_usd_alice get_total_borrow_usd --account_id "$acct")
    _uint_ge "$borrow_usd" 1 || _assert_fail borrow_usd_alice "total_borrow_usd=$borrow_usd want > 0"
    assert_hf_at_least hf_alice_post_borrow "$acct" "$WAD"
    assert_borrow_at_least debt_usdc_post_borrow "$acct" "$USDC_SAC" 200000000

    assert_bool_view account_exists_alice true account_exists --account_id "$acct"
    assert_int_view_eq pool_addr_view "$POOL" get_pool_address

    xfail supply_zero 'Error\(Contract, #14\)' "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id "$acct" --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 0)"

    # Both guards below have to bind on collateral (#100), and neither does at
    # rest: Alice's collateral (1k XLM plus the USDC the funding swap hands out)
    # dwarfs her debt, so a fixed over-borrow sits well inside her limit and any
    # withdrawal big enough to breach LTV exhausts pool liquidity first (#112).
    # Both amounts are therefore derived from her actual borrowing power, and we
    # first spend most of that power so the limit is the binding constraint. The
    # repays below read the debt back at runtime, so they clear this too.
    local ltv_wad debt_wad headroom_usdc over_usdc
    ltv_wad=$(_view_int ltv_usd_pre_edge get_ltv_collateral_usd --account_id "$acct")
    debt_wad=$(_view_int borrow_usd_pre_edge get_total_borrow_usd --account_id "$acct")
    # USD is WAD-scaled (1e18) and USDC has 7 decimals, so 1 USDC unit == 1e11.
    # Borrow 90% of the headroom: enough to leave the limit within reach, with
    # room for a price tick between this read and the transaction.
    headroom_usdc=$(awk -v l="$ltv_wad" -v d="$debt_wad" 'BEGIN{printf "%d", (l-d)/1e11*0.9}')
    if [ -z "$headroom_usdc" ] || [ "$headroom_usdc" -lt 1000000 ]; then
        _assert_fail borrow_to_ltv_edge "no borrowing headroom to set up the LTV guards (got ${headroom_usdc:-<none>})"
    else
        inv borrow_to_ltv_edge "$ALICE" "$CONTROLLER" -- borrow \
            --caller "$ALICE_ADDR" --account_id "$acct" \
            --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" "$headroom_usdc")" --to null >/dev/null
    fi

    # Half the original headroom against the ~10% that is left is unambiguously
    # over the limit, and in USDC it stays inside what the pool can lend, so the
    # revert is #100 and not #112.
    over_usdc=$(awk -v l="$ltv_wad" -v d="$debt_wad" 'BEGIN{printf "%d", (l-d)/1e11*0.5}')
    xfail borrow_over_ltv 'Error\(Contract, #100\)' "$ALICE" "$CONTROLLER" -- borrow \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" "$over_usdc")" --to null

    # Pulling all the XLM and half the USDC strips far more borrowing power than
    # the headroom left above, while staying well inside what the pool can pay
    # out -- so the withdrawal is refused for being unbacked, not for liquidity.
    local xlm_coll_pre usdc_coll_pre
    xlm_coll_pre=$(_view_int coll_xlm_pre_lock get_collateral_amount \
        --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")")
    usdc_coll_pre=$(_view_int coll_usdc_pre_lock get_collateral_amount \
        --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    xfail_sim withdraw_locked 'Error\(Contract, #100\)' "$ALICE" "$CONTROLLER" -- withdraw \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --withdrawals "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" "$xlm_coll_pre" "$USDC_SAC" $((usdc_coll_pre / 2)))" --to null

    local usdc_debt_pre_partial
usdc_debt_pre_partial=$(_view_int debt_usdc_pre_partial get_borrow_amount \
--account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    inv repay_partial "$ALICE" "$CONTROLLER" -- repay \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --payments "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 100000000)" >/dev/null
    assert_borrow_decreased debt_usdc_post_partial "$acct" "$USDC_SAC" "$usdc_debt_pre_partial"
    local usdc_debt xlm_debt
usdc_debt=$(view debt_usdc_alice "$CONTROLLER" -- get_borrow_amount \
--account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" | tr -d '"')
xlm_debt=$(view debt_xlm_alice "$CONTROLLER" -- get_borrow_amount \
--account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" | tr -d '"')
    inv repay_full_bulk "$ALICE" "$CONTROLLER" -- repay \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --payments "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" $((usdc_debt + 10000000)) "$XLM_SAC" $((xlm_debt + 10000000)))" >/dev/null
    assert_borrow_at_most debt_usdc_cleared "$acct" "$USDC_SAC" 1000000
    assert_borrow_at_most debt_xlm_cleared "$acct" "$XLM_SAC" 1000000

    leg_borrow_again() {
        inv borrow_again "$ALICE" "$CONTROLLER" -- borrow \
            --caller "$ALICE_ADDR" --account_id "$acct" \
            --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 120000000)" --to null >/dev/null
        local debt_after_borrow
debt_after_borrow=$(view debt_usdc_alice "$CONTROLLER" -- get_borrow_amount \
--account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" | tr -d '"')
        if [ -z "$debt_after_borrow" ] || [ "$debt_after_borrow" -lt 120000000 ]; then
            log "borrow_again: USDC debt too low ($debt_after_borrow) for cross-account repay"
            return 1
        fi
    }
    retry_leg leg_borrow_again
    leg_repay_cross_account() {
        inv repay_cross_account "$BOB" "$CONTROLLER" -- repay \
            --caller "$BOB_ADDR" --account_id "$acct" \
            --payments "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 130000000)" >/dev/null
    }
    retry_leg leg_repay_cross_account

    inv withdraw_partial "$ALICE" "$CONTROLLER" -- withdraw \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --withdrawals "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 5000000000)" --to null >/dev/null
    inv renew_account "$ALICE" "$CONTROLLER" -- renew_account \
        --caller "$ALICE_ADDR" --account_id "$acct" >/dev/null
local xlm_coll usdc_coll
xlm_coll=$(view coll_xlm_alice "$CONTROLLER" -- get_collateral_amount \
--account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" | tr -d '"')
usdc_coll=$(view coll_usdc_alice "$CONTROLLER" -- get_collateral_amount \
--account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" | tr -d '"')

    leg_withdraw_full_bulk() {
        inv withdraw_full_bulk "$ALICE" "$CONTROLLER" -- withdraw \
            --caller "$ALICE_ADDR" --account_id "$acct" \
            --withdrawals "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 0 "$USDC_SAC" 0)" --to null >/dev/null
    }
    retry_leg leg_withdraw_full_bulk
}
