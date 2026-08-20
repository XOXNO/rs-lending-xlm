# Live testnet coverage for controller `flash_position`.
# The mock receiver does not mint: it must be pre-funded and pushes collateral.

FP_MODE_SUCCESS=0
FP_MODE_KEEP_FUNDS=1
FP_MODE_BELOW_MIN=2
FP_MODE_PANIC=3
# PositionMode::Multiply / Normal
FP_POSITION_MODE=1
FP_MODE_NORMAL=0
FP_DEBT_AMOUNT="${FP_DEBT_AMOUNT:-10000000}"
FP_COLLATERAL_AMOUNT="${FP_COLLATERAL_AMOUNT:-10000000000}"
FP_RECEIVER_FUND="${FP_RECEIVER_FUND:-15000000000}"
FP_USDC_SWAP="${FP_USDC_SWAP:-10000000000}"
FP_EXTEND_COLLATERAL="${FP_EXTEND_COLLATERAL:-10000000}"
FP_DUST_COLLATERAL=1
FP_SMALL_DEBT="${FP_SMALL_DEBT:-10000000}"

fp_set_plan() {
    local label="$1" mode="$2" amount="${3:-$FP_COLLATERAL_AMOUNT}"
    local extra="${4:-$XLM_SAC}" extra_amt="${5:-0}"
    local coll="${FP_PLAN_ASSET:-$XLM_SAC}"
    inv "$label" "$ALICE" "$FP_RECV" -- set_plan \
        --mode "$mode" --collateral "$coll" --amount "$amount" \
        --extra "$extra" --extra_amount "$extra_amt" \
        --spoke_id "${FP_PLAN_SPOKE:-$PRIMARY_SPOKE_ID}" >/dev/null
}

fp_xfail_pair() {
    local base="$1" pattern="$2"
    local prev="${FP_ACCOUNT_ID:-0}"
    FP_ACCOUNT_ID=0
    fp_run xfail "${base}_new" "$pattern" || true
    FP_ACCOUNT_ID="$ALICE_FP_ACCT"
    fp_run xfail "${base}_ex" "$pattern" || true
    FP_ACCOUNT_ID="$prev"
}

fp_restore_min_borrow() {
    local floor="${1:-5000000000000000000}"
    inv fp_restore_min_borrow "$ADMIN" "$CONTROLLER" -- set_min_borrow_collateral_usd \
        --floor_wad "$floor" >/dev/null
}

fp_collaterals() {
    pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" "${1:-$FP_COLLATERAL_AMOUNT}"
}

fp_collaterals_on() {
    pay_vec "$1" "$2" "$3"
}

fp_refunds() {
    jq -nc --arg a "$1" '[$a]'
}

fp_run() {
    local kind="$1" label="$2" pattern="$3"
    local signer="${FP_SIGNER:-$ALICE}"
    local caller="${FP_CALLER_ADDR:-$ALICE_ADDR}"
    local account_id="${FP_ACCOUNT_ID:-0}"
    local spoke="${FP_SPOKE:-$PRIMARY_SPOKE_ID}"
    local pos_mode="${FP_POS_MODE:-$FP_POSITION_MODE}"
    local debt="${FP_DEBT:-$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")}"
    local amount="${FP_AMOUNT:-$FP_DEBT_AMOUNT}"
    local receiver="${FP_RECV:-$FLASH_POSITION_RECEIVER}"
    local collaterals="${FP_COLS:-$(fp_collaterals)}"
    local refunds="${FP_REFUNDS:-[]}"
    local -a args=(
        -- flash_position
        --caller "$caller"
        --account_id "$account_id"
        --spoke_id "$spoke"
        --mode "$pos_mode"
        --debt "$debt"
        --amount "$amount"
        --receiver "$receiver"
        --data 00
        --collaterals "$collaterals"
        --refund_assets "$refunds"
    )
    case "$kind" in
        xfail) xfail "$label" "$pattern" "$signer" "$CONTROLLER" "${args[@]}" ;;
        inv) inv "$label" "$signer" "$CONTROLLER" "${args[@]}" >/dev/null ;;
        create) inv_create "$label" "$signer" "$CONTROLLER" "${args[@]}" ;;
        *) die fp_run "unknown kind $kind" ;;
    esac
}

fp_restore_xlm_listing() {
    inv fp_restore_xlm_listing "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$XLM_SAC" "$PRIMARY_SPOKE_ID" true true 7000 7500 1000)" >/dev/null
}

fp_restore_usdc_listing() {
    inv fp_restore_usdc_listing "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$USDC_SAC" "$PRIMARY_SPOKE_ID" true true 7500 8000 500)" >/dev/null
}

fp_restore_usdc_curve() {
    inv fp_restore_usdc_curve "$ADMIN" "$CONTROLLER" -- upgrade_liquidity_pool_params \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" \
        --params "$(market_params_json "$USDC_SAC" 7 | jq -c '{
            max_borrow_rate, base_borrow_rate, slope1, slope2, slope3,
            mid_utilization, optimal_utilization, max_utilization,
            reserve_factor, is_flashloanable, flashloan_fee
        }')" >/dev/null
}

fp_restore_position_limits() {
    inv fp_restore_position_limits "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits '{"max_supply_positions":5,"max_borrow_positions":5}' >/dev/null
}

fp_ensure_token() {
    local sac="$1" need="$2" label="$3"
    local have gap
    have=$(balance "$sac" "$FP_RECV")
    have=${have:-0}
    if _uint_ge "$have" "$need"; then
        record "$label" ok skip "" "" "" "" "" "receiver already has $have"
        return 0
    fi
    gap=$((need - have))
    if [ "$sac" = "$XLM_SAC" ]; then
        sac_transfer "$ALICE" "$XLM_SAC" "$ALICE_ADDR" "$FP_RECV" "$gap" "$label" || return 1
        return 0
    fi
    inv "${label}_borrow" "$ALICE" "$CONTROLLER" -- borrow \
        --caller "$ALICE_ADDR" --account_id "$ALICE_FP_ACCT" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$sac" "$gap")" \
        --to null >/dev/null || return 1
    sac_transfer "$ALICE" "$sac" "$ALICE_ADDR" "$FP_RECV" "$gap" "${label}_xfer" || return 1
}

flow_flash_position_markets() {
    phase real_markets
    local xlm_band
    xlm_band=$(reflector_band XLM) || {
        log "XLM live price unavailable; cannot calibrate sanity band"
        return 1
    }
    create_market XLM "$PRIMARY_HUB_ID" "$XLM_SAC" 7 \
        "$(oracle_cfg_reflector XLM $xlm_band)" \
        "$(asset_config_json 7000 7500 1000)" || return 1
    create_market USDC "$PRIMARY_HUB_ID" "$USDC_SAC" 7 \
        "$(oracle_cfg_reflector USDC 900000000000000000 1100000000000000000)" \
        "$(asset_config_json 7500 8000 500)" || return 1
}

flow_flash_position_fund() {
    phase funding
    [ -n "${FUNDED_USDC:-}" ] && return 0
    local line code issuer
    line=$(classic_line "$USDC_SAC")
    code="${line%%:*}"; issuer="${line##*:}"
    trustline "$ADMIN" "$code" "$issuer" || return 1
    swap_xlm_to "$ADMIN" "$ADMIN_ADDR" "$USDC_SAC" "$FP_USDC_SWAP" fund_swap_usdc || return 1
    local got
    got=$(balance "$USDC_SAC" "$ADMIN_ADDR")
    [ -z "$got" ] || [ "$got" -le 0 ] && { log "funding swap produced no USDC"; return 1; }
    log "admin USDC balance: $got"
    save_state FUNDED_USDC 1
}

flow_flash_position() {
    phase flash_position
    if [ -z "${FLASH_POSITION_RECEIVER:-}" ]; then
        log "FLASH_POSITION_RECEIVER unset; skipping live flash_position coverage"
        record flash_position_skipped ok skip "" "" "" "" "" "receiver wasm not deployed"
        return 0
    fi
    FP_RECV="$FLASH_POSITION_RECEIVER"

    if [ -n "${FP_BASELINE_DONE:-}" ] || [ -n "${ALICE_FP_ACCT:-}" ]; then
        log "baseline flash_position already recorded; skipping create-path"
        return 0
    fi

    sac_transfer "$ALICE" "$XLM_SAC" "$ALICE_ADDR" "$FLASH_POSITION_RECEIVER" \
        "$FP_RECEIVER_FUND" fund_flash_position_receiver || return 1
    local recv_xlm
    recv_xlm=$(balance "$XLM_SAC" "$FLASH_POSITION_RECEIVER")
    _uint_ge "${recv_xlm:-0}" "$FP_COLLATERAL_AMOUNT" \
        || { _assert_fail fund_flash_position_receiver "receiver XLM $recv_xlm want >= $FP_COLLATERAL_AMOUNT"; return 1; }

    FP_ACCOUNT_ID=0
    FP_COLS='[]'
    fp_run xfail flash_position_empty_collaterals 'Error\(Contract, #16\)' || true
    FP_COLS="$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 0)"
    fp_run xfail flash_position_zero_mins 'Error\(Contract, #503\)' || true
    FP_COLS="$(fp_collaterals)"
    FP_RECV="$ALICE_ADDR"
    fp_run xfail flash_position_eoa_receiver 'Error\(Contract, #412\)' || true
    FP_RECV="$FLASH_POSITION_RECEIVER"

    fp_set_plan fp_plan_panic "$FP_MODE_PANIC" || return 1
    fp_run xfail flash_position_panic 'Error\(Contract, #2\)|Trapped|CallbackPanic' || true

    fp_set_plan fp_plan_keep "$FP_MODE_KEEP_FUNDS" || return 1
    fp_run xfail flash_position_keep_funds 'Error\(Contract, #504\)' || true

    fp_set_plan fp_plan_below "$FP_MODE_BELOW_MIN" || return 1
    fp_run xfail flash_position_below_min 'Error\(Contract, #504\)' || true

    fp_set_plan fp_plan_success "$FP_MODE_SUCCESS" || return 1
    local rev_pre rev_post acct recv_usdc min_debt
    rev_pre=$(_view_pool_int fp_revenue_pre get_revenue \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    acct=$(fp_run create flash_position_success "" | tr -d '"') || return 1
    save_state ALICE_FP_ACCT "$acct"
    log "flash_position account = $acct"

    rev_post=$(_view_pool_int fp_revenue_post get_revenue \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    if [ "$rev_post" = "${rev_pre:-}" ]; then
        record flash_position_zero_fee ok flash_position "" "" "" "" "" "$rev_pre -> $rev_post"
    else
        _assert_fail flash_position_zero_fee \
            "pool revenue $rev_pre -> $rev_post; flash_position must not book a flash fee"
    fi

    assert_bool_view fp_account_exists true account_exists --account_id "$acct"
    assert_hf_at_least hf_flash_position "$acct" "$WAD"
    min_debt=$((FP_DEBT_AMOUNT - 1))
    [ "$min_debt" -lt 1 ] && min_debt=1
    assert_borrow_at_least debt_usdc_flash_position "$acct" "$USDC_SAC" "$min_debt"
    assert_int_view_positive coll_xlm_flash_position get_collateral_amount \
        --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")"

    recv_usdc=$(balance "$USDC_SAC" "$FLASH_POSITION_RECEIVER")
    if _uint_ge "${recv_usdc:-0}" "$FP_DEBT_AMOUNT"; then
        record flash_position_debt_stays_on_receiver ok flash_position "" "" "" "" "" \
            "receiver USDC $recv_usdc (not pulled back)"
    else
        _assert_fail flash_position_debt_stays_on_receiver \
            "receiver USDC $recv_usdc want >= $FP_DEBT_AMOUNT (debt tokens must stay on the receiver)"
    fi
    save_state FP_BASELINE_DONE 1
}

fp_deploy_matrix_receiver() {
    if [ -n "${FLASH_POSITION_RECEIVER_V2:-}" ]; then
        FP_RECV="$FLASH_POSITION_RECEIVER_V2"
        return 0
    fi
    local out_f="$LOG_DIR/deploy_flashposrecv_v2.out" err_f="$LOG_DIR/deploy_flashposrecv_v2.err"
    run_deploy "$out_f" "$err_f" -- stellar contract deploy \
        --wasm "$WASM_DIR/flash_position_receiver.wasm" \
        --source "$ADMIN" "${NET_ARGS[@]}"
    local recv txh
    recv=$(sanitize_output "$out_f")
    txh=$(extract_signing_hash "$err_f")
    is_contract_id "$recv" || die deploy_flash_position_receiver_v2 \
        "updated flash_position receiver deploy produced no id: $(tail_err_note "$err_f")"
    save_state FLASH_POSITION_RECEIVER_V2 "$recv"
    record deploy_flash_position_receiver_v2 ok deploy "$txh" "" "" "" "" "$recv"
    FP_RECV="$recv"
    log "flash_position receiver v2 = $recv"
}

# Existing-account success, spare-HF KeepFunds denial, dust-min success,
# and the reachable error surface on the live controller.
flow_flash_position_matrix() {
    phase flash_position_matrix
    [ -n "${ALICE_FP_ACCT:-}" ] || die flash_position_matrix "ALICE_FP_ACCT missing; run baseline first"
    [ -n "${BOB_ADDR:-}" ] || die flash_position_matrix "BOB wallet missing"

    fp_deploy_matrix_receiver || return 1
    local line
    line=$(classic_line "$USDC_SAC")
    trustline "$ALICE" "${line%%:*}" "${line##*:}" || return 1
    fp_ensure_token "$XLM_SAC" 50000000 fund_fp_matrix_xlm || return 1
    fp_ensure_token "$USDC_SAC" 30000000 fund_fp_matrix_usdc || return 1

    FP_SIGNER="$ALICE"
    FP_CALLER_ADDR="$ALICE_ADDR"
    FP_SPOKE="$PRIMARY_SPOKE_ID"
    FP_POS_MODE="$FP_POSITION_MODE"
    FP_DEBT="$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")"
    FP_AMOUNT="$FP_SMALL_DEBT"
    FP_REFUNDS='[]'
    FP_RECV="${FLASH_POSITION_RECEIVER_V2:-$FLASH_POSITION_RECEIVER}"

    local debt_before coll_before hf_before
    debt_before=$(_view_int fp_ext_debt_pre get_borrow_amount --account_id "$ALICE_FP_ACCT" \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    coll_before=$(_view_int fp_ext_coll_pre get_collateral_amount --account_id "$ALICE_FP_ACCT" \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")")
    hf_before=$(_view_int fp_ext_hf_pre get_health_factor --account_id "$ALICE_FP_ACCT")
    log "existing account $ALICE_FP_ACCT debt=$debt_before coll=$coll_before hf=$hf_before"

    # --- success on the already-open account (must push measured collateral) ---
    FP_ACCOUNT_ID="$ALICE_FP_ACCT"
    FP_COLS="$(fp_collaterals "$FP_EXTEND_COLLATERAL")"
    fp_set_plan fp_plan_extend "$FP_MODE_SUCCESS" "$FP_EXTEND_COLLATERAL" || return 1
    fp_run inv flash_position_extend_existing "" || return 1
    assert_hf_at_least hf_flash_position_extended "$ALICE_FP_ACCT" "$WAD"
    local debt_after
    debt_after=$(_view_int fp_ext_debt_post get_borrow_amount --account_id "$ALICE_FP_ACCT" \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    _uint_ge "$debt_after" "$debt_before" \
        || _assert_fail fp_ext_debt_grew "debt $debt_before -> $debt_after; extend must mint more"
    local coll_after
    coll_after=$(_view_int fp_ext_coll_post get_collateral_amount --account_id "$ALICE_FP_ACCT" \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")")
    _uint_ge "$coll_after" "$coll_before" \
        || _assert_fail fp_ext_coll_grew "collateral $coll_before -> $coll_after"

    # Spare HF does not skip the push: KeepFunds on a healthy existing account
    # is still #504. flash_position is not a free borrow-to-receiver.
    fp_set_plan fp_plan_keep_existing "$FP_MODE_KEEP_FUNDS" "$FP_EXTEND_COLLATERAL" || return 1
    FP_COLS="$(fp_collaterals "$FP_EXTEND_COLLATERAL")"
    fp_run xfail flash_position_keep_funds_existing 'Error\(Contract, #504\)' || true

    # Closest honest "callback returns almost nothing": dust min, existing
    # collateral covers the new debt at finalize.
    FP_COLS="$(fp_collaterals "$FP_DUST_COLLATERAL")"
    fp_set_plan fp_plan_dust_existing "$FP_MODE_SUCCESS" "$FP_DUST_COLLATERAL" || return 1
    fp_run inv flash_position_dust_min_existing "" || return 1
    assert_hf_at_least hf_flash_position_dust_existing "$ALICE_FP_ACCT" "$WAD"

    # Extra undeclared asset is not deposited; listed refund returns it to caller.
    local alice_usdc_pre alice_usdc_post extra_usdc=10000000
    alice_usdc_pre=$(balance "$USDC_SAC" "$ALICE_ADDR")
    FP_COLS="$(fp_collaterals "$FP_EXTEND_COLLATERAL")"
    FP_REFUNDS="$(fp_refunds "$USDC_SAC")"
    fp_set_plan fp_plan_refund_extra "$FP_MODE_SUCCESS" "$FP_EXTEND_COLLATERAL" \
        "$USDC_SAC" "$extra_usdc" || return 1
    fp_run inv flash_position_refund_undeclared "" || return 1
    alice_usdc_post=$(balance "$USDC_SAC" "$ALICE_ADDR")
    if _uint_ge "${alice_usdc_post:-0}" "${alice_usdc_pre:-0}"; then
        record flash_position_refund_observed ok flash_position "" "" "" "" "" \
            "alice USDC $alice_usdc_pre -> $alice_usdc_post"
    else
        _assert_fail flash_position_refund_observed \
            "alice USDC $alice_usdc_pre -> $alice_usdc_post; refund_assets should return extra"
    fi
    FP_REFUNDS='[]'

    # Push the wrong listed asset (USDC) while declaring XLM — measured XLM
    # delta is 0, so the min fails. The undeclared USDC is not credited.
    fp_set_plan fp_plan_wrong_asset "$FP_MODE_SUCCESS" 0 "$USDC_SAC" 10000000 || return 1
    FP_COLS="$(fp_collaterals "$FP_EXTEND_COLLATERAL")"
    fp_run xfail flash_position_wrong_asset_push 'Error\(Contract, #504\)' || true

    # --- new-account dust fails solvency / min-borrow floor ---
    FP_ACCOUNT_ID=0
    FP_COLS="$(fp_collaterals "$FP_DUST_COLLATERAL")"
    fp_set_plan fp_plan_dust_new "$FP_MODE_SUCCESS" "$FP_DUST_COLLATERAL" || return 1
    fp_run xfail flash_position_dust_new_unhealthy 'Error\(Contract, #100\)' || true

    # --- input / auth / listing errors (callback never runs) ---
    FP_ACCOUNT_ID="$ALICE_FP_ACCT"
    FP_COLS="$(fp_collaterals "$FP_EXTEND_COLLATERAL")"
    FP_AMOUNT=0
    fp_run xfail flash_position_zero_debt 'Error\(Contract, #14\)' || true
    FP_AMOUNT=-1
    fp_run xfail flash_position_negative_debt 'Error\(Contract, #14\)' || true
    FP_AMOUNT="$FP_SMALL_DEBT"

    FP_COLS="$(fp_collaterals_on 99 "$XLM_SAC" "$FP_EXTEND_COLLATERAL")"
    FP_DEBT="$(hub_key 99 "$USDC_SAC")"
    fp_run xfail flash_position_inactive_hub 'Error\(Contract, #43\)' || true
    FP_DEBT="$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")"

    FP_COLS="$(fp_collaterals_on "$PRIMARY_HUB_ID" "$EURC_SAC" "$FP_EXTEND_COLLATERAL")"
    fp_run xfail flash_position_unlisted_collateral 'Error\(Contract, #307\)' || true

    FP_COLS="$(jq -nc --argjson h "$PRIMARY_HUB_ID" --arg a "$XLM_SAC" \
        '[[{hub_id:$h,asset:$a},"1"],[{hub_id:$h,asset:$a},"1"]]')"
    fp_run xfail flash_position_duplicate_collateral 'Error\(Contract, #16\)' || true

    FP_COLS="$(fp_collaterals "$FP_EXTEND_COLLATERAL")"
    FP_REFUNDS="$(fp_refunds "$XLM_SAC")"
    fp_run xfail flash_position_refund_overlap 'Error\(Contract, #16\)' || true
    FP_REFUNDS='[]'

    FP_RECV="$CONTROLLER"
    fp_run xfail flash_position_controller_receiver 'Error\(Contract, #412\)' || true
    FP_RECV="$POOL"
    fp_run xfail flash_position_pool_receiver 'Error\(Contract, #412\)' || true
    FP_RECV="${FLASH_POSITION_RECEIVER_V2:-$FLASH_POSITION_RECEIVER}"

    FP_ACCOUNT_ID=99999
    fp_run xfail flash_position_missing_account 'Error\(Contract, #24\)' || true
    FP_ACCOUNT_ID="$ALICE_FP_ACCT"

    FP_SPOKE="$SECONDARY_SPOKE_ID"
    fp_run xfail flash_position_spoke_mismatch 'Error\(Contract, #310\)' || true
    FP_SPOKE="$PRIMARY_SPOKE_ID"

    FP_POS_MODE="$FP_MODE_NORMAL"
    fp_run xfail flash_position_mode_mismatch 'Error\(Contract, #25\)' || true
    FP_POS_MODE="$FP_POSITION_MODE"

    FP_SIGNER="$BOB"
    FP_CALLER_ADDR="$BOB_ADDR"
    fp_run xfail flash_position_not_owner 'Error\(Contract, #44\)' || true
    FP_SIGNER="$ALICE"
    FP_CALLER_ADDR="$ALICE_ADDR"

    inv fp_pause "$ADMIN" "$CONTROLLER" -- pause >/dev/null
    fp_run xfail flash_position_paused 'Error\(Contract, #1000\)' || true
    inv fp_unpause "$ADMIN" "$CONTROLLER" -- unpause >/dev/null

    inv fp_pause_xlm "$ADMIN" "$CONTROLLER" -- set_spoke_asset_flags \
        --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" \
        --paused true --frozen false --no_seize false >/dev/null
    fp_run xfail flash_position_collateral_paused 'Error\(Contract, #315\)' || true
    fp_restore_xlm_listing || return 1

    inv fp_pause_usdc_debt "$ADMIN" "$CONTROLLER" -- set_spoke_asset_flags \
        --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" \
        --paused true --frozen false --no_seize false >/dev/null
    fp_run xfail flash_position_debt_paused 'Error\(Contract, #315\)' || true
    fp_restore_usdc_listing || return 1

    inv fp_xlm_not_collateral "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$XLM_SAC" "$PRIMARY_SPOKE_ID" false true 7000 7500 1000)" >/dev/null
    fp_run xfail flash_position_not_collateral 'Error\(Contract, #104\)' || true
    fp_restore_xlm_listing || return 1

    inv fp_usdc_not_borrowable "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$USDC_SAC" "$PRIMARY_SPOKE_ID" true false 7500 8000 500)" >/dev/null
    fp_run xfail flash_position_not_borrowable 'Error\(Contract, #107\)' || true
    fp_restore_usdc_listing || return 1

    inv fp_limits_one_supply "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits '{"max_supply_positions":1,"max_borrow_positions":5}' >/dev/null
    FP_COLS="$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 1)"
    fp_run xfail flash_position_supply_position_limit 'Error\(Contract, #109\)' || true
    FP_COLS="$(fp_collaterals "$FP_EXTEND_COLLATERAL")"
    fp_restore_position_limits || return 1

    local supplied cap
    supplied=$(_view_pool_int fp_xlm_supplied get_supplied_amount \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")")
    cap=${supplied:-0}
    inv fp_xlm_supply_cap "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$XLM_SAC" "$PRIMARY_SPOKE_ID" true true 7000 7500 1000 "$cap")" >/dev/null
    fp_set_plan fp_plan_supply_cap "$FP_MODE_SUCCESS" "$FP_EXTEND_COLLATERAL" || true
    fp_run xfail flash_position_supply_cap 'Error\(Contract, #311\)' || true
    fp_restore_xlm_listing || return 1

    local borrowed
    borrowed=$(_view_pool_int fp_usdc_borrowed get_borrowed_amount \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    inv fp_usdc_borrow_cap "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$USDC_SAC" "$PRIMARY_SPOKE_ID" true true 7500 8000 500 \
            1000000000000000000 "${borrowed:-0}")" >/dev/null
    fp_run xfail flash_position_borrow_cap 'Error\(Contract, #312\)' || true
    fp_restore_usdc_listing || return 1

    inv fp_usdc_util_cap "$ADMIN" "$CONTROLLER" -- upgrade_liquidity_pool_params \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" \
        --params "$(jq -nc '{
            max_borrow_rate: "2000000000000000000000000000",
            base_borrow_rate: "10000000000000000000000000",
            slope1: "40000000000000000000000000",
            slope2: "100000000000000000000000000",
            slope3: "1500000000000000000000000000",
            mid_utilization: "100000000000000000000000",
            optimal_utilization: "200000000000000000000000",
            max_utilization: "300000000000000000000000",
            reserve_factor: 1000,
            is_flashloanable: true,
            flashloan_fee: 5
        }')" >/dev/null
    fp_run xfail flash_position_utilization 'Error\(Contract, #127\)' || true
    fp_restore_usdc_curve || return 1

    local cash borrowed_now try_liq
    cash=$(_view_pool_int fp_usdc_cash get_reserves --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    borrowed_now=$(_view_pool_int fp_usdc_borrowed_liq get_borrowed_amount \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    # Draw more than remaining cash so the pool's liquidation buffer / cash
    # check binds rather than the spoke borrow cap (restored above).
    try_liq=$(( ${cash:-0} + ${borrowed_now:-0} + 1 ))
    if [ "$try_liq" -gt 1 ]; then
        FP_AMOUNT="$try_liq"
        fp_run xfail flash_position_insufficient_liquidity 'Error\(Contract, #112\)' || true
        FP_AMOUNT="$FP_SMALL_DEBT"
    fi

    assert_hf_at_least hf_flash_position_matrix_end "$ALICE_FP_ACCT" "$WAD"
    assert_bool_view fp_account_still_exists true account_exists --account_id "$ALICE_FP_ACCT"
    save_state FP_MATRIX_DONE 1
    record flash_position_matrix ok flash_position "" "" "" "" "" "matrix complete"
}

# Remaining create-path and dual-path cases that the baseline/matrix did not
# exercise: same-asset open, spoke 0 / unknown / deprecated, $5-floor #126,
# two-collateral position limit on an empty account, borrow-position limit,
# frozen listings, is_flashloanable=false, and the error surface on account_id=0.
flow_flash_position_gaps() {
    phase flash_position_gaps
    [ -n "${ALICE_FP_ACCT:-}" ] || die flash_position_gaps "ALICE_FP_ACCT missing"
    [ -n "${BOB_ADDR:-}" ] || die flash_position_gaps "BOB wallet missing"
    if [ -n "${FP_GAPS_DONE:-}" ]; then
        log "flash_position gaps already recorded; skipping"
        return 0
    fi

    fp_deploy_matrix_receiver || return 1
    FP_RECV="${FLASH_POSITION_RECEIVER_V2:-$FLASH_POSITION_RECEIVER}"
    FP_SIGNER="$ALICE"
    FP_CALLER_ADDR="$ALICE_ADDR"
    FP_SPOKE="$PRIMARY_SPOKE_ID"
    FP_POS_MODE="$FP_POSITION_MODE"
    FP_DEBT="$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")"
    FP_AMOUNT="$FP_SMALL_DEBT"
    FP_REFUNDS='[]'
    FP_PLAN_ASSET="$XLM_SAC"
    FP_COLS="$(fp_collaterals "$FP_EXTEND_COLLATERAL")"

    local line
    line=$(classic_line "$USDC_SAC")
    trustline "$ALICE" "${line%%:*}" "${line##*:}" || true
    # Several account_id=0 successes each need ~200 XLM so HF clears on a
    # brand-new account. 1 XLM vs 1 USDC is not enough if XLM is cheap.
    local create_coll=2000000000
    fp_ensure_token "$XLM_SAC" 12000000000 fund_fp_gaps_xlm || return 1
    fp_ensure_token "$USDC_SAC" 150000000 fund_fp_gaps_usdc || return 1
    # Resume after a mid-flow RPC fail can leave listings/caps/limits tight.
    fp_restore_xlm_listing || true
    fp_restore_usdc_listing || true
    fp_restore_usdc_curve || true
    fp_restore_position_limits || true
    fp_restore_min_borrow 5000000000000000000 || true

    local min_floor
    min_floor=$(_view_int fp_min_borrow_pre get_min_borrow_collateral_usd)
    [ -n "$min_floor" ] || min_floor=5000000000000000000

    # --- create-only: spoke identity ---
    FP_ACCOUNT_ID=0
    FP_SPOKE=0
    fp_run xfail flash_position_spoke_zero_new 'Error\(Contract, #300\)' || true
    FP_SPOKE=99
    fp_run xfail flash_position_unknown_spoke_new 'Error\(Contract, #300\)' || true
    FP_SPOKE="$SECONDARY_SPOKE_ID"
    fp_run xfail flash_position_empty_spoke_new 'Error\(Contract, #307\)' || true
    local dep_spoke
    dep_spoke=$(inv fp_add_deprecated_spoke "$ADMIN" "$CONTROLLER" -- add_spoke | tr -d '"[:space:]')
    inv fp_deprecate_spoke "$ADMIN" "$CONTROLLER" -- remove_spoke --id "$dep_spoke" >/dev/null
    FP_SPOKE="$dep_spoke"
    fp_run xfail flash_position_deprecated_spoke_new 'Error\(Contract, #301\)' || true
    FP_SPOKE="$PRIMARY_SPOKE_ID"

    # Existing + unknown spoke is a mismatch, not a missing-spoke load.
    FP_ACCOUNT_ID="$ALICE_FP_ACCT"
    FP_SPOKE=99
    fp_run xfail flash_position_unknown_spoke_ex 'Error\(Contract, #310\)|#300' || true
    FP_SPOKE="$PRIMARY_SPOKE_ID"

    # --- dual-path input / listing / auth (callback never required) ---
    FP_COLS='[]'
    fp_xfail_pair flash_position_empty_collaterals_gap 'Error\(Contract, #16\)'
    FP_COLS="$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 0)"
    fp_xfail_pair flash_position_zero_mins_gap 'Error\(Contract, #503\)'
    FP_COLS="$(fp_collaterals "$FP_EXTEND_COLLATERAL")"
    FP_AMOUNT=0
    fp_xfail_pair flash_position_zero_debt_gap 'Error\(Contract, #14\)'
    FP_AMOUNT=-1
    fp_xfail_pair flash_position_negative_debt_gap 'Error\(Contract, #14\)'
    FP_AMOUNT="$FP_SMALL_DEBT"

    FP_COLS="$(fp_collaterals_on 99 "$XLM_SAC" "$FP_EXTEND_COLLATERAL")"
    FP_DEBT="$(hub_key 99 "$USDC_SAC")"
    fp_xfail_pair flash_position_inactive_hub_gap 'Error\(Contract, #43\)'
    FP_DEBT="$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")"
    FP_COLS="$(fp_collaterals_on "$PRIMARY_HUB_ID" "$EURC_SAC" "$FP_EXTEND_COLLATERAL")"
    fp_xfail_pair flash_position_unlisted_collateral_gap 'Error\(Contract, #307\)'
    FP_COLS="$(jq -nc --argjson h "$PRIMARY_HUB_ID" --arg a "$XLM_SAC" \
        '[[{hub_id:$h,asset:$a},"1"],[{hub_id:$h,asset:$a},"1"]]')"
    fp_xfail_pair flash_position_duplicate_collateral_gap 'Error\(Contract, #16\)'
    FP_COLS="$(fp_collaterals "$FP_EXTEND_COLLATERAL")"
    FP_REFUNDS="$(fp_refunds "$XLM_SAC")"
    fp_xfail_pair flash_position_refund_overlap_gap 'Error\(Contract, #16\)'
    FP_REFUNDS='[]'

    local saved_recv="$FP_RECV"
    FP_RECV="$ALICE_ADDR"
    fp_xfail_pair flash_position_eoa_receiver_gap 'Error\(Contract, #412\)'
    FP_RECV="$CONTROLLER"
    fp_xfail_pair flash_position_controller_receiver_gap 'Error\(Contract, #412\)'
    FP_RECV="$POOL"
    fp_xfail_pair flash_position_pool_receiver_gap 'Error\(Contract, #412\)'
    FP_RECV="$saved_recv"

    fp_set_plan fp_plan_keep_gap "$FP_MODE_KEEP_FUNDS" "$FP_EXTEND_COLLATERAL" || true
    fp_xfail_pair flash_position_keep_funds_gap 'Error\(Contract, #504\)'
    fp_set_plan fp_plan_below_gap "$FP_MODE_BELOW_MIN" "$FP_EXTEND_COLLATERAL" || true
    fp_xfail_pair flash_position_below_min_gap 'Error\(Contract, #504\)'
    fp_set_plan fp_plan_panic_gap "$FP_MODE_PANIC" "$FP_EXTEND_COLLATERAL" || true
    fp_xfail_pair flash_position_panic_gap 'Error\(Contract, #2\)|Trapped|CallbackPanic'
    fp_set_plan fp_plan_wrong_gap "$FP_MODE_SUCCESS" 0 "$USDC_SAC" 10000000 || true
    fp_xfail_pair flash_position_wrong_asset_gap 'Error\(Contract, #504\)'

    inv fp_pause_gaps "$ADMIN" "$CONTROLLER" -- pause >/dev/null
    fp_xfail_pair flash_position_paused_gap 'Error\(Contract, #1000\)'
    inv fp_unpause_gaps "$ADMIN" "$CONTROLLER" -- unpause >/dev/null

    inv fp_pause_xlm_gaps "$ADMIN" "$CONTROLLER" -- set_spoke_asset_flags \
        --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" \
        --paused true --frozen false --no_seize false >/dev/null
    fp_xfail_pair flash_position_collateral_paused_gap 'Error\(Contract, #315\)'
    fp_restore_xlm_listing || return 1

    inv fp_freeze_xlm_gaps "$ADMIN" "$CONTROLLER" -- set_spoke_asset_flags \
        --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" \
        --paused false --frozen true --no_seize false >/dev/null
    fp_xfail_pair flash_position_collateral_frozen_gap 'Error\(Contract, #316\)'
    fp_restore_xlm_listing || return 1

    inv fp_pause_usdc_gaps "$ADMIN" "$CONTROLLER" -- set_spoke_asset_flags \
        --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" \
        --paused true --frozen false --no_seize false >/dev/null
    fp_xfail_pair flash_position_debt_paused_gap 'Error\(Contract, #315\)'
    fp_restore_usdc_listing || return 1

    inv fp_xlm_not_coll_gaps "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$XLM_SAC" "$PRIMARY_SPOKE_ID" false true 7000 7500 1000)" >/dev/null
    fp_xfail_pair flash_position_not_collateral_gap 'Error\(Contract, #104\)'
    fp_restore_xlm_listing || return 1

    inv fp_usdc_not_borr_gaps "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$USDC_SAC" "$PRIMARY_SPOKE_ID" true false 7500 8000 500)" >/dev/null
    fp_xfail_pair flash_position_not_borrowable_gap 'Error\(Contract, #107\)'
    fp_restore_usdc_listing || return 1

    # Empty account + two new collaterals vs max_supply=1.
    inv fp_limits_one_supply_gaps "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits '{"max_supply_positions":1,"max_borrow_positions":5}' >/dev/null
    FP_COLS="$(jq -nc --argjson h "$PRIMARY_HUB_ID" --arg x "$XLM_SAC" --arg u "$USDC_SAC" \
        '[[{hub_id:$h,asset:$x},"1"],[{hub_id:$h,asset:$u},"1"]]')"
    FP_ACCOUNT_ID=0
    # List length is checked before unique-position count: 2 declared
    # collaterals with max_supply_positions=1 is InvalidPayments (#16), not #109.
    fp_run xfail flash_position_two_collaterals_limit_new 'Error\(Contract, #16\)' || true
    # Existing already has XLM supply; declaring USDC as a second supply also #109.
    FP_ACCOUNT_ID="$ALICE_FP_ACCT"
    FP_COLS="$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 1)"
    fp_run xfail flash_position_second_supply_limit_ex 'Error\(Contract, #109\)' || true
    FP_COLS="$(fp_collaterals "$FP_EXTEND_COLLATERAL")"
    fp_restore_position_limits || true
    fp_restore_position_limits || return 1

    # Existing already has USDC debt; a second debt asset is a new borrow position.
    inv fp_limits_one_borrow_gaps "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits '{"max_supply_positions":5,"max_borrow_positions":1}' >/dev/null
    FP_DEBT="$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")"
    FP_ACCOUNT_ID="$ALICE_FP_ACCT"
    fp_run xfail flash_position_second_borrow_limit_ex 'Error\(Contract, #109\)' || true
    FP_DEBT="$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")"
    fp_restore_position_limits || return 1

    local supplied cap borrowed
    supplied=$(_view_pool_int fp_xlm_supplied_gaps get_supplied_amount \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")")
    cap=${supplied:-0}
    inv fp_xlm_supply_cap_gaps "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$XLM_SAC" "$PRIMARY_SPOKE_ID" true true 7000 7500 1000 "$cap")" >/dev/null
    fp_set_plan fp_plan_supply_cap_gaps "$FP_MODE_SUCCESS" "$FP_EXTEND_COLLATERAL" || true
    fp_xfail_pair flash_position_supply_cap_gap 'Error\(Contract, #311\)'
    fp_restore_xlm_listing || return 1

    borrowed=$(_view_pool_int fp_usdc_borrowed_gaps get_borrowed_amount \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    inv fp_usdc_borrow_cap_gaps "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$USDC_SAC" "$PRIMARY_SPOKE_ID" true true 7500 8000 500 \
            1000000000000000000 "${borrowed:-0}")" >/dev/null
    fp_xfail_pair flash_position_borrow_cap_gap 'Error\(Contract, #312\)'
    fp_restore_usdc_listing || return 1

    inv fp_usdc_util_cap_gaps "$ADMIN" "$CONTROLLER" -- upgrade_liquidity_pool_params \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" \
        --params "$(jq -nc '{
            max_borrow_rate: "2000000000000000000000000000",
            base_borrow_rate: "10000000000000000000000000",
            slope1: "40000000000000000000000000",
            slope2: "100000000000000000000000000",
            slope3: "1500000000000000000000000000",
            mid_utilization: "100000000000000000000000",
            optimal_utilization: "200000000000000000000000",
            max_utilization: "300000000000000000000000",
            reserve_factor: 1000,
            is_flashloanable: true,
            flashloan_fee: 5
        }')" >/dev/null
    fp_xfail_pair flash_position_utilization_gap 'Error\(Contract, #127\)'
    fp_restore_usdc_curve || return 1

    local cash borrowed_now try_liq
    cash=$(_view_pool_int fp_usdc_cash_gaps get_reserves \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    borrowed_now=$(_view_pool_int fp_usdc_borrowed_liq_gaps get_borrowed_amount \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    try_liq=$(( ${cash:-0} + ${borrowed_now:-0} + 1 ))
    if [ "$try_liq" -gt 1 ]; then
        FP_AMOUNT="$try_liq"
        fp_xfail_pair flash_position_insufficient_liquidity_gap 'Error\(Contract, #112\)'
        FP_AMOUNT="$FP_SMALL_DEBT"
    fi

    # Raise the USD floor so a 1-stroop USDC mint that is LTV-healthy still
    # fails #126 (checked after InsufficientCollateral).
    inv fp_min_borrow_high "$ADMIN" "$CONTROLLER" -- set_min_borrow_collateral_usd \
        --floor_wad 1000000000000000000000000000000000 >/dev/null
    FP_AMOUNT=1
    FP_COLS="$(fp_collaterals "$create_coll")"
    fp_set_plan fp_plan_min_borrow "$FP_MODE_SUCCESS" "$create_coll" || true
    fp_xfail_pair flash_position_min_borrow_gap 'Error\(Contract, #126\)'
    fp_restore_min_borrow "$min_floor" || return 1
    FP_AMOUNT="$FP_SMALL_DEBT"
    FP_COLS="$(fp_collaterals "$FP_EXTEND_COLLATERAL")"

    # --- create successes that were missing ---
    FP_ACCOUNT_ID=0
    FP_POS_MODE="$FP_MODE_NORMAL"
    FP_PLAN_ASSET="$USDC_SAC"
    FP_DEBT="$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")"
    FP_AMOUNT=10000000
    # USDC LTV is 75%: 5 USDC → $3.75 LTV-gated < $5 min-borrow floor (#126).
    # 10 USDC clears the default floor on a brand-new account.
    FP_COLS="$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 100000000)"
    fp_set_plan fp_plan_same_asset_new "$FP_MODE_SUCCESS" 100000000 || return 1
    local same_acct
    same_acct=$(fp_run create flash_position_same_asset_new "" | tr -d '"') || return 1
    save_state ALICE_FP_SAME_ACCT "$same_acct"
    assert_hf_at_least hf_same_asset_new "$same_acct" "$WAD"
    assert_borrow_at_least debt_same_asset_new "$same_acct" "$USDC_SAC" 9000000
    assert_int_view_positive coll_same_asset_new get_collateral_amount \
        --account_id "$same_acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")"

    # Existing Multiply account: add USDC supply while minting more USDC debt.
    FP_ACCOUNT_ID="$ALICE_FP_ACCT"
    FP_POS_MODE="$FP_POSITION_MODE"
    FP_COLS="$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 20000000)"
    fp_set_plan fp_plan_same_asset_ex "$FP_MODE_SUCCESS" 20000000 || return 1
    fp_run inv flash_position_same_asset_ex "" || return 1
    assert_hf_at_least hf_same_asset_ex "$ALICE_FP_ACCT" "$WAD"
    FP_PLAN_ASSET="$XLM_SAC"
    FP_COLS="$(fp_collaterals "$FP_EXTEND_COLLATERAL")"

    # Undeclared refund on a brand-new account.
    FP_ACCOUNT_ID=0
    FP_POS_MODE="$FP_POSITION_MODE"
    FP_PLAN_ASSET="$XLM_SAC"
    FP_COLS="$(fp_collaterals "$create_coll")"
    FP_REFUNDS="$(fp_refunds "$USDC_SAC")"
    local alice_usdc_pre alice_usdc_post
    alice_usdc_pre=$(balance "$USDC_SAC" "$ALICE_ADDR")
    fp_set_plan fp_plan_refund_new "$FP_MODE_SUCCESS" "$create_coll" \
        "$USDC_SAC" 10000000 || return 1
    local refund_acct
    refund_acct=$(fp_run create flash_position_refund_new "" | tr -d '"') || return 1
    save_state ALICE_FP_REFUND_ACCT "$refund_acct"
    alice_usdc_post=$(balance "$USDC_SAC" "$ALICE_ADDR")
    if _uint_ge "${alice_usdc_post:-0}" "${alice_usdc_pre:-0}"; then
        record flash_position_refund_new_observed ok flash_position "" "" "" "" "" \
            "alice USDC $alice_usdc_pre -> $alice_usdc_post"
    else
        _assert_fail flash_position_refund_new_observed \
            "alice USDC $alice_usdc_pre -> $alice_usdc_post"
    fi
    FP_REFUNDS='[]'
    assert_hf_at_least hf_refund_new "$refund_acct" "$WAD"

    # is_flashloanable=false must not block flash_position (it is not pool flash_loan).
    inv fp_usdc_no_flashloan "$ADMIN" "$CONTROLLER" -- upgrade_liquidity_pool_params \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" \
        --params "$(market_params_json "$USDC_SAC" 7 | jq -c '{
            max_borrow_rate, base_borrow_rate, slope1, slope2, slope3,
            mid_utilization, optimal_utilization, max_utilization,
            reserve_factor, is_flashloanable: false, flashloan_fee
        }')" >/dev/null
    FP_ACCOUNT_ID=0
    FP_COLS="$(fp_collaterals "$create_coll")"
    fp_set_plan fp_plan_noflash_new "$FP_MODE_SUCCESS" "$create_coll" || return 1
    local noflash_acct
    noflash_acct=$(fp_run create flash_position_no_flashloanable_new "" | tr -d '"') || return 1
    save_state ALICE_FP_NOFLASH_ACCT "$noflash_acct"
    assert_hf_at_least hf_noflash_new "$noflash_acct" "$WAD"
    FP_ACCOUNT_ID="$ALICE_FP_ACCT"
    FP_COLS="$(fp_collaterals "$FP_EXTEND_COLLATERAL")"
    fp_set_plan fp_plan_noflash_ex "$FP_MODE_SUCCESS" "$FP_EXTEND_COLLATERAL" || return 1
    fp_run inv flash_position_no_flashloanable_ex "" || return 1
    assert_hf_at_least hf_noflash_ex "$ALICE_FP_ACCT" "$WAD"
    fp_restore_usdc_curve || return 1

    # Empty account + max_supply=1 + a single collateral is allowed.
    inv fp_limits_one_supply_ok "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits '{"max_supply_positions":1,"max_borrow_positions":5}' >/dev/null
    FP_ACCOUNT_ID=0
    FP_COLS="$(fp_collaterals "$create_coll")"
    fp_set_plan fp_plan_one_supply_ok "$FP_MODE_SUCCESS" "$create_coll" || return 1
    local onesup_acct
    onesup_acct=$(fp_run create flash_position_one_supply_limit_ok_new "" | tr -d '"') || return 1
    save_state ALICE_FP_ONESUP_ACCT "$onesup_acct"
    assert_hf_at_least hf_one_supply_ok_new "$onesup_acct" "$WAD"
    fp_restore_position_limits || return 1

    assert_hf_at_least hf_flash_position_gaps_end "$ALICE_FP_ACCT" "$WAD"
    assert_bool_view fp_gaps_account_exists true account_exists --account_id "$ALICE_FP_ACCT"
    save_state FP_GAPS_DONE 1
    record flash_position_gaps ok flash_position "" "" "" "" "" "gaps complete"
}

# Live malicious receiver: every nested controller entry from the callback
# must fail (#400 or host InvalidAction). Uses a freshly deployed wasm so
# reentry modes exist on-chain.
flow_flash_position_malicious() {
    phase flash_position_malicious
    [ -n "${ALICE_FP_ACCT:-}" ] || die flash_position_malicious "ALICE_FP_ACCT missing"
    if [ -n "${FP_MALICIOUS_DONE:-}" ]; then
        log "malicious receiver coverage already recorded; skipping"
        return 0
    fi

    local out_f="$LOG_DIR/deploy_flashposrecv_mal.out" err_f="$LOG_DIR/deploy_flashposrecv_mal.err"
    run_deploy "$out_f" "$err_f" -- stellar contract deploy \
        --wasm "$WASM_DIR/flash_position_receiver.wasm" \
        --source "$ADMIN" "${NET_ARGS[@]}"
    local recv txh
    recv=$(sanitize_output "$out_f")
    txh=$(extract_signing_hash "$err_f")
    is_contract_id "$recv" || die deploy_flash_position_malicious \
        "malicious receiver deploy produced no id: $(tail_err_note "$err_f")"
    save_state FLASH_POSITION_RECEIVER_MAL "$recv"
    record deploy_flash_position_malicious ok deploy "$txh" "" "" "" "" "$recv"
    FP_RECV="$recv"

    FP_SIGNER="$ALICE"
    FP_CALLER_ADDR="$ALICE_ADDR"
    FP_ACCOUNT_ID="$ALICE_FP_ACCT"
    FP_SPOKE="$PRIMARY_SPOKE_ID"
    FP_POS_MODE="$FP_POSITION_MODE"
    FP_DEBT="$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")"
    FP_AMOUNT="$FP_SMALL_DEBT"
    FP_REFUNDS='[]'
    FP_PLAN_ASSET="$XLM_SAC"
    FP_COLS="$(fp_collaterals "$FP_EXTEND_COLLATERAL")"

    local re_pattern='Error\(Contract, #400\)|InvalidAction|re-entry|Error\(Contract, #44\)'
    local mode name
    for mode in 4 5 6 7 8 9; do
        case $mode in
            4) name=reenter_supply ;;
            5) name=reenter_borrow ;;
            6) name=reenter_withdraw ;;
            7) name=reenter_repay ;;
            8) name=reenter_flash_loan ;;
            9) name=reenter_flash_position ;;
        esac
        fp_set_plan "fp_plan_mal_$name" "$mode" "$FP_EXTEND_COLLATERAL" || return 1
        fp_run xfail "flash_position_malicious_$name" "$re_pattern" || true
    done

    save_state FP_MALICIOUS_DONE 1
    record flash_position_malicious ok flash_position "" "" "" "" "" "malicious receiver coverage complete"
}
