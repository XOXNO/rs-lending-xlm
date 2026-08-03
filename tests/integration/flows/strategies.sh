




flash_data_hex() {
    local mode="$1"
    jq -nc --argjson m "$mode" '{map:[{key:{symbol:"mode"},val:{u32:$m}}]}' \
        | stellar xdr encode --type ScVal | base64 -d | xxd -p | tr -d '\n'
}

flow_flash_loans() {
    phase flash_loans

    sac_transfer "$ALICE" "$USDC_SAC" "$ALICE_ADDR" "$FLASH_RECEIVER" 50000000 fund_flash_receiver

    inv flash_loan_success "$ALICE" "$CONTROLLER" -- flash_loan \
        --caller "$ALICE_ADDR" --asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" --amount 100000000 \
        --receiver "$FLASH_RECEIVER" --data "$(flash_data_hex 0)" >/dev/null











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
            --caller "$ALICE_ADDR" --asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" --amount 100000000 \
            --receiver "$FLASH_RECEIVER" --data "$(flash_data_hex $mode)"
    done
}






flow_strategies() {
    phase strategies
    local flash_usdc=300000000
    local swap_hex
    swap_hex=$(agg_route_hex "$USDC_SAC" "$XLM_SAC" "$flash_usdc") || return 1
    local macct
    macct=$(inv multiply_long "$ALICE" "$CONTROLLER" -- multiply \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --collateral "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" --debt_to_flash_loan "$flash_usdc" \
        --debt "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" --mode 2 --swap "$swap_hex" \
        --initial_payment "[$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC"),\"5000000000\"]" --convert_swap null | tr -d '"')
    save_state ALICE_MACCT "$macct"
    log "multiply account = $macct"
    assert_hf_at_least hf_multiply "$macct" "$WAD"




    local new_xlm_debt=1000000000
    swap_hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" "$new_xlm_debt") || return 1
    inv swap_debt "$ALICE" "$CONTROLLER" -- swap_debt \
        --caller "$ALICE_ADDR" --account_id "$macct" \
        --existing_debt "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" --amount "$new_xlm_debt" \
        --new_debt "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" --swap "$swap_hex" >/dev/null
    assert_borrow_at_least xlm_debt_post_swap "$macct" "$XLM_SAC" 500000000

    leg_swap_collateral() {
        local hex
        hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" 2000000000) || return 1
        inv swap_collateral "$ALICE" "$CONTROLLER" -- swap_collateral \
            --caller "$ALICE_ADDR" --account_id "$macct" \
            --current "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" --amount 2000000000 \
            --new "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" --swap "$hex" >/dev/null
    }
    retry_leg leg_swap_collateral







    inv supply_for_rdwc "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id "$macct" --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 10000000000)" >/dev/null
    inv borrow_for_rdwc "$ALICE" "$CONTROLLER" -- borrow \
        --caller "$ALICE_ADDR" --account_id "$macct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 550000000)" --to null >/dev/null
    leg_repay_debt_with_coll() {
        local hex
        hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" 5000000000) || return 1
        inv repay_debt_with_coll "$ALICE" "$CONTROLLER" -- repay_debt_with_collateral \
            --caller "$ALICE_ADDR" --account_id "$macct" \
            --collateral "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" --collateral_amount 5000000000 \
            --debt "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" --swap "$hex" --close_position false >/dev/null
    }
    retry_leg leg_repay_debt_with_coll

    assert_hf_at_least hf_post_strategies "$macct" "$WAD"










    local rdwc_acct
    rdwc_acct=$(inv rdwc_close_supply "$CAROL" "$CONTROLLER" -- supply \
        --caller "$CAROL_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 4000000000)" | tr -d '"') || return 1
    inv rdwc_close_borrow "$CAROL" "$CONTROLLER" -- borrow \
        --caller "$CAROL_ADDR" --account_id "$rdwc_acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 100000000)" --to null >/dev/null
    leg_rdwc_close() {
        local hex

        hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" 4000000000 0.10) || return 1
        inv rdwc_close "$CAROL" "$CONTROLLER" -- repay_debt_with_collateral \
            --caller "$CAROL_ADDR" --account_id "$rdwc_acct" \
            --collateral "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" --collateral_amount 4000000000 \
            --debt "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" --swap "$hex" --close_position true >/dev/null
    }
    retry_leg leg_rdwc_close

    assert_bool_view rdwc_closed false account_exists --account_id "$rdwc_acct"







    local flash_xlm=5000000000 sacct=""
    leg_multiply_short() {
        local hex
        hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" "$flash_xlm" 0.10) || return 1
        sacct=$(inv multiply_short "$ALICE" "$CONTROLLER" -- multiply \
            --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --collateral "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" --debt_to_flash_loan "$flash_xlm" \
            --debt "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" --mode 3 --swap "$hex" \
            --initial_payment "[$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC"),\"1000000000\"]" --convert_swap null | tr -d '"')
        [ -n "$sacct" ]
    }
    retry_leg leg_multiply_short || return 1
    save_state ALICE_SACCT "$sacct"
    assert_hf_at_least hf_short "$sacct" "$WAD"
}
