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
    view liq1_estimate "$CONTROLLER" -- get_liquidation_estimate --seize_mode "$(seize_transfer)" \
        --account_id "$acct" --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((100 * LIQ_UNIT)))" >/dev/null || return 1
    view liq1_avail "$CONTROLLER" -- get_liquidation_collateral --account_id "$acct" >/dev/null || return 1
    liq_transfer_leg partial "$acct" "$((100 * LIQ_UNIT))" || return 1
    assert_borrow_decreased liq1_debt_post_partial "$acct" "$SAC_LIQB" "$((600 * LIQ_UNIT))" || return 1
    view liq1_estimate_close "$CONTROLLER" -- get_liquidation_estimate --seize_mode "$(seize_transfer)" \
        --account_id "$acct" --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((600 * LIQ_UNIT)))" >/dev/null || return 1
    liq_transfer_leg full "$acct" "$((600 * LIQ_UNIT))" || return 1
    assert_borrow_at_most liq1_debt_cleared "$acct" "$SAC_LIQB" 0 || return 1
    save_state LIQ1_ACCT "$acct"
}

# Snapshot before submission; evaluate with indexes committed by that transaction.
liq_transfer_leg() {
    local leg="$1" acct="$2" offered="$3" positions supply debt expected paid gross fee
    local payer cash recipient holdings revenue collateral remaining
    positions=$(view "liq1_${leg}_positions" "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    payer=$(balance "$SAC_LIQB" "$CAROL_ADDR") || return 1
    cash=$(balance "$SAC_LIQB" "$POOL") || return 1
    recipient=$(balance "$SAC_LIQA" "$CAROL_ADDR") || return 1
    holdings=$(balance "$SAC_LIQA" "$POOL") || return 1
    revenue=$(_view_pool_int "liq1_${leg}_revenue" get_revenue --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQA")") || return 1
    collateral=$(_view_int "liq1_${leg}_collateral" get_collateral_amount --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQA")") || return 1
    inv "liq1_liquidate_$leg" "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_transfer)" \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" "$offered")" >/dev/null || return 1
    supply=$(view "liq1_${leg}_supply_index" "$POOL" -- get_sync_data --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQA")") || return 1
    debt=$(view "liq1_${leg}_debt_index" "$POOL" -- get_sync_data --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQB")") || return 1
    expected=$(liquidation_reference "$positions" "$supply" "$debt" "$offered") || { _assert_fail "liq1_${leg}_reference" "fixture outside independently bounded liquidation curve"; return 1; }
    read -r paid gross fee <<<"$expected"
    local positions_after
    positions_after=$(view "liq1_${leg}_positions_after" "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    assert_liquidation_debt_burn "liq1_${leg}_debt_burn" "$positions" "$positions_after" "$debt" "$paid" || return 1
    assert_delta "liq1_${leg}_payer" "$payer" "$(balance "$SAC_LIQB" "$CAROL_ADDR")" "-$paid" || return 1
    assert_delta "liq1_${leg}_cash" "$cash" "$(balance "$SAC_LIQB" "$POOL")" "$paid" || return 1
    assert_delta "liq1_${leg}_recipient" "$recipient" "$(balance "$SAC_LIQA" "$CAROL_ADDR")" "$(raw_sub "$gross" "$fee")" || return 1
    assert_delta "liq1_${leg}_holdings" "$holdings" "$(balance "$SAC_LIQA" "$POOL")" "$(raw_sub "$fee" "$gross")" || return 1
    assert_delta "liq1_${leg}_fee" "$revenue" "$(_view_pool_int "liq1_${leg}_revenue_after" get_revenue --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQA")")" "$fee" || return 1
    remaining=$(_view_int "liq1_${leg}_collateral_after" get_collateral_amount --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQA")") || return 1
    assert_delta "liq1_${leg}_seizure" "$collateral" "$remaining" "-$gross"
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

# Share-credit liquidation (`SeizeMode::Credit`, ADR-0019) through both
# admission paths. `liquidate` returns the receiving account id: 0 for
# Transfer, a new id for Credit(0), the same id for Credit(<existing>).
flow_liq_credit() {
    phase liq_credit
    local acct recv recv2
    acct=$(inv_create liqcr_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQG" $((1000 * LIQ_UNIT)))" | tr -d '"') || return 1
    inv liqcr_borrow "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((600 * LIQ_UNIT)))" --to null >/dev/null || return 1
    dual_px "$SAC_LIQG" LIQG $((WAD / 10 * 7)) liqcr_crash || return 1
    assert_hf_below_wad liqcr_hf "$acct" || return 1
    recv=$(liq_credit_leg new "$acct" 0 "$((100 * LIQ_UNIT))") || return 1
    recv2=$(liq_credit_leg existing "$acct" "$recv" "$((50 * LIQ_UNIT))") || return 1
    save_state LIQCR_ACCT "$acct"
    save_state LIQCR_RECV "$recv2"
}

# Credit reclassifies exact RAY shares; it never transfers collateral tokens.
liq_credit_leg() {
    local leg="$1" acct="$2" target="$3" paid="$4" recv key
    local before after receiving_before='[{},{}]' receiving_after pool_before pool_after debt attrs
    local payer cash recipient holdings
    key=$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQG")
    before=$(view "liqcr_${leg}_before" "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    if [ "$target" != 0 ]; then
        receiving_before=$(view "liqcr_${leg}_receiver_before" "$CONTROLLER" -- get_account_positions --account_id "$target") || return 1
    fi
    pool_before=$(view "liqcr_${leg}_pool_before" "$POOL" -- get_sync_data --hub_asset "$key") || return 1
    payer=$(balance "$SAC_LIQB" "$CAROL_ADDR") || return 1
    cash=$(balance "$SAC_LIQB" "$POOL") || return 1
    recipient=$(balance "$SAC_LIQG" "$CAROL_ADDR") || return 1
    holdings=$(balance "$SAC_LIQG" "$POOL") || return 1
    recv=$(inv "liqcr_liquidate_$leg" "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_credit "$target")" \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" "$paid")" | tr -d '\"[:space:]') || return 1
    if ! _is_uint "$recv" || [ "$recv" = 0 ] || [ "$recv" = "$acct" ] || { [ "$target" != 0 ] && [ "$recv" != "$target" ]; }; then
        _assert_fail "liqcr_${leg}_account_id" "invalid receiver $recv for Credit($target)"; return 1
    fi
    assert_view_eq_at "$POSITION_NFT" "liqcr_${leg}_owner_read" "$CAROL_ADDR" owner_of --token_id "$recv" || return 1
    record "liqcr_${leg}_owner" ok assert "" "" "" "" "" "receiver NFT belongs to liquidator"
    attrs=$(view "liqcr_${leg}_attributes" "$CONTROLLER" -- get_account_attributes --account_id "$recv") || return 1
    after=$(view "liqcr_${leg}_after" "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    receiving_after=$(view "liqcr_${leg}_receiver_after" "$CONTROLLER" -- get_account_positions --account_id "$recv") || return 1
    pool_after=$(view "liqcr_${leg}_pool_after" "$POOL" -- get_sync_data --hub_asset "$key") || return 1
    debt=$(view "liqcr_${leg}_debt_index" "$POOL" -- get_sync_data --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQB")") || return 1
    assert_liquidation_credit "liqcr_${leg}_shares" "$before" "$after" "$receiving_before" "$receiving_after" "$pool_before" "$pool_after" "$attrs" "$PRIMARY_SPOKE_ID" "$debt" "$paid" || return 1
    assert_liquidation_debt_burn "liqcr_${leg}_debt_burn" "$before" "$after" "$debt" "$paid" || return 1
    assert_delta "liqcr_${leg}_payer" "$payer" "$(balance "$SAC_LIQB" "$CAROL_ADDR")" "-$paid" || return 1
    assert_delta "liqcr_${leg}_cash" "$cash" "$(balance "$SAC_LIQB" "$POOL")" "$paid" || return 1
    assert_delta "liqcr_${leg}_recipient" "$recipient" "$(balance "$SAC_LIQG" "$CAROL_ADDR")" 0 || return 1
    assert_delta "liqcr_${leg}_holdings" "$holdings" "$(balance "$SAC_LIQG" "$POOL")" 0 || return 1
    printf '%s\n' "$recv"
}

# Share-credit receiver rules (ADR-0019): each call below must revert.
# Runs after flow_liq_spoke, which creates the SPOKE_ID that the
# spoke-mismatch case needs.
flow_liq_credit_rejections() {
    phase liq_credit_reject
    [ -n "${LIQCR_ACCT:-}" ] || { _assert_fail liqcr_prerequisite "missing LIQCR_ACCT"; return 1; }

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

    # A liquidator-owned account in a different spoke: an account's spoke sets
    # the risk configuration of every position it holds.
    if [ -n "${SPOKE_ID:-}" ]; then
        local carol_other
        carol_other=$(inv_create liqcr_carol_other_spoke "$CAROL" "$CONTROLLER" -- supply \
            --caller "$CAROL_ADDR" --account_id 0 --spoke_id "$SPOKE_ID" \
            --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQE" $((10 * LIQ_UNIT)))" | tr -d '"') || return 1
        xfail liqcr_reject_spoke_mismatch 'Error\(Contract, #310\)' "$CAROL" "$CONTROLLER" -- liquidate \
            --seize_mode "$(seize_credit "$carol_other")" \
            --liquidator "$CAROL_ADDR" --account_id "$LIQCR_ACCT" \
            --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((10 * LIQ_UNIT)))"
    fi
}

# `set_spoke_asset_flags`, `set_spoke_liquidation_curve` and `get_spoke_usage`.
#
# Order matters. `set_spoke_asset_flags` only tightens flags and no later flow
# relaxes LIQG, so the seizure halt lasts for the rest of the run: this flow
# runs after flow_liq_credit is done with LIQG. The curve change targets
# SPOKE_ID (from flow_liq_spoke), so primary-spoke liquidations do not change.
flow_spoke_flags_and_curve() {
    phase spoke_flags
    [ -n "${LIQCR_ACCT:-}" ] || { _assert_fail spoke_flags_prerequisite "missing LIQCR_ACCT"; return 1; }

    local liqg_key
    liqg_key=$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQG")

    # Spoke usage holds per-spoke cap consumption; the credit flow moved supply.
    # It returns a SpokeUsageRaw struct, so this checks a field, not an int.
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

    # A liquidatable account cannot lose collateral that has no_seize set.
    xfail sf_seizure_halted 'Error\(Contract, #318\)' "$CAROL" "$CONTROLLER" -- liquidate \
        --seize_mode "$(seize_transfer)" \
        --liquidator "$CAROL_ADDR" --account_id "$LIQCR_ACCT" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQB" $((10 * LIQ_UNIT)))"
}

# `force_socialize_bad_debt` (owner-only) and `recapitalize` (permissionless).
#
# Runs after flow_clean_bad_debt, which tests the permissionless cleanup on its
# own account; this flow builds a separate position and socializes it through
# the owner override.
recapitalization_reference() {
    local basis="$1"
    if [ "$#" -ge 4 ]; then basis=$(pool_accrual_at_committed_index "$1" "$4") || return 1; fi
    python3 - "$basis" "$2" "$3" "${4:-}" <<'PYRECAP'
import json,sys
state={k:int(v) for k,v in json.loads(sys.argv[1])['state'].items()};amount=int(sys.argv[2]);unit=10**(27-int(sys.argv[3]));R=10**27
assert amount>=0
claim=state['supplied']*state['supply_index']//(R*unit)
debt=(state['borrowed']*state['borrow_index']+R*unit-1)//(R*unit)
applied=min(amount,max(0,claim-state['cash']-debt))
if sys.argv[4]:
    actual={k:int(v) for k,v in json.loads(sys.argv[4])['state'].items()}
    state['cash']+=applied
    assert actual==state, 'recapitalization changed accounting beyond accrued state and applied cash'
print(applied)
PYRECAP
}

bad_debt_accounting() {
    local debt_accrued collateral_accrued
    debt_accrued=$(pool_accrual_at_committed_index "$2" "$3") || return 1
    collateral_accrued=$(pool_accrual_at_committed_index "$4" "$5") || return 1
    python3 - "$1" "$debt_accrued" "$3" "$collateral_accrued" "$5" <<'PYBAD'
import json,sys
positions,debt_before,debt_after,coll_before,coll_after=map(json.loads,sys.argv[1:6])
db,da,cb,ca=[{k:int(v) for k,v in x['state'].items()} for x in [debt_before,debt_after,coll_before,coll_after]]
R=10**27; half=lambda n,d:(n+d//2)//d; ceil=lambda n,d:(n+d-1)//d
assert len(positions[0])==len(positions[1])==1
s=int(next(iter(positions[0].values()))['scaled_amount']);d=int(next(iter(positions[1].values()))['scaled_amount'])
assert 0<d<=db['borrowed'] and s>0
value=half(db['supplied']*db['supply_index'],R)
loss=ceil(d*db['borrow_index'],R)
if value:
    factor=max(0,value-loss)*R//value
    db['supply_index']=max(db['supply_index']*factor//R,R//1000)
db['borrowed']-=d
assert da==db, 'bad debt did not apply exact loss index and debt share burn'
assert cb['supply_index']==R and cb['borrowed']==0
cb['revenue']+=s
assert ca==cb, 'collateral seizure changed accounting beyond exact revenue reclassification'
PYBAD
}

flow_force_socialize_and_recap() {
    phase force_socialize
    # flow_clean_bad_debt leaves LIQC at 15% of WAD, where the borrow below
    # fails with #100 InsufficientCollateral. Restore the price first.
    dual_px "$SAC_LIQC" LIQC "$WAD" fs_restore

    local acct
    acct=$(inv_create fs_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQC" $((30 * LIQ_UNIT)))" | tr -d '"') || return 1
    inv fs_borrow "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQD" $((12 * LIQ_UNIT)))" --to null >/dev/null

    dual_px "$SAC_LIQC" LIQC $((WAD / 100 * 30)) fs_crash || return 1
    # $9 collateral exceeds the $5 permissionless dust threshold; ~$12 debt.
    xfail fs_permissionless_denied 'Error\(Contract, #114\)' "$BOB" "$CONTROLLER" -- clean_bad_debt --caller "$BOB_ADDR" --account_id "$acct" || return 1
    xfail fs_owner_required "Missing signing key for account $ADMIN_ADDR" "$BOB" "$CONTROLLER" -- force_socialize_bad_debt --account_id "$acct" || return 1
    local cash_before collateral_before
    cash_before=$(balance "$SAC_LIQD" "$POOL") || return 1
    collateral_before=$(balance "$SAC_LIQC" "$POOL") || return 1

    local fs_positions fs_debt_before fs_coll_before fs_debt_after fs_coll_after
    fs_positions=$(view fs_positions_before "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    fs_debt_before=$(view fs_debt_before "$POOL" -- get_sync_data --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQD")") || return 1
    fs_coll_before=$(view fs_coll_before "$POOL" -- get_sync_data --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQC")") || return 1
    inv fs_force_socialize "$ADMIN" "$CONTROLLER" -- force_socialize_bad_debt \
        --account_id "$acct" >/dev/null
    fs_debt_after=$(view fs_debt_after "$POOL" -- get_sync_data --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQD")") || return 1
    fs_coll_after=$(view fs_coll_after "$POOL" -- get_sync_data --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQC")") || return 1
    bad_debt_accounting "$fs_positions" "$fs_debt_before" "$fs_debt_after" "$fs_coll_before" "$fs_coll_after" || { _assert_fail fs_socialized_accounting 'incorrect debt/share/revenue accounting'; return 1; }
    record fs_socialized_accounting ok assert "" "" "" "" "" 'exact committed-index accrual, debt burn, socialized loss index and collateral reclassification'
    assert_borrow_at_most fs_debt_cleared "$acct" "$SAC_LIQD" 0 || return 1
    assert_bool_view fs_account_closed false account_exists --account_id "$acct" || return 1
    xfail fs_nft_burned 'Error\(Contract, #200\)' "$BOB" "$POSITION_NFT" -- owner_of --token_id "$acct" || return 1
    assert_delta fs_no_debt_cash "$cash_before" "$(balance "$SAC_LIQD" "$POOL")" 0 || return 1
    assert_delta fs_no_collateral_cash "$collateral_before" "$(balance "$SAC_LIQC" "$POOL")" 0 || return 1

    # Validate the actual applied amount and excess refund, even when the
    # socialization already removed all backing shortfall (credited == 0).
    local expected recap_after payer_pre pool_pre credited amount=$((100 * LIQ_UNIT))
    payer_pre=$(balance "$SAC_LIQD" "$CAROL_ADDR") || return 1
    pool_pre=$(balance "$SAC_LIQD" "$POOL") || return 1
    credited=$(inv fs_recapitalize "$CAROL" "$CONTROLLER" -- recapitalize \
        --payer "$CAROL_ADDR" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQD")" \
        --amount "$amount" | tr -d '\"[:space:]') || return 1
    recap_after=$(view fs_recap_state "$POOL" -- get_sync_data --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQD")") || return 1
    expected=$(recapitalization_reference "$fs_debt_after" "$amount" 7 "$recap_after") \
        || { _assert_fail fs_recap_amount 'recapitalization changed accrued share/debt books'; return 1; }
    assert_raw_within fs_recap_amount "$credited" "$expected" 0 || return 1
    assert_delta fs_recap_cash "$(jq -r '.state.cash' <<<"$fs_debt_after")" "$(jq -r '.state.cash' <<<"$recap_after")" "$expected" || return 1
    [ "$(recapitalization_reference "$recap_after" "$amount" 7)" = 0 ] || { _assert_fail fs_recap_shortfall 'backing shortfall remains after excess recap'; return 1; }
    record fs_recap_shortfall ok assert "" "" "" "" "" 'independent backing shortfall zero after recap'

    assert_delta fs_recap_refund "$payer_pre" "$(balance "$SAC_LIQD" "$CAROL_ADDR")" "$(raw_sub 0 "$credited")" || return 1
    assert_delta fs_recap_backing "$pool_pre" "$(balance "$SAC_LIQD" "$POOL")" "$credited"

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

# GH-23. `remove_spoke` has no usage check and deprecation is one-way, so a
# spoke can hold live positions forever. Liquidation stays open there for a
# liquidator with no account in that spoke: `Credit(0)` creates the receiver
# inside the deprecated spoke, while supply into a new account there fails.
# Runs last in the liquidation block, so its LIQE/LIQF price moves do not
# affect earlier flows. Teardown drains the two accounts it leaves; that needs
# no active spoke.
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

    # CAROL owns no account in this spoke, so Credit(0) creates the receiver
    # in the deprecated spoke.
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

# Same SACs, separate hub books. The secondary borrower stays healthy while
# a primary-hub liquidation burns only the victim's shares and spoke usage.
flow_liq_multi_hub() {
    phase liq_multi_hub
    [ "$PRIMARY_HUB_ID" != "$SECONDARY_HUB_ID" ] || { _assert_fail lmh_hubs 'fixture requires distinct hubs'; return 1; }
    local code var sac hub role acct other result key before after supply debt expected paid gross fee
    for code in LIQH LIQI; do
        var="SAC_$code"
        issue_sac "$var" "$code" || return 1
        sac="${!var}"
        for role in "$BOB" "$CAROL"; do
            trustline "$role" "$code" "$ADMIN_ADDR" || return 1
        done
        mint_to "$sac" "$code" "$BOB_ADDR" $((10000 * LIQ_UNIT)) || return 1
        mint_to "$sac" "$code" "$CAROL_ADDR" $((10000 * LIQ_UNIT)) || return 1
        dual_px "$sac" "$code" "$WAD" "lmh_price_$code" || return 1
        for hub in "$PRIMARY_HUB_ID" "$SECONDARY_HUB_ID"; do
            create_market "${code}_H${hub}" "$hub" "$sac" 7 "$(oracle_cfg_mock_dual "$sac" "$code")" "$(asset_config_json 7000 7500 800)" || return 1
        done
    done
    for hub in "$PRIMARY_HUB_ID" "$SECONDARY_HUB_ID"; do
        inv "lmh_seed_$hub" "$CAROL" "$CONTROLLER" -- supply --caller "$CAROL_ADDR" \
            --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --assets "$(pay_vec "$hub" "$SAC_LIQI" $((2000 * LIQ_UNIT)))" >/dev/null || return 1
    done
    acct=$(inv_create lmh_supply "$BOB" "$CONTROLLER" -- supply --caller "$BOB_ADDR" \
        --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQH" $((1000 * LIQ_UNIT)))" | tr -d '"[:space:]') || return 1
    other=$(inv_create lmh_other_supply "$BOB" "$CONTROLLER" -- supply --caller "$BOB_ADDR" \
        --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$SECONDARY_HUB_ID" "$SAC_LIQH" $((1000 * LIQ_UNIT)))" | tr -d '"[:space:]') || return 1
    inv lmh_borrow "$BOB" "$CONTROLLER" -- borrow --caller "$BOB_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQI" $((600 * LIQ_UNIT)))" --to null >/dev/null || return 1
    inv lmh_other_borrow "$BOB" "$CONTROLLER" -- borrow --caller "$BOB_ADDR" --account_id "$other" \
        --borrows "$(pay_vec "$SECONDARY_HUB_ID" "$SAC_LIQI" $((100 * LIQ_UNIT)))" --to null >/dev/null || return 1
    dual_px "$SAC_LIQH" LIQH $((WAD / 10 * 7)) lmh_crash || return 1
    assert_can_liquidated lmh_victim_liquidatable "$acct" true || return 1
    assert_can_liquidated lmh_other_healthy "$other" false || return 1

    local isolated_before=() isolated_after=() usage_before=() usage_after=()
    local payer cash recipient holdings controller_debt controller_collateral revenue
    before=$(view lmh_positions_before "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    isolated_before+=("$(view lmh_other_positions_before "$CONTROLLER" -- get_account_positions --account_id "$other")") || return 1
    for sac in "$SAC_LIQH" "$SAC_LIQI"; do
        key=$(hub_key "$SECONDARY_HUB_ID" "$sac")
        isolated_before+=("$(view "lmh_other_book_before_$sac" "$POOL" -- get_sync_data --hub_asset "$key")") || return 1
        isolated_before+=("$(view "lmh_other_usage_before_$sac" "$CONTROLLER" -- get_spoke_usage --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$key")") || return 1
        usage_before+=("$(view "lmh_usage_before_$sac" "$CONTROLLER" -- get_spoke_usage --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$sac")")") || return 1
    done
    payer=$(balance "$SAC_LIQI" "$CAROL_ADDR") || return 1
    cash=$(balance "$SAC_LIQI" "$POOL") || return 1
    recipient=$(balance "$SAC_LIQH" "$CAROL_ADDR") || return 1
    holdings=$(balance "$SAC_LIQH" "$POOL") || return 1
    controller_debt=$(balance "$SAC_LIQI" "$CONTROLLER") || return 1
    controller_collateral=$(balance "$SAC_LIQH" "$CONTROLLER") || return 1
    revenue=$(_view_pool_int lmh_revenue_before get_revenue --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQH")") || return 1
    result=$(inv lmh_liquidate "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_transfer)" \
        --liquidator "$CAROL_ADDR" --account_id "$acct" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_LIQI" $((100 * LIQ_UNIT)))" | tr -d '"[:space:]') || return 1
    assert_raw_within lmh_transfer_result "$result" 0 0 || return 1
    after=$(view lmh_positions_after "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    supply=$(view lmh_supply_index "$POOL" -- get_sync_data --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQH")") || return 1
    debt=$(view lmh_debt_index "$POOL" -- get_sync_data --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQI")") || return 1
    expected=$(liquidation_reference "$before" "$supply" "$debt" "$((100 * LIQ_UNIT))") || { _assert_fail lmh_reference 'fixture outside independent liquidation curve'; return 1; }
    read -r paid gross fee <<<"$expected"
    assert_liquidation_debt_burn lmh_debt_burn "$before" "$after" "$debt" "$paid" || return 1
    assert_delta lmh_payer "$payer" "$(balance "$SAC_LIQI" "$CAROL_ADDR")" "-$paid" || return 1
    assert_delta lmh_pool_debt "$cash" "$(balance "$SAC_LIQI" "$POOL")" "$paid" || return 1
    assert_delta lmh_recipient "$recipient" "$(balance "$SAC_LIQH" "$CAROL_ADDR")" "$(raw_sub "$gross" "$fee")" || return 1
    assert_delta lmh_pool_collateral "$holdings" "$(balance "$SAC_LIQH" "$POOL")" "$(raw_sub "$fee" "$gross")" || return 1
    assert_delta lmh_controller_debt "$controller_debt" "$(balance "$SAC_LIQI" "$CONTROLLER")" 0 || return 1
    assert_delta lmh_controller_collateral "$controller_collateral" "$(balance "$SAC_LIQH" "$CONTROLLER")" 0 || return 1
    assert_delta lmh_fee "$revenue" "$(_view_pool_int lmh_revenue_after get_revenue --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$SAC_LIQH")")" "$fee" || return 1
    isolated_after+=("$(view lmh_other_positions_after "$CONTROLLER" -- get_account_positions --account_id "$other")") || return 1
    for sac in "$SAC_LIQH" "$SAC_LIQI"; do
        key=$(hub_key "$SECONDARY_HUB_ID" "$sac")
        isolated_after+=("$(view "lmh_other_book_after_$sac" "$POOL" -- get_sync_data --hub_asset "$key")") || return 1
        isolated_after+=("$(view "lmh_other_usage_after_$sac" "$CONTROLLER" -- get_spoke_usage --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$key")") || return 1
        usage_after+=("$(view "lmh_usage_after_$sac" "$CONTROLLER" -- get_spoke_usage --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$sac")")") || return 1
    done
    if ! python3 - "${isolated_before[@]}" "${isolated_after[@]}" "$SECONDARY_HUB_ID" "$SAC_LIQH" "$SAC_LIQI" <<'PYLMHISOLATION'
import json,sys
before=list(map(json.loads,sys.argv[1:6])); after=list(map(json.loads,sys.argv[6:11]))
assert before==after, 'secondary positions, market books or spoke usage changed'
for positions,asset in zip(before[0],sys.argv[12:14]):
    assert len(positions)==1
    key=json.loads(next(iter(positions)))
    assert key==dict(hub_id=int(sys.argv[11]),asset=asset), 'secondary fixture on wrong hub'
    assert int(next(iter(positions.values()))['scaled_amount'])>0
PYLMHISOLATION
    then
        _assert_fail lmh_isolation 'secondary hub positions, full pool state or spoke usage differ'; return 1
    fi
    record lmh_isolation ok assert "" "" "" "" "" 'secondary hub positions, books and spoke usage exactly unchanged'
    if ! python3 - "$before" "$after" "${usage_before[@]}" "${usage_after[@]}" "$gross" "$PRIMARY_HUB_ID" "$SAC_LIQH" "$SAC_LIQI" <<'PYLMHUSAGE'
import json,sys
before,after,coll_pre,debt_pre,coll_post,debt_post=map(json.loads,sys.argv[1:7])
for old,new,asset in zip(before,after,sys.argv[9:11]):
    assert len(old)==len(new)==1 and set(old)==set(new), 'victim market changed'
    assert json.loads(next(iter(old)))==dict(hub_id=int(sys.argv[8]),asset=asset)
amount=lambda positions:int(next(iter(positions.values()))['scaled_amount'])
seized=amount(before[0])-amount(after[0]); repaid=amount(before[1])-amount(after[1])
assert seized==int(sys.argv[7])*10**20 and repaid>0, 'incorrect primary collateral share burn'
assert int(coll_pre['supplied_scaled_ray'])-int(coll_post['supplied_scaled_ray'])==seized
assert coll_pre['borrowed_scaled_ray']==coll_post['borrowed_scaled_ray']
assert int(debt_pre['borrowed_scaled_ray'])-int(debt_post['borrowed_scaled_ray'])==repaid
assert debt_pre['supplied_scaled_ray']==debt_post['supplied_scaled_ray']
PYLMHUSAGE
    then
        _assert_fail lmh_usage 'primary collateral burn or exact spoke usage differs'; return 1
    fi
    record lmh_usage ok assert "" "" "" "" "" 'primary spoke usage matches exact victim collateral and debt share burns'
    for acct in "$acct" "$other"; do
        assert_view_eq_at "$POSITION_NFT" "lmh_owner_$acct" "$BOB_ADDR" owner_of --token_id "$acct" || return 1
        result=$(view "lmh_attributes_$acct" "$CONTROLLER" -- get_account_attributes --account_id "$acct") || return 1
        jq -e --argjson spoke "$PRIMARY_SPOKE_ID" '.spoke_id == $spoke and .mode == 0' <<<"$result" >/dev/null || { _assert_fail lmh_owner_spoke 'account spoke or mode changed'; return 1; }
    done
    record lmh_owner_spoke ok assert "" "" "" "" "" 'both surviving NFTs remain borrower-owned Normal accounts in the original spoke'
}
