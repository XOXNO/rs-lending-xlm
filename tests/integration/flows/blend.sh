# Live Blend TestnetV2 migrate_from_blend coverage.
# Request types (Blend v2): 0 supply, 1 withdraw, 2 supply-collateral,
# 3 withdraw-collateral, 4 borrow, 5 repay.
# Observed TestnetV2 XLM: c_factor/l_factor 0.90, rates scaled by 1e12.

blend_pool_id() {
    jq -r '.pools[0].address // empty' "$REPO_ROOT/configs/$NETWORK/blend.json"
}

blend_addr_json() {
    jq -nc --arg a "$1" '[$a]'
}

blend_addr_dup_json() {
    jq -nc --arg a "$1" '[$a,$a]'
}

blend_debt_json() {
    jq -nc --arg a "$1" --arg c "$2" '[[$a,$c]]'
}

blend_positions() {
    local label="$1" addr="$2"
    view "$label" "$BLEND_POOL" -- get_positions --address "$addr"
}

blend_maps_empty() {
    echo "$1" | jq -e \
        '((.collateral // {}) == {}) and ((.liabilities // {}) == {}) and ((.supply // {}) == {})' \
        >/dev/null
}

blend_map_sum() {
    echo "$1" | jq -r --arg k "$2" '[.[$k] // {} | to_entries[] | .value | tonumber] | add // 0'
}

blend_has_map() {
    local s
    s=$(blend_map_sum "$1" "$2")
    [ -n "$s" ] && [ "$s" != "0" ]
}

blend_restore_xlm() {
    inv "${1:-blend_restore_xlm}" "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$XLM_SAC" "$PRIMARY_SPOKE_ID" true true 7000 7500 1000)" >/dev/null
}

blend_restore_min_borrow() {
    inv "${1:-blend_restore_min_borrow}" "$ADMIN" "$CONTROLLER" -- set_min_borrow_collateral_usd \
        --floor_wad "${2:-0}" >/dev/null
}

blend_ctrl_xlm() {
    balance "$XLM_SAC" "$CONTROLLER"
}

blend_assert_ctrl_xlm_clean() {
    local label="$1" before="${2:-0}"
    local after
    after=$(blend_ctrl_xlm)
    _is_uint "${after:-}" || after=0
    _is_uint "${before:-}" || before=0
    if _uint_le "$after" "$before"; then
        record "$label" ok balance "" "" "" "" "" "controller XLM $before -> $after"
        return 0
    fi
    local delta=$((after - before))
    if [ "$delta" -le 1000 ]; then
        record "$label" ok balance "" "" "" "" "" "controller XLM dust +$delta"
        return 0
    fi
    _assert_fail "$label" "controller retained $delta XLM stroops ($before -> $after)"
}

blend_seed() {
    local label="$1" wallet="$2" addr="$3" coll="$4" supply="$5" debt="$6"
    local req
    req=$(jq -nc --arg xlm "$XLM_SAC" --argjson c "$coll" --argjson s "$supply" --argjson d "$debt" '
        []
        + (if $c > 0 then [{request_type:2, address:$xlm, amount:($c|tostring)}] else [] end)
        + (if $s > 0 then [{request_type:0, address:$xlm, amount:($s|tostring)}] else [] end)
        + (if $d > 0 then [{request_type:4, address:$xlm, amount:($d|tostring)}] else [] end)
    ')
    if [ "$req" = "[]" ]; then
        log "blend seed $label skipped: all amounts zero"
        return 1
    fi
    if inv "$label" "$wallet" "$BLEND_POOL" -- submit \
        --from "$addr" --spender "$addr" --to "$addr" --requests "$req" >/dev/null; then
        return 0
    fi
    if [ "$debt" -gt 0 ]; then
        log "blend seed $label with debt failed; retrying without borrow"
        req=$(jq -nc --arg xlm "$XLM_SAC" --argjson c "$coll" --argjson s "$supply" '
            []
            + (if $c > 0 then [{request_type:2, address:$xlm, amount:($c|tostring)}] else [] end)
            + (if $s > 0 then [{request_type:0, address:$xlm, amount:($s|tostring)}] else [] end)
        ')
        if [ "$req" = "[]" ]; then
            return 1
        fi
        inv "${label}_nodebt" "$wallet" "$BLEND_POOL" -- submit \
            --from "$addr" --spender "$addr" --to "$addr" --requests "$req" >/dev/null || return 1
        return 2
    fi
    return 1
}

# Prints the new/existing account id. Uses inv_create so a transient RPC
# success that did not persist the account is retried.
blend_migrate() {
    local label="$1" wallet="$2" addr="$3" account_id="$4"
    local coll_json="$5" supply_json="$6" debt_json="$7"
    inv_create "$label" "$wallet" "$CONTROLLER" -- migrate_from_blend \
        --caller "$addr" --account_id "$account_id" --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$coll_json" \
        --supply_assets "$supply_json" \
        --debt_caps "$debt_json" | tr -d '"'
}

# Same call without inv_create: for empty-balance migrates that mint then
# cleanup the account, and for existing-account merges where the id is known.
blend_migrate_inv() {
    local label="$1" wallet="$2" addr="$3" account_id="$4"
    local coll_json="$5" supply_json="$6" debt_json="$7"
    inv "$label" "$wallet" "$CONTROLLER" -- migrate_from_blend \
        --caller "$addr" --account_id "$account_id" --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$coll_json" \
        --supply_assets "$supply_json" \
        --debt_caps "$debt_json" | tr -d '"'
}

flow_blend_hub_liquidity() {
    phase blend_hub_liquidity
    if [ -n "${SEEDED:-}" ]; then
        log "hub already seeded (acct=${ADMIN_ACCT:-n/a})"
        return 0
    fi
    local acct
    acct=$(inv_create seed_xlm_hub "$ADMIN" "$CONTROLLER" -- supply \
        --caller "$ADMIN_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 20000000000)" | tr -d '"') || return 1
    save_state ADMIN_ACCT "$acct"
    save_state SEEDED 1
}

flow_blend_allowlist() {
    phase blend_allowlist
    BLEND_POOL="$(blend_pool_id)"
    [ -n "$BLEND_POOL" ] && [ "$BLEND_POOL" != "null" ] \
        || die blend_pool "no pools[0].address in configs/$NETWORK/blend.json"
    save_state BLEND_POOL "$BLEND_POOL"
    log "blend pool = $BLEND_POOL"

    view blend_pool_initial "$CONTROLLER" -- is_blend_pool_approved --pool "$BLEND_POOL" >/dev/null
    inv blend_pool_approve "$ADMIN" "$CONTROLLER" -- approve_blend_pool --pool "$BLEND_POOL" >/dev/null
    assert_bool_view blend_pool_true true is_blend_pool_approved --pool "$BLEND_POOL"
    inv blend_pool_revoke "$ADMIN" "$CONTROLLER" -- revoke_blend_pool --pool "$BLEND_POOL" >/dev/null
    assert_bool_view blend_pool_false false is_blend_pool_approved --pool "$BLEND_POOL"
    inv blend_pool_reapprove "$ADMIN" "$CONTROLLER" -- approve_blend_pool --pool "$BLEND_POOL" >/dev/null
    assert_bool_view blend_pool_reapproved true is_blend_pool_approved --pool "$BLEND_POOL"
}

flow_blend_rejects() {
    phase blend_rejects
    local empty='[]'
    local xlm_coll xlm_debt
    xlm_coll=$(blend_addr_json "$XLM_SAC")
    xlm_debt=$(blend_debt_json "$XLM_SAC" 1)

    xfail blend_empty_params 'Error\(Contract, #16\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$empty" --supply_assets "$empty" --debt_caps "$empty"

    xfail blend_duplicate_debt 'Error\(Contract, #7\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$empty" --supply_assets "$empty" \
        --debt_caps "$(jq -nc --arg a "$XLM_SAC" '[[$a,"1"],[$a,"1"]]')"

    xfail blend_zero_debt_cap 'Error\(Contract, #14\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$empty" --supply_assets "$empty" \
        --debt_caps "$(blend_debt_json "$XLM_SAC" 0)"

    inv blend_pool_revoke_for_unapproved "$ADMIN" "$CONTROLLER" -- revoke_blend_pool \
        --pool "$BLEND_POOL" >/dev/null
    xfail blend_unapproved 'Error\(Contract, #42\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"
    inv blend_pool_reapprove_after_unapproved "$ADMIN" "$CONTROLLER" -- approve_blend_pool \
        --pool "$BLEND_POOL" >/dev/null

    xfail blend_unlisted_collateral 'Error\(Contract, #216\)|#307|#6|#1' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$(blend_addr_json "$BLEND_POOL")" \
        --supply_assets "$empty" --debt_caps "$empty"

    xfail blend_unlisted_debt 'Error\(Contract, #216\)|#307|#6|#1|#107' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$empty" --supply_assets "$empty" \
        --debt_caps "$(blend_debt_json "$BLEND_POOL" 1)"

    xfail blend_missing_account 'Error\(Contract, #24\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 999999 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    xfail blend_spoke_zero 'Error\(Contract, #300\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id 0 \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    xfail blend_spoke_unknown 'Error\(Contract, #300\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id 999 \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    xfail blend_hub_unknown 'Error\(Contract, #43\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id 99 --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    xfail blend_spoke_no_xlm 'Error\(Contract, #307\)|#216' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$SECONDARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    xfail blend_caller_mismatch 'Error\(Auth|Host|Authentication|InvalidAction|#5|#44\)' \
        "$BOB" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    inv blend_pause "$ADMIN" "$CONTROLLER" -- pause >/dev/null
    xfail blend_paused 'Error\(Contract, #1000\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"
    inv blend_unpause "$ADMIN" "$CONTROLLER" -- unpause >/dev/null

    inv blend_pause_xlm "$ADMIN" "$CONTROLLER" -- set_spoke_asset_flags \
        --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" \
        --paused true --frozen false --no_seize false >/dev/null
    xfail blend_collateral_paused 'Error\(Contract, #315\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty" \
        || { blend_restore_xlm blend_restore_xlm_after_pause; return 1; }
    blend_restore_xlm blend_restore_xlm_after_pause

    inv blend_freeze_xlm "$ADMIN" "$CONTROLLER" -- set_spoke_asset_flags \
        --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" \
        --paused false --frozen true --no_seize false >/dev/null
    xfail blend_collateral_frozen 'Error\(Contract, #316\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty" \
        || { blend_restore_xlm blend_restore_xlm_after_freeze; return 1; }
    blend_restore_xlm blend_restore_xlm_after_freeze

    inv blend_xlm_not_coll "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$XLM_SAC" "$PRIMARY_SPOKE_ID" false true 7000 7500 1000)" >/dev/null
    xfail blend_not_collateral 'Error\(Contract, #104\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty" \
        || { blend_restore_xlm blend_restore_xlm_after_not_coll; return 1; }
    blend_restore_xlm blend_restore_xlm_after_not_coll

    inv blend_xlm_not_borr "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$XLM_SAC" "$PRIMARY_SPOKE_ID" true false 7000 7500 1000)" >/dev/null
    xfail blend_debt_not_borrowable 'Error\(Contract, #107\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$empty" --supply_assets "$empty" --debt_caps "$xlm_debt" \
        || { blend_restore_xlm blend_restore_xlm_after_not_borr; return 1; }
    blend_restore_xlm blend_restore_xlm_after_not_borr
}

flow_blend_migrate() {
    phase blend_migrate

    local coll_amt=2000000000
    local supply_amt=500000000
    local debt_amt=300000000
    local debt_cap=360000000
    local extra_coll=200000000
    local unhealthy_debt=1500000000
    local empty='[]'
    local xlm_coll xlm_supply xlm_debt xlm_dup
    xlm_coll=$(blend_addr_json "$XLM_SAC")
    xlm_supply=$(blend_addr_json "$XLM_SAC")
    xlm_debt=$(blend_debt_json "$XLM_SAC" "$debt_cap")
    xlm_dup=$(blend_addr_dup_json "$XLM_SAC")

    blend_restore_min_borrow blend_min_borrow_off 0
    local ctrl_xlm_before
    ctrl_xlm_before=$(blend_ctrl_xlm)
    _is_uint "${ctrl_xlm_before:-}" || ctrl_xlm_before=0
    log "controller XLM before migrates=$ctrl_xlm_before"

    # Real Blend burns bTokens on withdraw; a listed asset with 0 balance
    # reverts InvalidBTokenBurnAmount (#1217). The mock no-ops this path.
    xfail blend_zero_blend_balance 'Error\(Contract, #1217\)' "$EVE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$EVE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    # Zero Blend liability + hub debt cap: live Blend rejects a dToken burn of 0
    # (#1219). Position is unchanged, then a coll-only migrate must still work.
    blend_seed blend_seed_eve_coll "$EVE" "$EVE_ADDR" "$coll_amt" 0 0 || return 1
    xfail blend_zero_liab_cap 'Error\(Contract, #1219\)|#1217' "$EVE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$EVE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$xlm_debt"
    local eve_acct
    eve_acct=$(blend_migrate migrate_eve_coll_only "$EVE" "$EVE_ADDR" 0 \
        "$xlm_coll" "$empty" "$empty") || return 1
    save_state EVE_BLEND_ACCT "$eve_acct"
    assert_bool_view migrate_eve_exists true account_exists --account_id "$eve_acct"
    assert_int_view_positive migrate_eve_coll get_collateral_amount \
        --account_id "$eve_acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")"
    local eve_debt
    eve_debt=$(_view_int migrate_eve_debt get_borrow_amount --account_id "$eve_acct" \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" || echo 0)
    if [ "${eve_debt:-0}" = "0" ] || [ -z "$eve_debt" ]; then
        record migrate_eve_debt_free ok get_borrow_amount "" "" "" "" "" "0"
    else
        _assert_fail migrate_eve_debt_free "coll-only migrate left hub debt $eve_debt"
    fi
    assert_hf_at_least migrate_eve_hf "$eve_acct" "$WAD"

    local rc alice_has_debt=0
    blend_seed blend_seed_alice_debtcoll "$ALICE" "$ALICE_ADDR" "$coll_amt" "$supply_amt" "$debt_amt"
    rc=$?
    [ "$rc" -eq 0 ] && alice_has_debt=1
    local alice_pos
    alice_pos=$(blend_positions blend_alice_seeded "$ALICE_ADDR")
    echo "$alice_pos" | jq -e '.collateral != {} or .supply != {} or .liabilities != {}' >/dev/null \
        || die blend_alice_seeded "Alice Blend position empty after seed"
    if [ "$alice_has_debt" -eq 1 ] && ! blend_has_map "$alice_pos" liabilities; then
        log "alice seed reported debt success but Blend liabilities empty"
        alice_has_debt=0
    fi
    log "alice blend liabilities=$(blend_map_sum "$alice_pos" liabilities) seed_debt=$debt_amt cap=$debt_cap has_debt=$alice_has_debt"

    local alice_acct
    if [ "$alice_has_debt" -eq 1 ]; then
        alice_acct=$(blend_migrate migrate_alice_debtcoll "$ALICE" "$ALICE_ADDR" 0 \
            "$xlm_coll" "$xlm_supply" "$xlm_debt") || return 1
    else
        alice_acct=$(blend_migrate migrate_alice_collsupply "$ALICE" "$ALICE_ADDR" 0 \
            "$xlm_coll" "$xlm_supply" "$empty") || return 1
    fi
    save_state ALICE_BLEND_ACCT "$alice_acct"
    assert_bool_view migrate_alice_exists true account_exists --account_id "$alice_acct"
    assert_hf_at_least migrate_alice_hf "$alice_acct" "$WAD"
    assert_int_view_positive migrate_alice_coll get_collateral_amount \
        --account_id "$alice_acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")"
    local alice_coll
    alice_coll=$(_view_int migrate_alice_coll_raw get_collateral_amount \
        --account_id "$alice_acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")")
    _uint_ge "$alice_coll" $((coll_amt * 90 / 100)) \
        || _assert_fail migrate_alice_coll_min "coll $alice_coll want >= $((coll_amt * 90 / 100))"

    if [ "$alice_has_debt" -eq 1 ]; then
        assert_borrow_at_least migrate_alice_debt_min "$alice_acct" "$XLM_SAC" $((debt_amt * 90 / 100))
        assert_borrow_at_most migrate_alice_debt_max "$alice_acct" "$XLM_SAC" $((debt_amt * 120 / 100 + 1))
        assert_borrow_at_most migrate_alice_debt_below_cap "$alice_acct" "$XLM_SAC" $((debt_cap - 1))
    fi

    local alice_after
    alice_after=$(blend_positions blend_alice_swept "$ALICE_ADDR")
    if blend_maps_empty "$alice_after"; then
        record blend_alice_swept_empty ok get_positions "" "" "" "" "" "blend maps empty"
    elif [ "$alice_has_debt" -eq 1 ] && blend_has_map "$alice_after" liabilities; then
        _assert_fail blend_alice_swept_empty "Blend liabilities remain after debt migrate"
    else
        record blend_alice_swept_residual ok get_positions "" "" "" "" "" \
            "$(echo "$alice_after" | tr '\n' ' ' | head -c 160)"
    fi

    blend_seed blend_seed_bob_coll "$BOB" "$BOB_ADDR" "$coll_amt" 0 0 || return 1
    local bob_acct
    bob_acct=$(blend_migrate migrate_bob_coll_only "$BOB" "$BOB_ADDR" 0 \
        "$xlm_coll" "$empty" "$empty") || return 1
    save_state BOB_BLEND_ACCT "$bob_acct"
    assert_bool_view migrate_bob_exists true account_exists --account_id "$bob_acct"
    assert_int_view_positive migrate_bob_coll get_collateral_amount \
        --account_id "$bob_acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")"
    local bob_debt
    bob_debt=$(_view_int migrate_bob_debt get_borrow_amount --account_id "$bob_acct" \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" || echo 0)
    if [ "${bob_debt:-0}" = "0" ] || [ -z "$bob_debt" ]; then
        record migrate_bob_debt_free ok get_borrow_amount "" "" "" "" "" "0"
    else
        _assert_fail migrate_bob_debt_free "coll-only migrate left hub debt $bob_debt"
    fi
    assert_hf_at_least migrate_bob_hf "$bob_acct" "$WAD"

    local bob_coll_mid
    bob_coll_mid=$(_view_int bob_coll_mid get_collateral_amount --account_id "$bob_acct" \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")")
    # Already-swept Blend position: live pool rejects i128::MAX bToken burn.
    xfail blend_remigrate_empty 'Error\(Contract, #1217\)' "$BOB" "$CONTROLLER" -- migrate_from_blend \
        --caller "$BOB_ADDR" --account_id "$bob_acct" --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_dup" --supply_assets "$empty" --debt_caps "$empty"
    local bob_coll_same
    bob_coll_same=$(_view_int bob_coll_after_empty get_collateral_amount --account_id "$bob_acct" \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")")
    [ "$bob_coll_same" = "$bob_coll_mid" ] \
        || _assert_fail bob_coll_unchanged_empty "failed remigrate changed coll $bob_coll_mid -> $bob_coll_same"

    inv blend_manager_alice "$ADMIN" "$CONTROLLER" -- set_position_manager \
        --manager "$ALICE_ADDR" --is_active true >/dev/null
    inv blend_bob_add_delegate "$BOB" "$CONTROLLER" -- add_delegate \
        --caller "$BOB_ADDR" --account_id "$bob_acct" --delegate "$ALICE_ADDR" >/dev/null
    # migrate_from_blend always sweeps `caller`'s Blend position. A delegate
    # therefore moves the delegate's Blend funds into the owner's hub account.
    blend_seed blend_seed_alice_extra "$ALICE" "$ALICE_ADDR" "$extra_coll" 0 0 || return 1
    blend_migrate_inv migrate_bob_via_delegate "$ALICE" "$ALICE_ADDR" "$bob_acct" \
        "$xlm_coll" "$empty" "$empty" >/dev/null || return 1
    local bob_coll_after
    bob_coll_after=$(_view_int bob_coll_post_delegate get_collateral_amount --account_id "$bob_acct" \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")")
    _uint_ge "$bob_coll_after" $((bob_coll_mid + extra_coll * 80 / 100)) \
        || _assert_fail bob_coll_grew_delegate "coll $bob_coll_mid -> $bob_coll_after after delegate migrate"
    inv blend_bob_remove_delegate "$BOB" "$CONTROLLER" -- remove_delegate \
        --caller "$BOB_ADDR" --account_id "$bob_acct" --delegate "$ALICE_ADDR" >/dev/null
    inv blend_manager_alice_off "$ADMIN" "$CONTROLLER" -- set_position_manager \
        --manager "$ALICE_ADDR" --is_active false >/dev/null
    xfail blend_delegate_removed 'Error\(Contract, #44\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id "$bob_acct" --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    local carol_rc=1
    blend_seed blend_seed_carol_supply "$CAROL" "$CAROL_ADDR" 0 "$supply_amt" 0
    carol_rc=$?
    local carol_as_supply=0
    [ "$carol_rc" -eq 0 ] && carol_as_supply=1
    if [ "$carol_rc" -ne 0 ]; then
        blend_seed blend_seed_carol_supply_as_coll "$CAROL" "$CAROL_ADDR" "$supply_amt" 0 0 || return 1
    fi
    local carol_acct
    if [ "$carol_as_supply" -eq 1 ]; then
        carol_acct=$(blend_migrate migrate_carol_supply_only "$CAROL" "$CAROL_ADDR" 0 \
            "$empty" "$xlm_supply" "$empty") || \
            carol_acct=$(blend_migrate migrate_carol_as_coll "$CAROL" "$CAROL_ADDR" 0 \
                "$xlm_coll" "$empty" "$empty") || return 1
    else
        carol_acct=$(blend_migrate migrate_carol_as_coll "$CAROL" "$CAROL_ADDR" 0 \
            "$xlm_coll" "$empty" "$empty") || return 1
    fi
    save_state CAROL_BLEND_ACCT "$carol_acct"
    assert_bool_view migrate_carol_exists true account_exists --account_id "$carol_acct"
    assert_int_view_positive migrate_carol_coll get_collateral_amount \
        --account_id "$carol_acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")"
    assert_hf_at_least migrate_carol_hf "$carol_acct" "$WAD"

    local dave_hub
    dave_hub=$(inv_create dave_hub_supply "$DAVE" "$CONTROLLER" -- supply \
        --caller "$DAVE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 100000000)" | tr -d '"') || return 1
    local dave_coll_before
    dave_coll_before=$(_view_int dave_coll_pre get_collateral_amount --account_id "$dave_hub" \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")")
    blend_seed blend_seed_dave_coll "$DAVE" "$DAVE_ADDR" "$coll_amt" 0 0 || return 1
    local dave_acct
    dave_acct=$(blend_migrate_inv migrate_dave_existing "$DAVE" "$DAVE_ADDR" "$dave_hub" \
        "$xlm_coll" "$empty" "$empty") || return 1
    [ "$dave_acct" = "$dave_hub" ] || log "dave migrate returned id=$dave_acct hub=$dave_hub"
    local dave_coll_after
    dave_coll_after=$(_view_int dave_coll_post get_collateral_amount --account_id "$dave_hub" \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")")
    _uint_ge "$dave_coll_after" $((dave_coll_before + coll_amt * 80 / 100)) \
        || _assert_fail dave_coll_grew "collateral $dave_coll_before -> $dave_coll_after"
    assert_hf_at_least migrate_dave_hf "$dave_hub" "$WAD"
    save_state DAVE_BLEND_ACCT "$dave_hub"

    xfail blend_spoke_mismatch 'Error\(Contract, #310\)' "$DAVE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$DAVE_ADDR" --account_id "$dave_hub" --spoke_id "$SECONDARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    xfail blend_wrong_owner 'Error\(Contract, #44\)' "$BOB" "$CONTROLLER" -- migrate_from_blend \
        --caller "$BOB_ADDR" --account_id "$alice_acct" --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    # Frank starts Blend-healthy (30 XLM debt / 200 XLM coll) so cap / no-coll /
    # min-borrow gates are distinguishable from hub #100. Extra Blend borrow
    # then pushes into the c_factor 0.90 vs LTV 0.70 unhealthy window.
    local frank_has_debt=0 frank_unhealthy=0
    blend_seed blend_seed_frank_debt "$FRANK" "$FRANK_ADDR" "$coll_amt" 0 "$debt_amt"
    rc=$?
    [ "$rc" -eq 0 ] && frank_has_debt=1
    if [ "$frank_has_debt" -eq 1 ]; then
        xfail blend_cap_too_low 'Error\(Contract' "$FRANK" "$CONTROLLER" -- migrate_from_blend \
            --caller "$FRANK_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
            --collateral_assets "$xlm_coll" --supply_assets "$empty" \
            --debt_caps "$(blend_debt_json "$XLM_SAC" 1)"
        xfail blend_debt_without_collateral 'Error\(Contract, #100\)|#126' "$FRANK" "$CONTROLLER" -- migrate_from_blend \
            --caller "$FRANK_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
            --collateral_assets "$empty" --supply_assets "$empty" \
            --debt_caps "$(blend_debt_json "$XLM_SAC" "$unhealthy_debt")"
        inv blend_min_borrow_high "$ADMIN" "$CONTROLLER" -- set_min_borrow_collateral_usd \
            --floor_wad 1000000000000000000000000000000000 >/dev/null
        xfail blend_min_borrow 'Error\(Contract, #126\)' "$FRANK" "$CONTROLLER" -- migrate_from_blend \
            --caller "$FRANK_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
            --collateral_assets "$xlm_coll" --supply_assets "$empty" \
            --debt_caps "$xlm_debt"
        blend_restore_min_borrow blend_min_borrow_reset 0
        blend_seed blend_seed_frank_unhealthy "$FRANK" "$FRANK_ADDR" 0 0 $((unhealthy_debt - debt_amt))
        rc=$?
        if [ "$rc" -eq 0 ]; then
            frank_unhealthy=1
            xfail blend_unhealthy_end 'Error\(Contract, #100\)|#126' "$FRANK" "$CONTROLLER" -- migrate_from_blend \
                --caller "$FRANK_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
                --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
                --collateral_assets "$xlm_coll" --supply_assets "$empty" \
                --debt_caps "$(blend_debt_json "$XLM_SAC" $((unhealthy_debt + unhealthy_debt / 5)))"
        else
            record blend_unhealthy_end environment-blocked submit "" "" "" "" "" \
                "Blend rejected extra borrow into the hub-unhealthy window"
        fi
    else
        record blend_frank_debt_seed environment-blocked submit "" "" "" "" "" \
            "Blend pool had no borrow liquidity; skipped cap-too-low, min-borrow, unhealthy"
    fi

    if [ "$alice_has_debt" -eq 0 ]; then
        record blend_alice_debt_seed environment-blocked submit "" "" "" "" "" \
            "Blend pool had no borrow liquidity for Alice; same-asset debt loop skipped"
    fi

    blend_assert_ctrl_xlm_clean blend_controller_xlm_clean "$ctrl_xlm_before"
}
