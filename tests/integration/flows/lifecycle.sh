# Core lending lifecycle on real-oracle markets (XLM / USDC / EURC):
# market bring-up, wallet funding via the aggregator, supply / borrow /
# repay / withdraw in single and bulk variants, views, and guard reverts.

# Creates the three real-feed markets (idempotent across resumes).
flow_real_markets() {
    phase real_markets
    create_market XLM "$XLM_SAC" 7 \
        "$(oracle_cfg_reflector XLM 1000000000000000 100000000000000000000)" \
        "$(asset_config_json 7000 7500 1000)"
    create_market USDC "$USDC_SAC" 7 \
        "$(oracle_cfg_reflector USDC 900000000000000000 1100000000000000000)" \
        "$(asset_config_json 7500 8000 500)"
    create_market EURC "$EURC_SAC" 7 \
        "$(oracle_cfg_reflector EURC 800000000000000000 1500000000000000000)" \
        "$(asset_config_json 7500 8000 500)"
}

# Derives CODE:ISSUER for a classic-asset SAC from its name().
classic_line() {
    local sac="$1"
    stellar contract invoke --id "$sac" --source "$ADMIN" --network "$NETWORK" --send=no \
        -- name 2>/dev/null | tr -d '"'
}

# Funds ALICE/BOB/CAROL with USDC: one admin swap, then SAC transfers
# (multiple rapid swaps trip the aggregator's stale min-out check).
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
    # 5,000 XLM → USDC in one swap, then split three ways.
    swap_xlm_to "$ADMIN" "$ADMIN_ADDR" "$USDC_SAC" 50000000000 fund_swap_usdc || return 1
    local got
    got=$(balance "$USDC_SAC" "$ADMIN_ADDR")
    [ -z "$got" ] || [ "$got" -le 0 ] && { log "funding swap produced no USDC"; return 1; }
    log "admin USDC balance: $got"
    local share=$((got / 4))
    sac_transfer "$ADMIN" "$USDC_SAC" "$ADMIN_ADDR" "$ALICE_ADDR" "$share" fund_alice_usdc
    sac_transfer "$ADMIN" "$USDC_SAC" "$ADMIN_ADDR" "$BOB_ADDR" "$share" fund_bob_usdc
    sac_transfer "$ADMIN" "$USDC_SAC" "$ADMIN_ADDR" "$CAROL_ADDR" "$share" fund_carol_usdc
    # EURC for repay-variety: alice swaps a small XLM amount herself.
    line=$(classic_line "$EURC_SAC")
    trustline "$ALICE" "${line%%:*}" "${line##*:}"
    swap_xlm_to "$ALICE" "$ALICE_ADDR" "$EURC_SAC" 5000000000 fund_alice_eurc
    save_state FUNDED_USDC 1
}

# Admin seeds borrow-side liquidity so user borrows do not hit utilization caps.
flow_seed_liquidity() {
    phase seed_liquidity
    [ -n "${SEEDED:-}" ] && return 0
    local usdc_left acct
    usdc_left=$(balance "$USDC_SAC" "$ADMIN_ADDR")
    [ -z "$usdc_left" ] || [ "$usdc_left" -le 0 ] && { log "no USDC to seed"; return 1; }
    acct=$(inv seed_supply "$ADMIN" "$CONTROLLER" -- supply \
        --caller "$ADMIN_ADDR" --account_id 0 --e_mode_category 0 \
        --assets "$(pay_vec "$XLM_SAC" 20000000000 "$USDC_SAC" "$usdc_left")" | tr -d '"') || return 1
    save_state ADMIN_ACCT "$acct"
    save_state SEEDED 1
}

flow_lifecycle() {
    phase lifecycle
    # Create account: single-asset supply (XLM).
    local acct
    acct=$(inv supply_create "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id 0 --e_mode_category 0 \
        --assets "$(pay_vec "$XLM_SAC" 10000000000)" | tr -d '"')
    save_state ALICE_ACCT "$acct"
    log "alice account = $acct"

    # Bulk supply: two assets in one tx.
    local usdc_half=$(( $(balance "$USDC_SAC" "$ALICE_ADDR") / 2 ))
    inv supply_bulk "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id "$acct" --e_mode_category 0 \
        --assets "$(pay_vec "$XLM_SAC" 5000000000 "$USDC_SAC" "$usdc_half")" >/dev/null

    # Views.
    view hf_alice "$CONTROLLER" -- health_factor --account_id "$acct" >/dev/null
    view coll_usd_alice "$CONTROLLER" -- total_collateral_in_usd --account_id "$acct" >/dev/null
    view ltv_usd_alice "$CONTROLLER" -- ltv_collateral_in_usd --account_id "$acct" >/dev/null
    view attrs_alice "$CONTROLLER" -- get_account_attributes --account_id "$acct" >/dev/null
    view positions_alice "$CONTROLLER" -- get_account_positions --account_id "$acct" >/dev/null
    view markets_view "$CONTROLLER" -- get_all_markets_detailed \
        --assets "[\"$XLM_SAC\",\"$USDC_SAC\",\"$EURC_SAC\"]" >/dev/null
    view indexes_view "$CONTROLLER" -- get_all_market_indexes_detailed \
        --assets "[\"$XLM_SAC\",\"$USDC_SAC\"]" >/dev/null

    # Borrow: single, then bulk (two assets in one tx).
    inv borrow_single "$ALICE" "$CONTROLLER" -- borrow \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$USDC_SAC" 200000000)" >/dev/null
    inv borrow_bulk "$ALICE" "$CONTROLLER" -- borrow \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$USDC_SAC" 150000000 "$XLM_SAC" 1000000000)" >/dev/null
    view borrow_usd_alice "$CONTROLLER" -- total_borrow_in_usd --account_id "$acct" >/dev/null

    # Guard reverts: zero amount, over-LTV borrow, paused-state behavior is in admin flow.
    xfail supply_zero 'Error\(Contract, #14\)' "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id "$acct" --e_mode_category 0 \
        --assets "$(pay_vec "$XLM_SAC" 0)"
    # Over-LTV but under pool liquidity — a larger ask hits the pool's
    # InsufficientLiquidity (#112) before the controller's LTV check (#100),
    # so borrow XLM (deep seeded liquidity) just above the LTV headroom.
    xfail borrow_over_ltv 'Error\(Contract, #100\)' "$ALICE" "$CONTROLLER" -- borrow \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$XLM_SAC" 25000000000)"
    xfail withdraw_locked 'Error\(Contract, #1[0-9][0-9]\)' "$ALICE" "$CONTROLLER" -- withdraw \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --withdrawals "$(pay_vec "$XLM_SAC" 15000000000)"

    # Repay: partial single, then full bulk (overpay refunds; XLM debt small).
    inv repay_partial "$ALICE" "$CONTROLLER" -- repay \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --payments "$(pay_vec "$USDC_SAC" 100000000)" >/dev/null
    local usdc_debt xlm_debt
    usdc_debt=$(view debt_usdc_alice "$CONTROLLER" -- borrow_amount_for_token \
        --account_id "$acct" --asset "$USDC_SAC" | tr -d '"')
    xlm_debt=$(view debt_xlm_alice "$CONTROLLER" -- borrow_amount_for_token \
        --account_id "$acct" --asset "$XLM_SAC" | tr -d '"')
    inv repay_full_bulk "$ALICE" "$CONTROLLER" -- repay \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --payments "$(pay_vec "$USDC_SAC" $((usdc_debt + 10000000)) "$XLM_SAC" $((xlm_debt + 10000000)))" >/dev/null

    # Cross-account repay: bob repays on alice's account (ownership not required).
    inv borrow_again "$ALICE" "$CONTROLLER" -- borrow \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$USDC_SAC" 120000000)" >/dev/null
    inv repay_cross_account "$BOB" "$CONTROLLER" -- repay \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --payments "$(pay_vec "$USDC_SAC" 130000000)" >/dev/null

    # Withdraw: partial, then full close (removes the account).
    inv withdraw_partial "$ALICE" "$CONTROLLER" -- withdraw \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --withdrawals "$(pay_vec "$XLM_SAC" 5000000000)" >/dev/null
    inv renew_account "$ALICE" "$CONTROLLER" -- renew_account \
        --caller "$ALICE_ADDR" --account_id "$acct" >/dev/null
    local xlm_coll usdc_coll
    xlm_coll=$(view coll_xlm_alice "$CONTROLLER" -- collateral_amount_for_token \
        --account_id "$acct" --asset "$XLM_SAC" | tr -d '"')
    usdc_coll=$(view coll_usdc_alice "$CONTROLLER" -- collateral_amount_for_token \
        --account_id "$acct" --asset "$USDC_SAC" | tr -d '"')
    inv withdraw_full_bulk "$ALICE" "$CONTROLLER" -- withdraw \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --withdrawals "$(pay_vec "$XLM_SAC" "$xlm_coll" "$USDC_SAC" "$usdc_coll")" >/dev/null
}
