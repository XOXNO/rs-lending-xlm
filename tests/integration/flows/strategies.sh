# Strategy endpoints (aggregator-routed) and flash-loan modes, on real markets.
#
# Swap payloads are aggregator routeXdr bytes (max_splits=1 — multi-hop
# payloads exceed the tx budget). swap_debt's `amount` is the NEW debt
# borrowed; size every leg above the $10 dust floor (#126).

# XDR-encodes FlashLoanRequest{mode} for the test receiver's `data` arg.
# (Swap legs use retry_leg from lib/invoke.sh — quotes are refreshed inside
# each callback, so every attempt re-simulates against current venue state.)
flash_data_hex() {
    local mode="$1"
    jq -nc --argjson m "$mode" '{map:[{key:{symbol:"mode"},val:{u32:$m}}]}' \
        | stellar xdr encode --type ScVal | base64 -d | xxd -p | tr -d '\n'
}

flow_flash_loans() {
    phase flash_loans
    # Fund the receiver so it can cover the flash-loan fee.
    sac_transfer "$ALICE" "$USDC_SAC" "$ALICE_ADDR" "$FLASH_RECEIVER" 50000000 fund_flash_receiver

    inv flash_loan_success "$ALICE" "$CONTROLLER" -- flash_loan \
        --caller "$ALICE_ADDR" --asset "$USDC_SAC" --amount 100000000 \
        --receiver "$FLASH_RECEIVER" --data "$(flash_data_hex 0)" >/dev/null

    # Each malicious mode must REVERT the flash loan; the exact code varies by
    # mechanism (the precise per-mode codes are pinned by the unit / Certora
    # flash_loan rules). #402 = InvalidFlashloanRepay, #400 = FlashLoanOngoing.
    #  - reenter_pool: the receiver re-enters pool.flash_loan while the pool is
    #    already on the call stack; Soroban 26.x forbids contract re-entry at the
    #    HOST level -> Error(Context, InvalidAction), not a #40x contract code.
    #  - panic: the receiver panics with ReceiverError::CallbackPanic (#3), which
    #    propagates as a contract error (or a host trap).
    #  - reenter_supply: the receiver re-enters controller.supply; the repay
    #    shortfall (#402) / FlashLoanOngoing (#400) / host re-entry block aborts it.
    local mode name pattern
    for mode in 1 2 3 4 5; do
        case $mode in
            1) name=no_repay; pattern='Error\(Contract, #402\)' ;;
            2) name=under_repay; pattern='Error\(Contract, #402\)' ;;
            3) name=reenter_pool; pattern='InvalidAction|re-entry|Error\(Contract, #40[0-9]\)' ;;
            4) name=panic; pattern='Error\(Contract, #3\)|Trapped|Error\(Contract, #40[0-9]\)' ;;
            5) name=reenter_supply; pattern='Error\(Contract, #40[0-9]\)|InvalidAction|re-entry' ;;
        esac
        xfail "flash_loan_$name" "$pattern" "$ALICE" "$CONTROLLER" -- flash_loan \
            --caller "$ALICE_ADDR" --asset "$USDC_SAC" --amount 100000000 \
            --receiver "$FLASH_RECEIVER" --data "$(flash_data_hex $mode)"
    done
}

# Strategy legs run exclusively on the XLM↔USDC venue: the testnet EURC AMM
# pool holds dust (1,500 XLM quotes to ~0.14 EURC), so EURC coverage stops at
# market creation + oracle + the aggregator purchase leg in the funding flow.
# Sizing accounts for the oracle/DEX divergence (Reflector XLM ≈ $0.19 vs DEX
# rate ≈ $0.134): HF math uses ORACLE prices while swap legs fill at DEX rates.
flow_strategies() {
    phase strategies
    # multiply LONG: flash USDC debt, swap to XLM collateral; alice fronts XLM.
    local flash_usdc=300000000   # 30 USDC
    local swap_hex
    swap_hex=$(agg_route_hex "$USDC_SAC" "$XLM_SAC" "$flash_usdc") || return 1
    local macct
    macct=$(inv multiply_long "$ALICE" "$CONTROLLER" -- multiply \
        --caller "$ALICE_ADDR" --account_id 0 --e_mode_category 0 \
        --collateral_token "$XLM_SAC" --debt_to_flash_loan "$flash_usdc" \
        --debt_token "$USDC_SAC" --mode 2 --swap "$swap_hex" \
        --initial_payment "[\"$XLM_SAC\",\"5000000000\"]" | tr -d '"')
    save_state ALICE_MACCT "$macct"
    log "multiply account = $macct"
    assert_hf_at_least hf_multiply "$macct" "$WAD"

    # swap_debt: convert part of the USDC debt into XLM debt (forward quote on
    # the NEW debt; both remaining USDC debt and new XLM debt stay above the
    # $10 dust floor).
    local new_xlm_debt=1000000000   # 100 XLM ≈ $19 oracle
    swap_hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" "$new_xlm_debt") || return 1
    inv swap_debt "$ALICE" "$CONTROLLER" -- swap_debt \
        --caller "$ALICE_ADDR" --account_id "$macct" \
        --existing_debt_token "$USDC_SAC" --amount "$new_xlm_debt" \
        --new_debt_token "$XLM_SAC" --swap "$swap_hex" >/dev/null
    # The migration created XLM debt (the new debt token) on the account.
    assert_borrow_at_least xlm_debt_post_swap "$macct" "$XLM_SAC" 500000000

    # swap_collateral: move 200 XLM of collateral into USDC.
    leg_swap_collateral() {
        local hex
        hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" 2000000000) || return 1
        inv swap_collateral "$ALICE" "$CONTROLLER" -- swap_collateral \
            --caller "$ALICE_ADDR" --account_id "$macct" \
            --current_collateral "$XLM_SAC" --amount 2000000000 \
            --new_collateral "$USDC_SAC" --swap "$hex" >/dev/null
    }
    retry_leg leg_swap_collateral

    # repay_debt_with_collateral: sell 500 XLM collateral toward the USDC
    # debt. XLM→USDC at ≥500 XLM is the only quote shape the testnet router
    # reliably serves 1-hop (small or reverse-direction quotes route through
    # broken middle pools). Widen the account first: extra XLM collateral
    # keeps LTV safe and a topped-up USDC debt keeps the post-repay residue
    # above the $10 floor.
    inv supply_for_rdwc "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id "$macct" --e_mode_category 0 \
        --assets "$(pay_vec "$XLM_SAC" 10000000000)" >/dev/null
    inv borrow_for_rdwc "$ALICE" "$CONTROLLER" -- borrow \
        --caller "$ALICE_ADDR" --account_id "$macct" \
        --borrows "$(pay_vec "$USDC_SAC" 550000000)" >/dev/null
    leg_repay_debt_with_coll() {
        local hex
        hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" 5000000000) || return 1
        inv repay_debt_with_coll "$ALICE" "$CONTROLLER" -- repay_debt_with_collateral \
            --caller "$ALICE_ADDR" --account_id "$macct" \
            --collateral_token "$XLM_SAC" --collateral_amount 5000000000 \
            --debt_token "$USDC_SAC" --swap "$hex" --close_position false >/dev/null
    }
    retry_leg leg_repay_debt_with_coll

    assert_hf_at_least hf_post_strategies "$macct" "$WAD"

    # repay_debt_with_collateral close_position=true (full-close branch). A
    # dedicated CAROL account (full XLM balance at this phase) takes a small,
    # single USDC debt, then sells XLM collateral that over-covers it. The branch
    # asserts borrow_positions is empty (#... CannotCloseWithRemainingDebt) then
    # withdraws ALL remaining collateral, so the account is fully closed.
    local rdwc_acct
    rdwc_acct=$(inv rdwc_close_supply "$CAROL" "$CONTROLLER" -- supply \
        --caller "$CAROL_ADDR" --account_id 0 --e_mode_category 0 \
        --assets "$(pay_vec "$XLM_SAC" 3000000000)" | tr -d '"') || return 1
    inv rdwc_close_borrow "$CAROL" "$CONTROLLER" -- borrow \
        --caller "$CAROL_ADDR" --account_id "$rdwc_acct" \
        --borrows "$(pay_vec "$USDC_SAC" 300000000)" >/dev/null
    leg_rdwc_close() {
        local hex
        hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" 2500000000) || return 1
        inv rdwc_close "$CAROL" "$CONTROLLER" -- repay_debt_with_collateral \
            --caller "$CAROL_ADDR" --account_id "$rdwc_acct" \
            --collateral_token "$XLM_SAC" --collateral_amount 2500000000 \
            --debt_token "$USDC_SAC" --swap "$hex" --close_position true >/dev/null
    }
    retry_leg leg_rdwc_close
    # Full close empties + deregisters the account.
    assert_bool_view rdwc_closed false account_exists --account_id "$rdwc_acct"

    # multiply SHORT: flash 300 XLM debt ($57 oracle), swap into USDC
    # collateral (~$40 at DEX rate) + 45 USDC initial payment — covers the
    # oracle/DEX gap so post-state LTV holds even at 5% slippage.
    local flash_xlm=3000000000 sacct=""
    leg_multiply_short() {
        local hex
        hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" "$flash_xlm") || return 1
        sacct=$(inv multiply_short "$ALICE" "$CONTROLLER" -- multiply \
            --caller "$ALICE_ADDR" --account_id 0 --e_mode_category 0 \
            --collateral_token "$USDC_SAC" --debt_to_flash_loan "$flash_xlm" \
            --debt_token "$XLM_SAC" --mode 3 --swap "$hex" \
            --initial_payment "[\"$USDC_SAC\",\"450000000\"]" | tr -d '"')
        [ -n "$sacct" ]
    }
    retry_leg leg_multiply_short || return 1
    save_state ALICE_SACCT "$sacct"
    assert_hf_at_least hf_short "$sacct" "$WAD"
}
