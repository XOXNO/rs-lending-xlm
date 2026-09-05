: "${LIQ_UNIT:=10000000}"
: "${LIQ_CODES:=(LIQA LIQB LIQC LIQD LIQE LIQF LIQG)}"

flow_liq_setup() {
    phase liq_setup
    [ -n "${LIQ_SETUP_DONE:-}" ] && return 0
    deploy_mock_reflector
    deploy_mock_redstone
    local code var sac
    for code in "${LIQ_CODES[@]}"; do
        var="SAC_$code"
        issue_sac "$var" "$code"
        sac="${!var}"
        for w in "$ALICE" "$BOB" "$CAROL"; do
            trustline "$w" "$code" "$ADMIN_ADDR"
        done
        mint_to "$sac" "$code" "$BOB_ADDR"   $((100000 * LIQ_UNIT))
        mint_to "$sac" "$code" "$CAROL_ADDR" $((100000 * LIQ_UNIT))

        dual_px "$sac" "$code" "$WAD" "px_init_$code"
    done

    create_market LIQA "$PRIMARY_HUB_ID" "$SAC_LIQA" 7 "$(oracle_cfg_mock_dual "$SAC_LIQA" LIQA)" "$(asset_config_json 7000 7500 800)"
    create_market LIQB "$PRIMARY_HUB_ID" "$SAC_LIQB" 7 "$(oracle_cfg_mock_dual "$SAC_LIQB" LIQB)" "$(asset_config_json 7000 7500 800)"
    create_market LIQC "$PRIMARY_HUB_ID" "$SAC_LIQC" 7 "$(oracle_cfg_mock_dual "$SAC_LIQC" LIQC)" "$(asset_config_json 7000 7500 800)"
    create_market LIQD "$PRIMARY_HUB_ID" "$SAC_LIQD" 7 "$(oracle_cfg_mock_dual "$SAC_LIQD" LIQD)" "$(asset_config_json 7000 7500 800)"
    create_market LIQE "$PRIMARY_HUB_ID" "$SAC_LIQE" 7 "$(oracle_cfg_mock_dual "$SAC_LIQE" LIQE)" "$(asset_config_json 7000 7500 200)"
    create_market LIQF "$PRIMARY_HUB_ID" "$SAC_LIQF" 7 "$(oracle_cfg_mock_dual "$SAC_LIQF" LIQF)" "$(asset_config_json 7000 7500 200)"
    create_market LIQG "$PRIMARY_HUB_ID" "$SAC_LIQG" 7 "$(oracle_cfg_mock_dual "$SAC_LIQG" LIQG)" "$(asset_config_json 7000 7500 800)"

    inv liq_seed_liquidity "$CAROL" "$CONTROLLER" -- supply \
        --caller "$CAROL_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((50000 * LIQ_UNIT)) "$SAC_LIQD" $((50000 * LIQ_UNIT)) "$SAC_LIQF" $((50000 * LIQ_UNIT)))" >/dev/null || return 1
    save_state LIQ_SETUP_DONE 1
}

flow_liq_single() {
    phase liq_single
    local acct
    acct=$(inv_create liq1_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQA" $((1000 * LIQ_UNIT)))" | tr -d '"')
    inv liq1_borrow "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((600 * LIQ_UNIT)))" --to null >/dev/null

    assert_can_liquidated liq1_can_liq_pre "$acct" false
    xfail liq1_liquidate_healthy 'Error\(Contract, #101\)' "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_transfer)" \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((100 * LIQ_UNIT)))"

    dual_px "$SAC_LIQA" LIQA $((WAD / 10 * 7)) liq1_crash
    assert_hf_below_wad liq1_hf "$acct"
    assert_can_liquidated liq1_can_liq "$acct" true
    # Pin the seizure to the estimate and the fee to pool revenue (INV-LIQ-02):
    # on debt-decreased alone, an over-seizing or fee-less liquidation passes.
    local liq1_est_seized liq1_coll_pre liq1_rev_pre
    liq1_est_seized=$(view liq1_estimate "$CONTROLLER" -- get_liquidation_estimate --seize_mode "$(seize_transfer)" \
        --account_id "$acct" --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((100 * LIQ_UNIT)))" \
        | jq -r '[.seized_collaterals[]?.amount | tonumber] | add // 0')
    view liq1_avail "$CONTROLLER" -- get_liquidation_collateral --account_id "$acct" >/dev/null
    liq1_coll_pre=$(_view_int liq1_coll_pre get_collateral_amount \
        --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQA")")
    liq1_rev_pre=$(_view_pool_int liq1_rev_pre get_revenue --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQA")")

    local liq1_debt_pre_partial=$((600 * LIQ_UNIT))
    inv liq1_liquidate_partial "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_transfer)" \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((100 * LIQ_UNIT)))" >/dev/null
    assert_borrow_decreased liq1_debt_post_partial "$acct" "$SAC_LIQB" "$liq1_debt_pre_partial"

    local liq1_coll_post liq1_seized liq1_rev_post
    liq1_coll_post=$(_view_int liq1_coll_post get_collateral_amount \
        --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQA")")
    liq1_seized=$((liq1_coll_pre - liq1_coll_post))
    # Interest accrues between estimate and execution; 0.1% covers that drift
    # while a doubled or zero seizure still fails.
    if _uint_ge "$liq1_seized" $((liq1_est_seized * 999 / 1000)) \
        && _uint_le "$liq1_seized" $((liq1_est_seized * 1001 / 1000)); then
        record liq1_seizure_matches_estimate ok liquidate "" "" "" "" "" "seized=$liq1_seized estimate=$liq1_est_seized"
    else
        _assert_fail liq1_seizure_matches_estimate "seized=$liq1_seized want within 0.1% of estimate $liq1_est_seized"
    fi
    liq1_rev_post=$(_view_pool_int liq1_rev_post get_revenue --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQA")")
    if _uint_lt "${liq1_rev_pre:-0}" "$liq1_rev_post"; then
        record liq1_fee_booked ok liquidate "" "" "" "" "" "$liq1_rev_pre -> $liq1_rev_post"
    else
        _assert_fail liq1_fee_booked "pool revenue $liq1_rev_pre -> $liq1_rev_post; want an increase from the liquidation fee"
    fi

    assert_borrow_at_most liq1_debt_cap_partial "$acct" "$SAC_LIQB" $(( 501 * LIQ_UNIT ))

    local est refund close
    est=$(view liq1_estimate_close "$CONTROLLER" -- get_liquidation_estimate --seize_mode "$(seize_transfer)" \
        --account_id "$acct" --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((600 * LIQ_UNIT)))")
    refund=$(jq -r '[.refunds[]?.amount | tonumber] | add // 0' <<<"$est")
    close=$(( 600 * LIQ_UNIT - refund ))
    leg_liq1_full() {
        inv liq1_liquidate_full "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_transfer)" \
            --liquidator "$CAROL_ADDR" --account_id "$acct" \
            --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $(( close * 998 / 1000 )))" >/dev/null
    }
    retry_leg leg_liq1_full

    assert_borrow_at_most liq1_debt_cleared "$acct" "$SAC_LIQB" $(( 100 * LIQ_UNIT ))
    save_state LIQ1_ACCT "$acct"
}

flow_liq_bulk() {
    phase liq_bulk
    local acct
    acct=$(inv_create liq2_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQC" $((800 * LIQ_UNIT)) "$SAC_LIQA" $((1143 * LIQ_UNIT)))" | tr -d '"')
    inv liq2_borrow_bulk "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((500 * LIQ_UNIT)) "$SAC_LIQD" $((500 * LIQ_UNIT)))" --to null >/dev/null

    dual_px "$SAC_LIQC" LIQC $((WAD / 10 * 7)) liq2_crash_c
    dual_px "$SAC_LIQA" LIQA $((WAD / 100 * 49)) liq2_crash_a
    assert_hf_below_wad liq2_hf "$acct"

    local liq2_debt_b_pre liq2_debt_d_pre
liq2_debt_b_pre=$(_view_int liq2_debt_b_pre get_borrow_amount --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQB")")
liq2_debt_d_pre=$(_view_int liq2_debt_d_pre get_borrow_amount --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQD")")
    inv liq2_liquidate_bulk "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_transfer)" \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((150 * LIQ_UNIT)) "$SAC_LIQD" $((150 * LIQ_UNIT)))" >/dev/null
    assert_borrow_decreased liq2_debt_b_post "$acct" "$SAC_LIQB" "$liq2_debt_b_pre"
    assert_borrow_decreased liq2_debt_d_post "$acct" "$SAC_LIQD" "$liq2_debt_d_pre"
    save_state LIQ2_ACCT "$acct"
}

flow_liq_spoke() {
    phase liq_spoke
    if [ -z "${SPOKE_ID:-}" ]; then
        local spoke_id
        spoke_id=$(inv spoke_add_category "$ADMIN" "$CONTROLLER" -- add_spoke | tr -d '"')
        save_state SPOKE_ID "$spoke_id"
        inv spoke_add_liqe "$ADMIN" "$CONTROLLER" -- add_asset_to_spoke \
            --input "$(spoke_args "$PRIMARY_HUB_ID" "$SAC_LIQE" "$spoke_id" true false 9500 9700 200)" >/dev/null
        inv spoke_add_liqf "$ADMIN" "$CONTROLLER" -- add_asset_to_spoke \
            --input "$(spoke_args "$PRIMARY_HUB_ID" "$SAC_LIQF" "$spoke_id" false true 9500 9700 200)" >/dev/null
    fi
    view spoke_view "$CONTROLLER" -- get_spoke --spoke_id "$SPOKE_ID" >/dev/null

    local acct
    acct=$(inv_create liq3_supply_spoke "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQE" $((1000 * LIQ_UNIT)))" | tr -d '"')

    inv liq3_borrow_spoke "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQF" $((920 * LIQ_UNIT)))" --to null >/dev/null

    dual_px "$SAC_LIQE" LIQE $((WAD / 100 * 94)) liq3_crash
    assert_hf_below_wad liq3_hf "$acct"
    local liq3_debt_pre=$((920 * LIQ_UNIT))
    inv liq3_liquidate_spoke "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_transfer)" \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQF" $((400 * LIQ_UNIT)))" >/dev/null
    assert_borrow_decreased liq3_debt_post "$acct" "$SAC_LIQF" "$liq3_debt_pre"
    save_state LIQ3_ACCT "$acct"
}

# Share-credit liquidation (`SeizeMode::Credit`), covering both admission paths
# and every binding rule from ADR-0019. `liquidate` returns the receiving
# account id — 0 for Transfer, the new id for Credit(0), the same id back for
# Credit(<existing>) — which is what makes these assertions possible.
#
# Runs after flow_liq_spoke so SPOKE_ID exists: the spoke-mismatch rejection
# needs a liquidator-owned account sitting in a *different* spoke.
flow_liq_credit() {
    phase liq_credit

    local acct
    acct=$(inv_create liqcr_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQG" $((1000 * LIQ_UNIT)))" | tr -d '"') || return 1
    inv liqcr_borrow "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((600 * LIQ_UNIT)))" --to null >/dev/null

    dual_px "$SAC_LIQG" LIQG $((WAD / 10 * 7)) liqcr_crash
    assert_hf_below_wad liqcr_hf "$acct"

    # --- Credit(0): mints a fresh receiving account owned by the liquidator ---
    local recv
    recv=$(inv liqcr_liquidate_new "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_credit 0)" \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((100 * LIQ_UNIT)))" | tr -d '"')

    if [ -z "$recv" ] || [ "$recv" = "0" ] || [ "$recv" = "$acct" ]; then
        _assert_fail liqcr_new_account_id "Credit(0) returned '$recv'; want a fresh id != 0 and != $acct"
    else
        record liqcr_new_account_id ok liquidate "" "" "" "" "" "receiver=$recv"
    fi

    assert_bool_view liqcr_new_account_exists true account_exists --account_id "$recv"
    # Net of the protocol fee, so only positivity is asserted here; the exact
    # net-vs-gross split is what the LiqSeize/LiqCredit event pair carries.
    assert_int_view_positive liqcr_new_credited get_collateral_amount \
        --account_id "$recv" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQG")"
    assert_borrow_decreased liqcr_debt_post_new "$acct" "$SAC_LIQB" $((600 * LIQ_UNIT))

    # --- Credit(<existing>): credits the same account a second time ---
    local credited_pre recv2 liqcr_debt_pre_existing
    credited_pre=$(_view_int liqcr_credited_pre get_collateral_amount \
        --account_id "$recv" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQG")")
    liqcr_debt_pre_existing=$(_view_int liqcr_debt_pre_existing get_borrow_amount \
        --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQB")")

    recv2=$(inv liqcr_liquidate_existing "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_credit "$recv")" \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((50 * LIQ_UNIT)))" | tr -d '"')

    if [ "$recv2" != "$recv" ]; then
        _assert_fail liqcr_existing_account_id "Credit($recv) returned '$recv2'; want the same id back"
    else
        record liqcr_existing_account_id ok liquidate "" "" "" "" "" "receiver=$recv2"
    fi
    assert_borrow_decreased liqcr_debt_post_existing "$acct" "$SAC_LIQB" "$liqcr_debt_pre_existing"

    local credited_post
    credited_post=$(_view_int liqcr_credited_post get_collateral_amount \
        --account_id "$recv" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQG")")
    if [ "$credited_post" -gt "$credited_pre" ]; then
        record liqcr_credit_accumulates ok liquidate "" "" "" "" "" "$credited_pre -> $credited_post"
    else
        _assert_fail liqcr_credit_accumulates "collateral $credited_pre -> $credited_post; want an increase"
    fi

    save_state LIQCR_ACCT "$acct"
    save_state LIQCR_RECV "$recv"
}

# The binding rules that make share-credit safe. Each must reject, or a
# liquidator could move seized collateral into an account the protocol never
# vetted — a different owner, a different risk regime, or back to the victim.
flow_liq_credit_rejections() {
    phase liq_credit_reject
    [ -n "${LIQCR_ACCT:-}" ] || { log "liq_credit_reject: no LIQCR_ACCT, skipping"; return 0; }

    # Crediting the liquidated account itself would hand the collateral straight back.
    xfail liqcr_reject_self 'Error\(Contract, #133\)' "$CAROL" "$CONTROLLER" -- liquidate \
        --seize_mode "$(seize_credit "$LIQCR_ACCT")" \
        --liquidator "$CAROL_ADDR" --account_id "$LIQCR_ACCT" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((10 * LIQ_UNIT)))"

    # An account the liquidator neither owns nor is delegated on.
    if [ -n "${LIQ1_ACCT:-}" ] && [ "${LIQ1_ACCT}" != "${LIQCR_ACCT}" ]; then
        xfail liqcr_reject_not_owner 'Error\(Contract, #44\)' "$CAROL" "$CONTROLLER" -- liquidate \
            --seize_mode "$(seize_credit "$LIQ1_ACCT")" \
            --liquidator "$CAROL_ADDR" --account_id "$LIQCR_ACCT" \
            --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((10 * LIQ_UNIT)))"
    fi

    # A liquidator-owned account bound to a different spoke: the credited shares
    # are the liquidated spoke's supply, and an account's spoke is what supplies
    # the risk configuration for everything it holds.
    if [ -n "${SPOKE_ID:-}" ]; then
        local carol_other
        carol_other=$(inv_create liqcr_carol_other_spoke "$CAROL" "$CONTROLLER" -- supply \
            --caller "$CAROL_ADDR" --account_id 0 --spoke_id "$SPOKE_ID" \
            --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQE" $((10 * LIQ_UNIT)))" | tr -d '"') || return 0
        xfail liqcr_reject_spoke_mismatch 'Error\(Contract, #310\)' "$CAROL" "$CONTROLLER" -- liquidate \
            --seize_mode "$(seize_credit "$carol_other")" \
            --liquidator "$CAROL_ADDR" --account_id "$LIQCR_ACCT" \
            --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((10 * LIQ_UNIT)))"
    fi
}

# `set_spoke_asset_flags`, `set_spoke_liquidation_curve` and `get_spoke_usage`.
#
# Ordering is load-bearing. `set_spoke_asset_flags` can only tighten a flag, so
# halting LIQG's seizure leg is irreversible for the rest of the run — it has to
# come after flow_liq_credit is finished with LIQG. The curve change is applied
# to the secondary spoke so it cannot perturb the primary spoke's liquidations.
flow_spoke_flags_and_curve() {
    phase spoke_flags
    [ -n "${LIQCR_ACCT:-}" ] || { log "spoke_flags: no LIQCR_ACCT, skipping"; return 0; }

    local liqg_key
    liqg_key=$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQG")

    # Spoke usage is the only place per-spoke cap consumption is tracked, so it
    # is worth reading after the credit flow has moved supply around. Returns a
    # SpokeUsageRaw struct, so this is a field check rather than an int view.
    local usage supplied_ray
    usage=$(view sf_usage "$CONTROLLER" -- get_spoke_usage \
        --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$liqg_key")
    supplied_ray=$(jq -r '.supplied_scaled_ray // empty' <<<"$usage" 2>/dev/null)
    if [ -n "$supplied_ray" ] && [ "$supplied_ray" != "null" ]; then
        record sf_usage_supplied ok get_spoke_usage "" "" "" "" "" "supplied=$supplied_ray"
    else
        _assert_fail sf_usage_supplied "get_spoke_usage returned no supplied field: $usage"
    fi

    if [ -n "${SPOKE_ID:-}" ]; then
        inv sf_set_curve "$ADMIN" "$CONTROLLER" -- set_spoke_liquidation_curve \
            --id "$SPOKE_ID" --target_hf_wad $((WAD / 100 * 105)) \
            --hf_for_max_bonus_wad $((WAD / 100 * 85)) \
            --liquidation_bonus_factor_bps 9000 >/dev/null
        view sf_spoke_after_curve "$CONTROLLER" -- get_spoke --spoke_id "$SPOKE_ID" >/dev/null
    fi

    # Keep the account liquidatable so the rejection below can only be the
    # seizure halt, never a health-factor refusal.
    dual_px "$SAC_LIQG" LIQG $((WAD / 100 * 50)) sf_crash
    assert_can_liquidated sf_can_liq "$LIQCR_ACCT" true

    inv sf_set_no_seize "$ADMIN" "$CONTROLLER" -- set_spoke_asset_flags \
        --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$liqg_key" \
        --paused false --frozen false --no_seize true >/dev/null
    assert_market_field sf_no_seize_set "$SAC_LIQG" no_seize true

    # The whole point of the flag: a liquidatable account whose only collateral
    # is halted cannot have that collateral seized.
    xfail sf_seizure_halted 'Error\(Contract, #318\)' "$CAROL" "$CONTROLLER" -- liquidate \
        --seize_mode "$(seize_transfer)" \
        --liquidator "$CAROL_ADDR" --account_id "$LIQCR_ACCT" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((10 * LIQ_UNIT)))"
}

# `force_socialize_bad_debt` (owner-only) and `recapitalize` (permissionless).
#
# Runs after flow_clean_bad_debt so the permissionless cleanup path has already
# been exercised on its own account; this one builds a separate position and
# socializes it through the owner override instead.
flow_force_socialize_and_recap() {
    phase force_socialize
    # flow_clean_bad_debt leaves LIQC crashed to 15%, so a fresh position built
    # on it would be underwater before it is borrowed against — the borrow below
    # failed with #100 InsufficientCollateral. Restore the price first so this
    # flow controls its own setup regardless of what ran before it.
    dual_px "$SAC_LIQC" LIQC "$WAD" fs_restore

    local acct
    acct=$(inv_create fs_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQC" $((30 * LIQ_UNIT)))" | tr -d '"') || return 0
    inv fs_borrow "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQD" $((12 * LIQ_UNIT)))" --to null >/dev/null

    dual_px "$SAC_LIQC" LIQC $((WAD / 100 * 10)) fs_crash

    inv fs_force_socialize "$ADMIN" "$CONTROLLER" -- force_socialize_bad_debt \
        --account_id "$acct" >/dev/null
    assert_borrow_at_most fs_debt_cleared "$acct" "$SAC_LIQD" 0

    # Socialized bad debt leaves the pool short of its backing. recapitalize
    # applies only up to that shortfall and refunds the rest, so an oversized
    # payment probes how much shortfall exists without risking an overpay.
    # Invoked for real, not simulated: it moves tokens from the payer.
    inv fs_recapitalize "$CAROL" "$CONTROLLER" -- recapitalize \
        --payer "$CAROL_ADDR" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQD")" \
        --amount $((100 * LIQ_UNIT)) >/dev/null
}

flow_clean_bad_debt() {
    phase clean_bad_debt
    xfail cbd_healthy 'Error\(Contract, #114\)' "$ADMIN" "$CONTROLLER" -- clean_bad_debt \
        --caller "$ADMIN_ADDR" --account_id "${LIQ2_ACCT:-1}"
    local acct
    acct=$(inv_create cbd_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQC" $((30 * LIQ_UNIT)))" | tr -d '"')
    inv cbd_borrow "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((12 * LIQ_UNIT)))" --to null >/dev/null

    dual_px "$SAC_LIQC" LIQC $((WAD / 100 * 15)) cbd_crash
    inv cbd_clean "$ADMIN" "$CONTROLLER" -- clean_bad_debt \
        --caller "$ADMIN_ADDR" --account_id "$acct" >/dev/null
    assert_borrow_at_most cbd_debt_cleared "$acct" "$SAC_LIQB" 0
}

# 2026-09 gap hunt, GH-23. `remove_spoke` has no usage check and deprecation
# is one-way, so a spoke can hold live positions forever. Liquidation must stay
# open there for a liquidator with no account in that spoke: `Credit(0)`
# creates the receiver inside the deprecated spoke, while a plain supply into a
# new account there is still refused. Runs last in the liquidation block so the
# LIQE/LIQF price moves cannot disturb the earlier flows; teardown drains the
# two accounts it leaves behind, which needs no active spoke.
flow_liq_deprecated_spoke_credit() {
    phase liq_deprecated_spoke
    if [ -n "${DEPR_SPOKE_DONE:-}" ]; then
        log "deprecated-spoke liquidation already recorded; skipping"
        return 0
    fi
    local spoke
    spoke=$(inv depr_spoke_add "$ADMIN" "$CONTROLLER" -- add_spoke | tr -d '"[:space:]')
    [[ "$spoke" =~ ^[1-9][0-9]*$ ]] || die depr_spoke_add "add_spoke returned invalid spoke id '$spoke'"
    inv depr_spoke_add_liqe "$ADMIN" "$CONTROLLER" -- add_asset_to_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$SAC_LIQE" "$spoke" true false 9000 9500 300)" >/dev/null
    inv depr_spoke_add_liqf "$ADMIN" "$CONTROLLER" -- add_asset_to_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$SAC_LIQF" "$spoke" false true 9000 9500 300)" >/dev/null
    # The mock feeds are shared with flow_liq_spoke, so pin both at one dollar
    # before sizing the position.
    dual_px "$SAC_LIQE" LIQE "$WAD" depr_px_reset_e
    dual_px "$SAC_LIQF" LIQF "$WAD" depr_px_reset_f

    local acct
    acct=$(inv_create depr_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$spoke" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQE" $((1000 * LIQ_UNIT)))" | tr -d '"') || return 1
    inv depr_borrow "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQF" $((850 * LIQ_UNIT)))" --to null >/dev/null

    inv depr_spoke_deprecate "$ADMIN" "$CONTROLLER" -- remove_spoke --id "$spoke" >/dev/null
    # New exposure through a fresh account stays closed (#301).
    xfail depr_supply_new_account 'Error\(Contract, #301\)' "$CAROL" "$CONTROLLER" -- supply \
        --caller "$CAROL_ADDR" --account_id 0 --spoke_id "$spoke" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQE" $((10 * LIQ_UNIT)))"

    dual_px "$SAC_LIQE" LIQE $((WAD / 100 * 85)) depr_crash
    assert_hf_below_wad depr_hf "$acct"

    # CAROL owns no account in this spoke. Before the fix Credit(0) died on
    # the deprecated-spoke check inside account creation.
    local recv
    recv=$(inv depr_liquidate_credit0 "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_credit 0)" \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQF" $((200 * LIQ_UNIT)))" | tr -d '"')
    if [ -z "$recv" ] || [ "$recv" = "0" ] || [ "$recv" = "$acct" ]; then
        _assert_fail depr_receiver_created "Credit(0) returned '$recv'; want a fresh id != 0 and != $acct"
    else
        record depr_receiver_created ok liquidate "" "" "" "" "" "receiver=$recv"
        local recv_spoke
        recv_spoke=$(view depr_receiver_attrs "$CONTROLLER" -- get_account_attributes --account_id "$recv" \
            | jq -r '.spoke_id // empty' 2>/dev/null)
        if [ "$recv_spoke" = "$spoke" ]; then
            record depr_receiver_in_deprecated_spoke ok get_account_attributes "" "" "" "" "" "spoke_id=$recv_spoke"
        else
            _assert_fail depr_receiver_in_deprecated_spoke "receiver sits in spoke '$recv_spoke', want $spoke"
        fi
    fi
    assert_borrow_decreased depr_debt_post "$acct" "$SAC_LIQF" $((850 * LIQ_UNIT))
    assert_borrow_at_most depr_debt_cap "$acct" "$SAC_LIQF" $((651 * LIQ_UNIT))
    save_state DEPR_SPOKE_ID "$spoke"
    save_state DEPR_SPOKE_DONE 1
}
