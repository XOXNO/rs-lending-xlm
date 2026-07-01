# Core lending lifecycle on real-oracle markets (XLM / USDC / EURC):
# market bring-up, wallet funding via the aggregator, supply / borrow /
# repay / withdraw in single and bulk variants, views, and guard reverts.

# Creates the three real-feed markets (idempotent across resumes).
flow_real_markets() {
    phase real_markets
    create_market XLM "$PRIMARY_HUB_ID" "$XLM_SAC" 7 \
        "$(oracle_cfg_reflector XLM 1000000000000000 100000000000000000000)" \
        "$(asset_config_json 7000 7500 1000)"
    create_market USDC "$PRIMARY_HUB_ID" "$USDC_SAC" 7 \
        "$(oracle_cfg_reflector USDC 900000000000000000 1100000000000000000)" \
        "$(asset_config_json 7500 8000 500)"
    create_market EURC "$PRIMARY_HUB_ID" "$EURC_SAC" 7 \
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
        --caller "$ADMIN_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 20000000000 "$USDC_SAC" "$usdc_left")" | tr -d '"') || return 1
    save_state ADMIN_ACCT "$acct"
    save_state SEEDED 1
}

flow_lifecycle() {
    phase lifecycle
    # Create account: single-asset supply (XLM).
    local acct
    acct=$(inv supply_create "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 10000000000)" | tr -d '"')
    save_state ALICE_ACCT "$acct"
    log "alice account = $acct"

    # Bulk supply: two assets in one tx.
    local usdc_half=$(( $(balance "$USDC_SAC" "$ALICE_ADDR") / 2 ))
    inv supply_bulk "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id "$acct" --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 5000000000 "$USDC_SAC" "$usdc_half")" >/dev/null

    # Views.
    view hf_alice "$CONTROLLER" -- get_health_factor --account_id "$acct" >/dev/null
    view coll_usd_alice "$CONTROLLER" -- get_total_collateral_usd --account_id "$acct" >/dev/null
    view ltv_usd_alice "$CONTROLLER" -- get_ltv_collateral_usd --account_id "$acct" >/dev/null
    view attrs_alice "$CONTROLLER" -- get_account_attributes --account_id "$acct" >/dev/null
view positions_alice "$CONTROLLER" -- get_account_positions --account_id "$acct" >/dev/null
view markets_view "$CONTROLLER" -- get_markets_detailed \
--hub_assets "$(hub_vec "$PRIMARY_HUB_ID" "$XLM_SAC" "$USDC_SAC" "$EURC_SAC")" >/dev/null
view indexes_view "$CONTROLLER" -- get_market_indexes_detailed \
--hub_assets "$(hub_vec "$PRIMARY_HUB_ID" "$XLM_SAC" "$USDC_SAC")" >/dev/null

    # Borrow: single, then bulk (two assets in one tx).
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

    # Vault-integrator surfaces (read-only): account existence, pool address, and
    # the max_* sizing views integrators depend on. ALICE holds XLM+USDC
    # collateral with USDC+XLM debt here, so each is exercised with live state.
    assert_bool_view account_exists_alice true account_exists --account_id "$acct"
    assert_int_view_eq pool_addr_view "$POOL" get_pool_address
    assert_int_view_positive max_supply_usdc max_supply --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")"
    assert_int_view_positive max_withdraw_xlm max_withdraw --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")"
    assert_int_view_nonneg max_borrow_usdc max_borrow --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")"

    # Guard reverts: zero amount, over-LTV borrow, paused-state behavior is in admin flow.
    xfail supply_zero 'Error\(Contract, #14\)' "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id "$acct" --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 0)"
    # Over-LTV but under pool liquidity — a larger ask hits the pool's
    # InsufficientLiquidity (#112) before the controller's LTV check (#100),
    # so borrow XLM (deep seeded liquidity) just above the LTV headroom.
    xfail borrow_over_ltv 'Error\(Contract, #100\)' "$ALICE" "$CONTROLLER" -- borrow \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 25000000000)" --to null
    # Over-LTV withdraw guard (#100). Simulate-only (xfail_sim): a single XLM-only
    # withdrawal is not over-LTV when ALICE also holds USDC collateral, and live
    # oracle drift makes any fixed size unreliable — a real send that unexpectedly
    # lands would strip collateral and break the withdraw flow below. Withdrawing
    # ALL collateral (XLM+USDC, amount-0 sentinel) while debt is still open zeroes
    # collateral against open debt, so the post-withdraw health gate
    # (validation.rs InsufficientCollateral) reverts deterministically at any price.
    xfail_sim withdraw_locked 'Error\(Contract, #100\)' "$ALICE" "$CONTROLLER" -- withdraw \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --withdrawals "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 0 "$USDC_SAC" 0)" --to null

    # Repay: partial single, then full bulk (overpay refunds; XLM debt small).
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

    # Cross-account repay: bob repays on alice's account (ownership not required).
    # Both legs retry: live Reflector round rotation between sim and apply
    # invalidates the footprint (storage exceeded_limit trap) at 5-minute
    # boundaries, and the repay depends on the borrow having landed.
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

    # Withdraw: partial, then full close (removes the account).
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
    # Full close via the amount-0 sentinel: retried because live oracle round
    # rotation can trap between sim and apply.
    leg_withdraw_full_bulk() {
        inv withdraw_full_bulk "$ALICE" "$CONTROLLER" -- withdraw \
            --caller "$ALICE_ADDR" --account_id "$acct" \
            --withdrawals "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 0 "$USDC_SAC" 0)" --to null >/dev/null
    }
    retry_leg leg_withdraw_full_bulk
}
