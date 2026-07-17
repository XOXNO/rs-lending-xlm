# Liquidation suite on mock-priced markets: partial / full / multi-debt bulk
# liquidations, spoke liquidation, clean_bad_debt, and the
# healthy-account guard reverts. Mock prices make HF crashes deterministic.
#
# Markets use the anchored (Reflector-mock primary + RedStone-mock anchor) shape
# so the wide sanity band survives the crashes below: a `Single`-strategy band is
# capped at +/-10%, but these tests crash prices to ~15% of the listing price.
# Both legs are moved in lock-step (`dual_px`), so the resolved midpoint equals
# the intended price and the primary/anchor tolerance check stays satisfied.
#
# Scenario ordering matters: price crashes are market-global, so accounts
# created later are sized against the already-crashed price.

: "${LIQ_UNIT:=10000000}"
: "${LIQ_CODES:=(LIQA LIQB LIQC LIQD LIQE LIQF LIQG)}"

# Issues SACs, trustlines, mints, creates single-source mock markets at $1,
# seeds debt-side liquidity.
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
        # Seed primary + anchor at the listing price ($1) before configuring.
        dual_px "$sac" "$code" "$WAD" "px_init_$code"
    done
    # Markets: LIQE/LIQF get spoke-friendly base config. Anchored shape (feed id =
    # asset code) keeps the wide sanity band exempt from the single-source cap.
    create_market LIQA "$PRIMARY_HUB_ID" "$SAC_LIQA" 7 "$(oracle_cfg_mock_dual "$SAC_LIQA" LIQA)" "$(asset_config_json 7000 7500 800)"
    create_market LIQB "$PRIMARY_HUB_ID" "$SAC_LIQB" 7 "$(oracle_cfg_mock_dual "$SAC_LIQB" LIQB)" "$(asset_config_json 7000 7500 800)"
    create_market LIQC "$PRIMARY_HUB_ID" "$SAC_LIQC" 7 "$(oracle_cfg_mock_dual "$SAC_LIQC" LIQC)" "$(asset_config_json 7000 7500 800)"
    create_market LIQD "$PRIMARY_HUB_ID" "$SAC_LIQD" 7 "$(oracle_cfg_mock_dual "$SAC_LIQD" LIQD)" "$(asset_config_json 7000 7500 800)"
    create_market LIQE "$PRIMARY_HUB_ID" "$SAC_LIQE" 7 "$(oracle_cfg_mock_dual "$SAC_LIQE" LIQE)" "$(asset_config_json 7000 7500 200)"
    create_market LIQF "$PRIMARY_HUB_ID" "$SAC_LIQF" 7 "$(oracle_cfg_mock_dual "$SAC_LIQF" LIQF)" "$(asset_config_json 7000 7500 200)"
    create_market LIQG "$PRIMARY_HUB_ID" "$SAC_LIQG" 7 "$(oracle_cfg_mock_dual "$SAC_LIQG" LIQG)" "$(asset_config_json 7000 7500 800)"
    # Carol seeds liquidity on every borrow-side market in one bulk supply
    # (the issuer cannot hold a trustline to its own asset, so the admin
    # never holds LIQ tokens itself).
    inv liq_seed_liquidity "$CAROL" "$CONTROLLER" -- supply \
        --caller "$CAROL_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((50000 * LIQ_UNIT)) "$SAC_LIQD" $((50000 * LIQ_UNIT)) "$SAC_LIQF" $((50000 * LIQ_UNIT)))" >/dev/null || return 1
    save_state LIQ_SETUP_DONE 1
}

# Partial then full liquidation of a single-collateral / single-debt account.
flow_liq_single() {
    phase liq_single
    local acct
    acct=$(inv liq1_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQA" $((1000 * LIQ_UNIT)))" | tr -d '"')
    inv liq1_borrow "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((600 * LIQ_UNIT)))" --to null >/dev/null

    assert_can_liquidated liq1_can_liq_pre "$acct" false
    xfail liq1_liquidate_healthy 'Error\(Contract, #101\)' "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((100 * LIQ_UNIT)))"

    # Crash collateral 30% → HF ≈ 0.875.
    dual_px "$SAC_LIQA" LIQA $((WAD / 10 * 7)) liq1_crash
    assert_hf_below_wad liq1_hf "$acct"
    assert_can_liquidated liq1_can_liq "$acct" true
    view liq1_estimate "$CONTROLLER" -- get_liquidation_estimate \
        --account_id "$acct" --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((100 * LIQ_UNIT)))" >/dev/null
    view liq1_avail "$CONTROLLER" -- get_liquidation_collateral --account_id "$acct" >/dev/null

    local liq1_debt_pre_partial=$((600 * LIQ_UNIT))
    inv liq1_liquidate_partial "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((100 * LIQ_UNIT)))" >/dev/null
    assert_borrow_decreased liq1_debt_post_partial "$acct" "$SAC_LIQB" "$liq1_debt_pre_partial"
    # Repaid 100 of 600 → ~500 remain, plus a little accrued interest (+1 unit buffer).
    assert_borrow_at_most liq1_debt_cap_partial "$acct" "$SAC_LIQB" $(( 501 * LIQ_UNIT ))

    # Full liquidation. An overpay cannot be submitted directly: the contract
    # transfers the ON-CHAIN recomputed close amount from the liquidator, so
    # the signed auth (recorded at simulation) goes stale as interest accrues
    # → Error(Auth, InvalidAction) at apply. Estimate the close via the view,
    # then submit just under it — payments ≤ close transfer the exact user
    # amount and stay auth-stable. (Design note for liquidation bots.)
    local est refund close
    est=$(view liq1_estimate_close "$CONTROLLER" -- get_liquidation_estimate \
        --account_id "$acct" --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((600 * LIQ_UNIT)))")
    refund=$(jq -r '[.refunds[]?.amount | tonumber] | add // 0' <<<"$est")
    close=$(( 600 * LIQ_UNIT - refund ))
    leg_liq1_full() {
        inv liq1_liquidate_full "$CAROL" "$CONTROLLER" -- liquidate \
            --liquidator "$CAROL_ADDR" --account_id "$acct" \
            --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $(( close * 998 / 1000 )))" >/dev/null
    }
    retry_leg leg_liq1_full
    # A single liquidation can't fully clear (close-factor + collateral cap a
    # per-call seizure), so the debt is heavily reduced (600 -> well under 100),
    # not zeroed. The residual is the protocol's per-liquidation limit.
    assert_borrow_at_most liq1_debt_cleared "$acct" "$SAC_LIQB" $(( 100 * LIQ_UNIT ))
    save_state LIQ1_ACCT "$acct"
}

# Multi-collateral / multi-debt account liquidated with a BULK repay
# (two debt assets in one liquidate tx) — seizes proportionally from all
# collaterals.
flow_liq_bulk() {
    phase liq_bulk
    local acct
    acct=$(inv liq2_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQC" $((800 * LIQ_UNIT)) "$SAC_LIQA" $((1143 * LIQ_UNIT)))" | tr -d '"')
    inv liq2_borrow_bulk "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((500 * LIQ_UNIT)) "$SAC_LIQD" $((500 * LIQ_UNIT)))" --to null >/dev/null

    # Crash both collaterals 30%; LIQA moves from 0.70 to 0.49.
    dual_px "$SAC_LIQC" LIQC $((WAD / 10 * 7)) liq2_crash_c
    dual_px "$SAC_LIQA" LIQA $((WAD / 100 * 49)) liq2_crash_a
    assert_hf_below_wad liq2_hf "$acct"

    local liq2_debt_b_pre liq2_debt_d_pre
liq2_debt_b_pre=$(_view_int liq2_debt_b_pre get_borrow_amount --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQB")")
liq2_debt_d_pre=$(_view_int liq2_debt_d_pre get_borrow_amount --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQD")")
    inv liq2_liquidate_bulk "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((150 * LIQ_UNIT)) "$SAC_LIQD" $((150 * LIQ_UNIT)))" >/dev/null
    assert_borrow_decreased liq2_debt_b_post "$acct" "$SAC_LIQB" "$liq2_debt_b_pre"
    assert_borrow_decreased liq2_debt_d_post "$acct" "$SAC_LIQD" "$liq2_debt_d_pre"
    save_state LIQ2_ACCT "$acct"
}

# Spoke category lifecycle + liquidation inside the category.
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
    acct=$(inv liq3_supply_spoke "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQE" $((1000 * LIQ_UNIT)))" | tr -d '"')
    # 92% LTV borrow — only possible inside the spoke category (asset LTV is 70%).
    inv liq3_borrow_spoke "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQF" $((920 * LIQ_UNIT)))" --to null >/dev/null
    # 6% price drop puts HF under 1 at the 97% spoke threshold.
    dual_px "$SAC_LIQE" LIQE $((WAD / 100 * 94)) liq3_crash
    assert_hf_below_wad liq3_hf "$acct"
    local liq3_debt_pre=$((920 * LIQ_UNIT))
    inv liq3_liquidate_spoke "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQF" $((400 * LIQ_UNIT)))" >/dev/null
    assert_borrow_decreased liq3_debt_post "$acct" "$SAC_LIQF" "$liq3_debt_pre"
    save_state LIQ3_ACCT "$acct"
}

# Direct bad-debt socialization without a liquidation: tiny position, crash
# collateral until total collateral ≤ the $5 bad-debt threshold while
# debt > collateral, then KEEPER clean_bad_debt closes it.
flow_clean_bad_debt() {
    phase clean_bad_debt
    xfail cbd_healthy 'Error\(Contract, #114\)' "$ADMIN" "$CONTROLLER" -- clean_bad_debt \
        --caller "$ADMIN_ADDR" --account_id "${LIQ2_ACCT:-1}"
    local acct
    acct=$(inv cbd_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQC" $((30 * LIQ_UNIT)))" | tr -d '"')
    inv cbd_borrow "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((12 * LIQ_UNIT)))" --to null >/dev/null
    # LIQC is at $0.70 → 30 units = $21 collateral, $12 debt. Crash to $0.15:
    # collateral $4.50 ≤ $5 threshold and debt > collateral → socializable.
    dual_px "$SAC_LIQC" LIQC $((WAD / 100 * 15)) cbd_crash
    inv cbd_clean "$ADMIN" "$CONTROLLER" -- clean_bad_debt \
        --caller "$ADMIN_ADDR" --account_id "$acct" >/dev/null
    assert_borrow_at_most cbd_debt_cleared "$acct" "$SAC_LIQB" 0
}

