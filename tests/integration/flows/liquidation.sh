# Liquidation suite on mock-priced markets: partial / full / multi-debt bulk
# liquidations, e-mode liquidation, clean_bad_debt, and the
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
    # Markets: LIQE/LIQF get e-mode-friendly base config.
    create_market LIQA "$SAC_LIQA" 7 "$(oracle_cfg_mock_single "$SAC_LIQA")" "$(asset_config_json 7000 7500 800)"
    create_market LIQB "$SAC_LIQB" 7 "$(oracle_cfg_mock_single "$SAC_LIQB")" "$(asset_config_json 7000 7500 800)"
    create_market LIQC "$SAC_LIQC" 7 "$(oracle_cfg_mock_single "$SAC_LIQC")" "$(asset_config_json 7000 7500 800)"
    create_market LIQD "$SAC_LIQD" 7 "$(oracle_cfg_mock_single "$SAC_LIQD")" "$(asset_config_json 7000 7500 800)"
    create_market LIQE "$SAC_LIQE" 7 "$(oracle_cfg_mock_single "$SAC_LIQE")" "$(asset_config_json 7000 7500 200)"
    create_market LIQF "$SAC_LIQF" 7 "$(oracle_cfg_mock_single "$SAC_LIQF")" "$(asset_config_json 7000 7500 200)"
    create_market LIQG "$SAC_LIQG" 7 "$(oracle_cfg_mock_single "$SAC_LIQG")" "$(asset_config_json 7000 7500 800)"
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
    assert_can_liquidated liq1_can_liq_pre "$acct" false
    xfail liq1_liquidate_healthy 'Error\(Contract, #101\)' "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$SAC_LIQB" $((100 * LIQ_UNIT)))"

    # Crash collateral 30% → HF ≈ 0.875.
    set_mock_price "$SAC_LIQA" $((WAD / 10 * 7)) liq1_crash
    assert_hf_below_wad liq1_hf "$acct"
    assert_can_liquidated liq1_can_liq "$acct" true
    view liq1_estimate "$CONTROLLER" -- liquidation_estimations_detailed \
        --account_id "$acct" --debt_payments "$(pay_vec "$SAC_LIQB" $((100 * LIQ_UNIT)))" >/dev/null
    view liq1_avail "$CONTROLLER" -- liquidation_collateral_available --account_id "$acct" >/dev/null

    # Partial liquidation: repay 100 of 600 debt.
    local liq1_debt_pre_partial=$((600 * LIQ_UNIT))
    inv liq1_liquidate_partial "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$SAC_LIQB" $((100 * LIQ_UNIT)))" >/dev/null
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
        --caller "$BOB_ADDR" --account_id 0 --e_mode_category 0 \
        --assets "$(pay_vec "$SAC_LIQC" $((800 * LIQ_UNIT)) "$SAC_LIQA" $((1143 * LIQ_UNIT)))" | tr -d '"')
    inv liq2_borrow_bulk "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$SAC_LIQB" $((500 * LIQ_UNIT)) "$SAC_LIQD" $((500 * LIQ_UNIT)))" >/dev/null

    # Crash both collaterals 30% (LIQA from its current 0.70 → 0.49).
    set_mock_price "$SAC_LIQC" $((WAD / 10 * 7)) liq2_crash_c
    set_mock_price "$SAC_LIQA" $((WAD / 100 * 49)) liq2_crash_a
    assert_hf_below_wad liq2_hf "$acct"

    local liq2_debt_b_pre liq2_debt_d_pre
    liq2_debt_b_pre=$(_view_int liq2_debt_b_pre borrow_amount_for_token --account_id "$acct" --asset "$SAC_LIQB")
    liq2_debt_d_pre=$(_view_int liq2_debt_d_pre borrow_amount_for_token --account_id "$acct" --asset "$SAC_LIQD")
    inv liq2_liquidate_bulk "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$SAC_LIQB" $((150 * LIQ_UNIT)) "$SAC_LIQD" $((150 * LIQ_UNIT)))" >/dev/null
    assert_borrow_decreased liq2_debt_b_post "$acct" "$SAC_LIQB" "$liq2_debt_b_pre"
    assert_borrow_decreased liq2_debt_d_post "$acct" "$SAC_LIQD" "$liq2_debt_d_pre"
    save_state LIQ2_ACCT "$acct"
}

# E-mode category lifecycle + liquidation inside the category.
flow_liq_emode() {
    phase liq_emode
    if [ -z "${EMODE_ID:-}" ]; then
        local emode_id
        emode_id=$(inv emode_add_category "$ADMIN" "$CONTROLLER" -- add_e_mode_category | tr -d '"')
        save_state EMODE_ID "$emode_id"
        inv emode_add_liqe "$ADMIN" "$CONTROLLER" -- add_asset_to_e_mode_category \
            --asset "$SAC_LIQE" --category_id "$emode_id" --can_collateral true --can_borrow false \
            --ltv 9500 --threshold 9700 --bonus 200 >/dev/null
        inv emode_add_liqf "$ADMIN" "$CONTROLLER" -- add_asset_to_e_mode_category \
            --asset "$SAC_LIQF" --category_id "$emode_id" --can_collateral false --can_borrow true \
            --ltv 9500 --threshold 9700 --bonus 200 >/dev/null
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
    assert_hf_below_wad liq3_hf "$acct"
    local liq3_debt_pre=$((920 * LIQ_UNIT))
    inv liq3_liquidate_emode "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$SAC_LIQF" $((400 * LIQ_UNIT)))" >/dev/null
    assert_borrow_decreased liq3_debt_post "$acct" "$SAC_LIQF" "$liq3_debt_pre"
    save_state LIQ3_ACCT "$acct"
}

# Direct bad-debt socialization without a liquidation: tiny position, crash
# collateral until total collateral ≤ the $5 bad-debt threshold while
# debt > collateral, then KEEPER clean_bad_debt closes it.
flow_clean_bad_debt() {
    phase clean_bad_debt
    # Healthy-account guard first.
    xfail cbd_healthy 'Error\(Contract, #114\)' "$ADMIN" "$CONTROLLER" -- clean_bad_debt \
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
    assert_borrow_at_most cbd_debt_cleared "$acct" "$SAC_LIQB" 0
}

# Supply-cap / borrow-cap revert coverage on a dedicated, stable ($1) mock market.
# borrow_cap/supply_cap default to 0 (disabled) protocol-wide, so these reverts
# are otherwise never exercised. Each cap is tightened via edit_asset_config,
# breached (#105 / #106), then reset to disabled so nothing leaks to later flows.
flow_caps() {
    phase caps
    if [ -z "${CAP_SETUP_DONE:-}" ]; then
        # CAP: the capped market (borrow target). CAPC: a stable mock collateral
        # so the borrow-cap test needs no real market — keeps this flow pure-mock.
        issue_sac SAC_CAP CAP
        issue_sac SAC_CAPC CAPC
        for code in CAP CAPC; do
            trustline "$BOB" "$code" "$ADMIN_ADDR"
            trustline "$CAROL" "$code" "$ADMIN_ADDR"
        done
        mint_to "$SAC_CAP"  CAP  "$BOB_ADDR"   $((100000 * LIQ_UNIT))
        mint_to "$SAC_CAP"  CAP  "$CAROL_ADDR" $((100000 * LIQ_UNIT))
        mint_to "$SAC_CAPC" CAPC "$BOB_ADDR"   $((100000 * LIQ_UNIT))
        set_mock_price "$SAC_CAP"  "$WAD" px_init_CAP
        set_mock_price "$SAC_CAPC" "$WAD" px_init_CAPC
        create_market CAP  "$SAC_CAP"  7 "$(oracle_cfg_mock_single "$SAC_CAP")"  "$(asset_config_json 7000 7500 800)"
        create_market CAPC "$SAC_CAPC" 7 "$(oracle_cfg_mock_single "$SAC_CAPC")" "$(asset_config_json 7000 7500 800)"
        # Carol seeds CAP liquidity so the borrow-cap test has cash to draw.
        inv cap_seed "$CAROL" "$CONTROLLER" -- supply \
            --caller "$CAROL_ADDR" --account_id 0 --e_mode_category 0 \
            --assets "$(pay_vec "$SAC_CAP" $((20000 * LIQ_UNIT)))" >/dev/null || return 1
        save_state CAP_SETUP_DONE 1
    fi

    # Supply cap: tighten below current supply, breach, reset to disabled.
    inv cap_supply_tighten "$ADMIN" "$CONTROLLER" -- edit_asset_config \
        --asset "$SAC_CAP" --cfg "$(asset_config_json 7000 7500 800 '.supply_cap="1"')" >/dev/null
    xfail cap_supply_breach 'Error\(Contract, #105\)' "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --e_mode_category 0 \
        --assets "$(pay_vec "$SAC_CAP" $((1000 * LIQ_UNIT)))"
    inv cap_supply_reset "$ADMIN" "$CONTROLLER" -- edit_asset_config \
        --asset "$SAC_CAP" --cfg "$(asset_config_json 7000 7500 800)" >/dev/null

    # Borrow cap: BOB supplies stable CAPC collateral, then a CAP borrow above the
    # cap reverts (#106). The tiny borrow stays well within LTV so #106, not #100.
    local cap_acct
    cap_acct=$(inv cap_coll_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --e_mode_category 0 \
        --assets "$(pay_vec "$SAC_CAPC" $((3000 * LIQ_UNIT)))" | tr -d '"') || return 1
    inv cap_borrow_tighten "$ADMIN" "$CONTROLLER" -- edit_asset_config \
        --asset "$SAC_CAP" --cfg "$(asset_config_json 7000 7500 800 '.borrow_cap="1"')" >/dev/null
    xfail cap_borrow_breach 'Error\(Contract, #106\)' "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$cap_acct" \
        --borrows "$(pay_vec "$SAC_CAP" $((10 * LIQ_UNIT)))"
    inv cap_borrow_reset "$ADMIN" "$CONTROLLER" -- edit_asset_config \
        --asset "$SAC_CAP" --cfg "$(asset_config_json 7000 7500 800)" >/dev/null
}
