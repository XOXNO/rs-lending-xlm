# Liquidation suite on mock-priced markets: partial / full / multi-debt bulk
# liquidations, e-mode liquidation, isolation gates, clean_bad_debt, and the
# healthy-account guard reverts. Mock prices make HF crashes deterministic.
#
# Scenario ordering matters: price crashes are market-global, so accounts
# created later are sized against the already-crashed price.

LIQ_CODES=(LIQA LIQB LIQC LIQD LIQE LIQF LIQG)
LIQ_UNIT=10000000   # 1.0 token at 7 decimals

# Issues SACs, trustlines, mints, creates single-source mock markets at $1,
# seeds debt-side liquidity.
flow_liq_setup() {
    phase liq_setup
    [ -n "${LIQ_SETUP_DONE:-}" ] && return 0
    deploy_mock_reflector
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
        set_mock_price "$sac" "$WAD" "px_init_$code"
    done
    # Markets: LIQE/LIQF get e-mode-friendly base config; LIQG is isolated.
    create_market LIQA "$SAC_LIQA" 7 "$(oracle_cfg_mock_single "$SAC_LIQA")" "$(asset_config_json 7000 7500 800)"
    create_market LIQB "$SAC_LIQB" 7 "$(oracle_cfg_mock_single "$SAC_LIQB")" "$(asset_config_json 7000 7500 800)"
    create_market LIQC "$SAC_LIQC" 7 "$(oracle_cfg_mock_single "$SAC_LIQC")" "$(asset_config_json 7000 7500 800)"
    create_market LIQD "$SAC_LIQD" 7 "$(oracle_cfg_mock_single "$SAC_LIQD")" "$(asset_config_json 7000 7500 800)"
    create_market LIQE "$SAC_LIQE" 7 "$(oracle_cfg_mock_single "$SAC_LIQE")" "$(asset_config_json 7000 7500 200)"
    create_market LIQF "$SAC_LIQF" 7 "$(oracle_cfg_mock_single "$SAC_LIQF")" "$(asset_config_json 7000 7500 200)"
    create_market LIQG "$SAC_LIQG" 7 "$(oracle_cfg_mock_single "$SAC_LIQG")" \
        "$(asset_config_json 7000 7500 800 '.is_isolated_asset=true | .isolation_debt_ceiling_usd_wad="1000000000000000000000"')"
    # Carol seeds liquidity on every borrow-side market in one bulk supply
    # (the issuer cannot hold a trustline to its own asset, so the admin
    # never holds LIQ tokens itself).
    inv liq_seed_liquidity "$CAROL" "$CONTROLLER" -- supply \
        --caller "$CAROL_ADDR" --account_id 0 --e_mode_category 0 \
        --assets "$(pay_vec "$SAC_LIQB" $((50000 * LIQ_UNIT)) "$SAC_LIQD" $((50000 * LIQ_UNIT)) "$SAC_LIQF" $((50000 * LIQ_UNIT)))" >/dev/null || return 1
    save_state LIQ_SETUP_DONE 1
}

# Partial then full liquidation of a single-collateral / single-debt account.
flow_liq_single() {
    phase liq_single
    local acct
    acct=$(inv liq1_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --e_mode_category 0 \
        --assets "$(pay_vec "$SAC_LIQA" $((1000 * LIQ_UNIT)))" | tr -d '"')
    inv liq1_borrow "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$SAC_LIQB" $((600 * LIQ_UNIT)))" >/dev/null

    # Healthy-account guards.
    view liq1_can_liq_pre "$CONTROLLER" -- can_be_liquidated --account_id "$acct" >/dev/null
    xfail liq1_liquidate_healthy 'Error\(Contract, #101\)' "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$SAC_LIQB" $((100 * LIQ_UNIT)))"

    # Crash collateral 30% → HF ≈ 0.875.
    set_mock_price "$SAC_LIQA" $((WAD / 10 * 7)) liq1_crash
    view liq1_hf "$CONTROLLER" -- health_factor --account_id "$acct" >/dev/null
    view liq1_can_liq "$CONTROLLER" -- can_be_liquidated --account_id "$acct" >/dev/null
    view liq1_estimate "$CONTROLLER" -- liquidation_estimations_detailed \
        --account_id "$acct" --debt_payments "$(pay_vec "$SAC_LIQB" $((100 * LIQ_UNIT)))" >/dev/null
    view liq1_avail "$CONTROLLER" -- liquidation_collateral_available --account_id "$acct" >/dev/null

    # Partial liquidation: repay 100 of 600 debt.
    inv liq1_liquidate_partial "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$SAC_LIQB" $((100 * LIQ_UNIT)))" >/dev/null
    view liq1_hf_after_partial "$CONTROLLER" -- health_factor --account_id "$acct" >/dev/null

    # Full liquidation. An overpay cannot be submitted directly: the contract
    # transfers the ON-CHAIN recomputed close amount from the liquidator, so
    # the signed auth (recorded at simulation) goes stale as interest accrues
    # → Error(Auth, InvalidAction) at apply. Estimate the close via the view,
    # then submit just under it — payments ≤ close transfer the exact user
    # amount and stay auth-stable. (Design note for liquidation bots.)
    local est refund close
    est=$(view liq1_estimate_close "$CONTROLLER" -- liquidation_estimations_detailed \
        --account_id "$acct" --debt_payments "$(pay_vec "$SAC_LIQB" $((600 * LIQ_UNIT)))")
    refund=$(jq -r '[.refunds[]?.amount | tonumber] | add // 0' <<<"$est")
    close=$(( 600 * LIQ_UNIT - refund ))
    leg_liq1_full() {
        inv liq1_liquidate_full "$CAROL" "$CONTROLLER" -- liquidate \
            --liquidator "$CAROL_ADDR" --account_id "$acct" \
            --debt_payments "$(pay_vec "$SAC_LIQB" $(( close * 998 / 1000 )))" >/dev/null
    }
    retry_leg leg_liq1_full
    view liq1_positions_final "$CONTROLLER" -- get_account_positions --account_id "$acct" >/dev/null || true
    save_state LIQ1_ACCT "$acct"
}

# Multi-collateral / multi-debt account liquidated with a BULK repay
# (two debt assets in one liquidate tx) — seizes proportionally from all
# collaterals.
flow_liq_bulk() {
    phase liq_bulk
    local acct
    acct=$(inv liq2_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --e_mode_category 0 \
        --assets "$(pay_vec "$SAC_LIQC" $((800 * LIQ_UNIT)) "$SAC_LIQA" $((1143 * LIQ_UNIT)))" | tr -d '"')
    inv liq2_borrow_bulk "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$SAC_LIQB" $((500 * LIQ_UNIT)) "$SAC_LIQD" $((500 * LIQ_UNIT)))" >/dev/null

    # Crash both collaterals 30% (LIQA from its current 0.70 → 0.49).
    set_mock_price "$SAC_LIQC" $((WAD / 10 * 7)) liq2_crash_c
    set_mock_price "$SAC_LIQA" $((WAD / 100 * 49)) liq2_crash_a
    view liq2_hf "$CONTROLLER" -- health_factor --account_id "$acct" >/dev/null

    inv liq2_liquidate_bulk "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$SAC_LIQB" $((150 * LIQ_UNIT)) "$SAC_LIQD" $((150 * LIQ_UNIT)))" >/dev/null
    view liq2_positions_after "$CONTROLLER" -- get_account_positions --account_id "$acct" >/dev/null
    save_state LIQ2_ACCT "$acct"
}

# E-mode category lifecycle + liquidation inside the category.
flow_liq_emode() {
    phase liq_emode
    if [ -z "${EMODE_ID:-}" ]; then
        local emode_id
        emode_id=$(inv emode_add_category "$ADMIN" "$CONTROLLER" -- add_e_mode_category \
            --ltv 9500 --threshold 9700 --bonus 200 | tr -d '"')
        save_state EMODE_ID "$emode_id"
        inv emode_add_liqe "$ADMIN" "$CONTROLLER" -- add_asset_to_e_mode_category \
            --asset "$SAC_LIQE" --category_id "$emode_id" --can_collateral true --can_borrow false >/dev/null
        inv emode_add_liqf "$ADMIN" "$CONTROLLER" -- add_asset_to_e_mode_category \
            --asset "$SAC_LIQF" --category_id "$emode_id" --can_collateral false --can_borrow true >/dev/null
    fi
    view emode_view "$CONTROLLER" -- get_e_mode_category --category_id "$EMODE_ID" >/dev/null

    local acct
    acct=$(inv liq3_supply_emode "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --e_mode_category "$EMODE_ID" \
        --assets "$(pay_vec "$SAC_LIQE" $((1000 * LIQ_UNIT)))" | tr -d '"')
    # 92% LTV borrow — only possible inside the e-mode category (asset LTV is 70%).
    inv liq3_borrow_emode "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$SAC_LIQF" $((920 * LIQ_UNIT)))" >/dev/null
    # 6% price drop puts HF under 1 at the 97% e-mode threshold.
    set_mock_price "$SAC_LIQE" $((WAD / 100 * 94)) liq3_crash
    view liq3_hf "$CONTROLLER" -- health_factor --account_id "$acct" >/dev/null
    inv liq3_liquidate_emode "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$SAC_LIQF" $((400 * LIQ_UNIT)))" >/dev/null
    save_state LIQ3_ACCT "$acct"
}

# Isolation-mode gates: isolated collateral only borrows isolation-enabled
# assets; isolated debt is tracked against the ceiling.
flow_liq_isolation() {
    phase isolation
    local acct
    acct=$(inv iso_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --e_mode_category 0 \
        --assets "$(pay_vec "$SAC_LIQG" $((500 * LIQ_UNIT)))" | tr -d '"')
    xfail iso_borrow_blocked 'Error\(Contract, #305\)' "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$SAC_LIQB" $((50 * LIQ_UNIT)))"
    inv iso_enable_borrow "$ADMIN" "$CONTROLLER" -- edit_asset_config \
        --asset "$SAC_LIQB" --cfg "$(asset_config_json 7000 7500 800 '.isolation_borrow_enabled=true')" >/dev/null
    inv iso_borrow_ok "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$SAC_LIQB" $((50 * LIQ_UNIT)))" >/dev/null
    view iso_debt_view "$CONTROLLER" -- get_isolated_debt --asset "$SAC_LIQG" >/dev/null
    save_state ISO_ACCT "$acct"
}

# Direct bad-debt socialization without a liquidation: tiny position, crash
# collateral until total collateral ≤ the $5 bad-debt threshold while
# debt > collateral, then KEEPER clean_bad_debt closes it.
flow_clean_bad_debt() {
    phase clean_bad_debt
    # Healthy-account guard first.
    xfail cbd_healthy 'Error\(Contract, #1[0-9][0-9]\)' "$ADMIN" "$CONTROLLER" -- clean_bad_debt \
        --caller "$ADMIN_ADDR" --account_id "${LIQ2_ACCT:-1}"
    local acct
    acct=$(inv cbd_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --e_mode_category 0 \
        --assets "$(pay_vec "$SAC_LIQC" $((30 * LIQ_UNIT)))" | tr -d '"')
    inv cbd_borrow "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$SAC_LIQB" $((12 * LIQ_UNIT)))" >/dev/null
    # LIQC is at $0.70 → 30 units = $21 collateral, $12 debt. Crash to $0.15:
    # collateral $4.50 ≤ $5 threshold and debt > collateral → socializable.
    set_mock_price "$SAC_LIQC" $((WAD / 100 * 15)) cbd_crash
    inv cbd_clean "$ADMIN" "$CONTROLLER" -- clean_bad_debt \
        --caller "$ADMIN_ADDR" --account_id "$acct" >/dev/null
}
