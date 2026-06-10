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

    local mode name
    for mode in 1 2 3 4 5; do
        case $mode in
            1) name=no_repay ;;
            2) name=under_repay ;;
            3) name=reenter_pool ;;
            4) name=panic ;;
            5) name=reenter_supply ;;
        esac
        xfail "flash_loan_$name" 'Error' "$ALICE" "$CONTROLLER" -- flash_loan \
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
    view hf_multiply "$CONTROLLER" -- health_factor --account_id "$macct" >/dev/null

    # swap_debt: convert part of the USDC debt into XLM debt (forward quote on
    # the NEW debt; both remaining USDC debt and new XLM debt stay above the
    # $10 dust floor).
    local new_xlm_debt=1000000000   # 100 XLM ≈ $19 oracle
    swap_hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" "$new_xlm_debt") || return 1
    inv swap_debt "$ALICE" "$CONTROLLER" -- swap_debt \
        --caller "$ALICE_ADDR" --account_id "$macct" \
        --existing_debt_token "$USDC_SAC" --amount "$new_xlm_debt" \
        --new_debt_token "$XLM_SAC" --swap "$swap_hex" >/dev/null

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

    # repay_debt_with_collateral: sell 5 USDC collateral toward the XLM debt
    # (sized so the remaining XLM debt stays above the $10 floor).
    leg_repay_debt_with_coll() {
        local hex
        hex=$(agg_route_hex "$USDC_SAC" "$XLM_SAC" 50000000) || return 1
        inv repay_debt_with_coll "$ALICE" "$CONTROLLER" -- repay_debt_with_collateral \
            --caller "$ALICE_ADDR" --account_id "$macct" \
            --collateral_token "$USDC_SAC" --collateral_amount 50000000 \
            --debt_token "$XLM_SAC" --swap "$hex" --close_position false >/dev/null
    }
    retry_leg leg_repay_debt_with_coll

    view hf_post_strategies "$CONTROLLER" -- health_factor --account_id "$macct" >/dev/null

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
    view hf_short "$CONTROLLER" -- health_factor --account_id "$sacct" >/dev/null
}
