# Check stored risk snapshots, preserving supply shares and the entire debt map.
risk_assert_stamps() {
    local label="$1" account="$2" asset="$3" expected="$4" before="$5" after
    after=$(view "$label" "$CONTROLLER" -- get_account_positions --account_id "$account") || return 1
    if ! jq -e --arg asset "$asset" --argjson hub "$PRIMARY_HUB_ID" \
        --argjson expected "$expected" --argjson before "$before" '
        def principal: [(.[0] | with_entries(.value |= {scaled_amount})), .[1]];
        (principal == ($before | principal)) and
        ([.[0] | to_entries[] | select((.key | fromjson) == {asset:$asset,hub_id:$hub}) |
          .value | [.loan_to_value,.liquidation_threshold,.liquidation_bonus,.liquidation_fees]] == [$expected])
    ' <<<"$after" >/dev/null; then
        _assert_fail "$label" "risk tuple must be $expected with unchanged supply shares and debt: $after"
        return 1
    fi
    record "$label" ok assert "" "" "" "" "" "stored risk tuple $expected; principal unchanged"
}

flow_risk_refresh() {
    phase risk_refresh
    # Zero rates from creation keep both indexes at RAY. At price $1, 150
    # collateral * 70% / 100 debt is exactly HF 1.05; test +/- one raw unit.
    deploy_mock_reflector || return 1
    issue_sac SAC_RISK RISK || return 1
    trustline "$ALICE" RISK "$ADMIN_ADDR" || return 1
    mint_to "$SAC_RISK" RISK "$ALICE_ADDR" 100000000000 || return 1
    set_mock_price "$SAC_RISK" "$WAD" risk_price || return 1
    local key params other
    key=$(hub_key "$PRIMARY_HUB_ID" "$SAC_RISK")
    params=$(market_params_json "$SAC_RISK" 7 | jq -c '.base_borrow_rate="0" | .slope1="0" | .slope2="0" | .slope3="0"') || return 1
    inv risk_create_market "$ADMIN" "$CONTROLLER" -- create_liquidity_pool \
        --hub_id "$PRIMARY_HUB_ID" --asset "$SAC_RISK" --params "$params" >/dev/null || return 1
    oracle_cfg_mock_single "$SAC_RISK" > "$LOG_DIR/risk_oracle_config.json" || return 1
    view risk_resolve_oracle "$GOVERNANCE" -- resolve_asset_oracle --key "$(price_key_token "$SAC_RISK")" \
        --oracle-file-path "$LOG_DIR/risk_oracle_config.json" > "$LOG_DIR/risk_oracle_resolved.json" || return 1
    inv risk_set_oracle "$ADMIN" "$PRICE_AGGREGATOR" -- set_oracle --key "$(price_key_token "$SAC_RISK")" \
        --oracle-file-path "$LOG_DIR/risk_oracle_resolved.json" >/dev/null || return 1
    inv risk_primary_listing "$ADMIN" "$CONTROLLER" -- add_asset_to_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$SAC_RISK" "$PRIMARY_SPOKE_ID" true true 7500 8000 500)" >/dev/null || return 1
    other=$(inv risk_other_spoke "$ADMIN" "$CONTROLLER" -- add_spoke | tr -d '\"[:space:]') || return 1
    inv risk_other_listing "$ADMIN" "$CONTROLLER" -- add_asset_to_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$SAC_RISK" "$other" true true 8000 8100 400)" >/dev/null || return 1
    save_state MARKETS "${MARKETS:+$MARKETS }$PRIMARY_HUB_ID:$SAC_RISK"

    local below equal above free low account amount spoke debt name snapshot
    local below_before equal_before above_before free_before low_before
    for name in below equal above free low; do
        spoke="$PRIMARY_SPOKE_ID"; debt=1000000000
        case "$name" in
            below) amount=1499999999;;
            equal) amount=1500000000;;
            above) amount=1500000001;;
            free) amount=100000000; spoke="$other"; debt=0;;
            low) amount=1500000000; spoke="$other"; debt=1200000000;;
        esac
        account=$(inv_create "risk_supply_$name" "$ALICE" "$CONTROLLER" -- supply \
            --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$spoke" \
            --assets "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_RISK" "$amount")") || return 1
        printf -v "$name" '%s' "$account"
        if [ "$debt" -gt 0 ]; then
            inv "risk_borrow_$name" "$ALICE" "$CONTROLLER" -- borrow --caller "$ALICE_ADDR" \
                --account_id "$account" --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$SAC_RISK" "$debt")" --to null >/dev/null || return 1
            assert_int_view_eq "risk_debt_$name" "$debt" get_borrow_amount --account_id "$account" --hub_asset "$key" || return 1
        fi
        snapshot=$(view "risk_before_$name" "$CONTROLLER" -- get_account_positions --account_id "$account") || return 1
        printf -v "${name}_before" '%s' "$snapshot"
    done
    assert_view_eq_at "$POOL" risk_zero_rate 0 get_borrow_rate --hub_asset "$key" || return 1
    assert_int_view_eq risk_low_hf 1012500000000000000 get_health_factor --account_id "$low" || return 1

    inv risk_edit_primary "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$SAC_RISK" "$PRIMARY_SPOKE_ID" true true 5000 7000 900 | jq -c '.liquidation_fees=50')" >/dev/null || return 1
    inv risk_edit_other "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$SAC_RISK" "$other" true true 5100 6900 800 | jq -c '.liquidation_fees=60')" >/dev/null || return 1
    # Permissionless keeper; alternate spokes within the same cached batch.
    inv risk_ltv_batch "$BOB" "$CONTROLLER" -- update_account_threshold --caller "$BOB_ADDR" \
        --has_risks false --account_ids "[$below,$free,$equal,$low,$above]" >/dev/null || return 1
    for name in below equal above; do
        snapshot="${name}_before"
        risk_assert_stamps "risk_ltv_only_$name" "${!name}" "$SAC_RISK" '[5000,8000,500,100]' "${!snapshot}" || return 1
    done
    risk_assert_stamps risk_ltv_only_free "$free" "$SAC_RISK" '[5100,8100,400,100]' "$free_before" || return 1
    risk_assert_stamps risk_ltv_only_low "$low" "$SAC_RISK" '[5100,8100,400,100]' "$low_before" || return 1

    inv risk_full_batch "$BOB" "$CONTROLLER" -- update_account_threshold --caller "$BOB_ADDR" \
        --has_risks true --account_ids "[$below,$free,$equal,$above]" >/dev/null || return 1
    risk_assert_stamps risk_gate_below "$below" "$SAC_RISK" '[5000,8000,500,100]' "$below_before" || return 1
    risk_assert_stamps risk_gate_equal "$equal" "$SAC_RISK" '[5000,7000,900,50]' "$equal_before" || return 1
    risk_assert_stamps risk_gate_above "$above" "$SAC_RISK" '[5000,7000,900,50]' "$above_before" || return 1
    risk_assert_stamps risk_debt_free "$free" "$SAC_RISK" '[5100,6900,800,60]' "$free_before" || return 1
    assert_int_view_eq risk_hf_equal 1050000000000000000 get_health_factor --account_id "$equal" || return 1
    assert_int_view_eq risk_hf_above 1050000000700000000 get_health_factor --account_id "$above" || return 1
    assert_int_view_eq risk_hf_held 1199999999200000000 get_health_factor --account_id "$below" || return 1

    # An existing HF below 1.05 rejects the full batch, even when its adverse
    # tuple is held. The preceding debt-free update must roll back as well.
    inv risk_edit_rollback "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$SAC_RISK" "$other" true true 5200 6800 1000 | jq -c '.liquidation_fees=25')" >/dev/null || return 1
    xfail risk_full_low_hf 'Error\(Contract, #102\)' "$BOB" "$CONTROLLER" -- update_account_threshold \
        --caller "$BOB_ADDR" --has_risks true --account_ids "[$free,$low]" || return 1
    risk_assert_stamps risk_atomic_free "$free" "$SAC_RISK" '[5100,6900,800,60]' "$free_before" || return 1
    risk_assert_stamps risk_atomic_low "$low" "$SAC_RISK" '[5100,8100,400,100]' "$low_before" || return 1
    inv risk_ltv_low_hf "$BOB" "$CONTROLLER" -- update_account_threshold --caller "$BOB_ADDR" \
        --has_risks false --account_ids "[$free,$low]" >/dev/null || return 1
    risk_assert_stamps risk_ltv_low_hf_free "$free" "$SAC_RISK" '[5200,6900,800,60]' "$free_before" || return 1
    risk_assert_stamps risk_ltv_low_hf_debt "$low" "$SAC_RISK" '[5200,8100,400,100]' "$low_before"
}

flow_admin() {
    phase admin

    inv admin_pause "$ADMIN" "$CONTROLLER" -- pause >/dev/null
    xfail paused_supply 'Error\(Contract, #1000\)' "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 1000000000)"
    inv admin_unpause "$ADMIN" "$CONTROLLER" -- unpause >/dev/null
    xfail unpause_when_live 'Error\(Contract, #1001\)' "$ADMIN" "$CONTROLLER" -- unpause

    inv set_position_limits "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits '{"max_supply_positions":5,"max_borrow_positions":5}' >/dev/null

    inv update_pool_params "$ADMIN" "$CONTROLLER" -- upgrade_liquidity_pool_params \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$EURC_SAC")" \
        --params "$(market_params_json "$EURC_SAC" 7 | jq -c '{
            max_borrow_rate, base_borrow_rate, slope1, slope2, slope3,
            mid_utilization, optimal_utilization, max_utilization,
            reserve_factor: 1500, is_flashloanable, flashloan_fee
        }')" >/dev/null

    inv edit_asset_config_admin "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$EURC_SAC" "$PRIMARY_SPOKE_ID" true true 6500 7000 900)" >/dev/null
    assert_market_field market_cfg_ltv "$EURC_SAC" loan_to_value 6500
    assert_market_field market_cfg_thr "$EURC_SAC" liquidation_threshold 7000
    assert_market_field market_cfg_bonus "$EURC_SAC" liquidation_bonus 900

    xfail asset_cfg_bad_bounds 'Error\(Contract, #113\)' "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$EURC_SAC" "$PRIMARY_SPOKE_ID" true true 9000 7000 900)"

    local tol_bands eurc_key
    eurc_key=$(price_key_token "$EURC_SAC")
    tol_bands=$(view oracle_tol_resolve "$GOVERNANCE" -- resolve_oracle_tolerance \
        --tolerance 300)
    inv set_tolerance "$ADMIN" "$PRICE_AGGREGATOR" -- set_tolerance \
        --key "$eurc_key" --tolerance "$tol_bands" >/dev/null

    xfail oracle_tol_owner_guard "Missing signing key for account $ADMIN_ADDR" "$ALICE" "$PRICE_AGGREGATOR" -- set_tolerance \
        --key "$eurc_key" --tolerance "$tol_bands"

    flow_price_aggregator_extra "$eurc_key"

    inv update_indexes "$ADMIN" "$CONTROLLER" -- update_indexes \
        --caller "$ADMIN_ADDR" --assets "$(hub_vec "$PRIMARY_HUB_ID" "$XLM_SAC" "$USDC_SAC" "$EURC_SAC")" >/dev/null
    inv update_indexes_alice "$ALICE" "$CONTROLLER" -- update_indexes \
        --caller "$ALICE_ADDR" --assets "$(hub_vec "$PRIMARY_HUB_ID" "$XLM_SAC")" >/dev/null

    inv update_account_threshold "$ADMIN" "$CONTROLLER" -- update_account_threshold \
        --caller "$ADMIN_ADDR" --has_risks false \
        --account_ids "[${ADMIN_ACCT:-1}]" >/dev/null
    inv update_account_threshold "$ALICE" "$CONTROLLER" -- update_account_threshold \
        --caller "$ALICE_ADDR" --has_risks false --account_ids "[${ADMIN_ACCT:-1}]" >/dev/null

    local pool_rev_before
    pool_rev_before=$(_view_pool_int pool_revenue_pre get_revenue --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    local accumulator_pre caller_pre pool_pre controller_pre claimed
    accumulator_pre=$(balance "$USDC_SAC" "$ADMIN_ADDR") || return 1
    caller_pre=$(balance "$USDC_SAC" "$ALICE_ADDR") || return 1
    pool_pre=$(balance "$USDC_SAC" "$POOL") || return 1
    controller_pre=$(balance "$USDC_SAC" "$CONTROLLER") || return 1
    claimed=$(inv claim_revenue "$ALICE" "$CONTROLLER" -- claim_revenue \
        --caller "$ALICE_ADDR" --assets "$(hub_vec "$PRIMARY_HUB_ID" "$USDC_SAC")" | jq -er '.[0]') || return 1
    _uint_ge "$claimed" 1 || { _assert_fail claim_nonzero "claim must exercise nonzero fees"; return 1; }
    assert_delta claim_recipient "$accumulator_pre" "$(balance "$USDC_SAC" "$ADMIN_ADDR")" "$claimed" || return 1
    assert_delta claim_pool "$pool_pre" "$(balance "$USDC_SAC" "$POOL")" "$(raw_sub 0 "$claimed")" || return 1
    assert_delta claim_caller "$caller_pre" "$(balance "$USDC_SAC" "$ALICE_ADDR")" 0 || return 1
    assert_delta claim_controller "$controller_pre" "$(balance "$USDC_SAC" "$CONTROLLER")" 0 || return 1
    assert_pool_revenue_decreased pool_revenue_post "$USDC_SAC" "$pool_rev_before"
    view pool_rates_view "$POOL" -- get_borrow_rate --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" >/dev/null
    view pool_util_view "$POOL" -- get_utilisation --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" >/dev/null

    # The remaining pool reads must return a non-negative value for a live market.
    local usdc_hub
    usdc_hub=$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")
    assert_int_view_at_nonneg pool_reserves_view "$POOL" get_reserves --hub_asset "$usdc_hub"
    assert_int_view_at_nonneg pool_supplied_view "$POOL" get_supplied_amount --hub_asset "$usdc_hub"
    assert_int_view_at_nonneg pool_borrowed_view "$POOL" get_borrowed_amount --hub_asset "$usdc_hub"
    assert_int_view_at_nonneg pool_deposit_rate_view "$POOL" get_deposit_rate --hub_asset "$usdc_hub"
    assert_int_view_at_nonneg pool_delta_time_view "$POOL" get_delta_time --hub_asset "$usdc_hub"

    local tmp_cat
    tmp_cat=$(inv spoke_tmp_add "$ADMIN" "$CONTROLLER" -- add_spoke | tr -d '"')
    inv spoke_tmp_add_asset "$ADMIN" "$CONTROLLER" -- add_asset_to_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$EURC_SAC" "$tmp_cat" true true 8000 8500 300)" >/dev/null

    xfail spoke_bad_bounds 'Error\(Contract, #113\)' "$ADMIN" "$CONTROLLER" -- add_asset_to_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$EURC_SAC" "$tmp_cat" true true 8600 8500 300)"
    inv spoke_tmp_edit_asset "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$EURC_SAC" "$tmp_cat" true false 8100 8600 250)" >/dev/null
inv spoke_tmp_remove_asset "$ADMIN" "$CONTROLLER" -- remove_asset_from_spoke \
--hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$EURC_SAC")" --spoke_id "$tmp_cat" >/dev/null
    inv spoke_tmp_deprecate "$ADMIN" "$CONTROLLER" -- remove_spoke --id "$tmp_cat" >/dev/null
    xfail spoke_deprecated_supply 'Error\(Contract, #301\)' "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$tmp_cat" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 1000000000)"

    local bob_minb_acct
    bob_minb_acct=$(inv_create minb_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 5000000000)" | tr -d '"')
    inv minb_set_high "$ADMIN" "$CONTROLLER" -- set_min_borrow_collateral_usd \
        --floor_wad 1000000000000000000000000000000000 >/dev/null
    assert_int_view_eq minb_read_high 1000000000000000000000000000000000 get_min_borrow_collateral_usd
    xfail minb_borrow_blocked 'Error\(Contract, #126\)' "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$bob_minb_acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 1000000)" --to null
    inv minb_reset "$ADMIN" "$CONTROLLER" -- set_min_borrow_collateral_usd --floor_wad 0 >/dev/null
    assert_int_view_eq minb_read_zero 0 get_min_borrow_collateral_usd
    xfail minb_negative 'Error\(Contract, #116\)' "$ADMIN" "$CONTROLLER" -- set_min_borrow_collateral_usd \
--floor_wad=-1

view pool_address_view "$CONTROLLER" -- get_pool_address >/dev/null
view market_index_xlm "$CONTROLLER" -- get_market_index \
--hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" >/dev/null
view total_borrow_bob_minb "$CONTROLLER" -- get_total_borrow_usd \
--account_id "$bob_minb_acct" >/dev/null
inv manager_activate_alice "$ADMIN" "$CONTROLLER" -- set_position_manager \
--manager "$ALICE_ADDR" --is_active true >/dev/null
inv delegate_add_alice "$BOB" "$CONTROLLER" -- add_delegate \
--caller "$BOB_ADDR" --account_id "$bob_minb_acct" --delegate "$ALICE_ADDR" >/dev/null
inv delegated_borrow_usdc "$ALICE" "$CONTROLLER" -- borrow \
--caller "$ALICE_ADDR" --account_id "$bob_minb_acct" \
--borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 1000000)" --to null >/dev/null
inv delegate_remove_alice "$BOB" "$CONTROLLER" -- remove_delegate \
--caller "$BOB_ADDR" --account_id "$bob_minb_acct" --delegate "$ALICE_ADDR" >/dev/null
xfail delegated_borrow_removed 'Error\(Contract, #44\)' "$ALICE" "$CONTROLLER" -- borrow \
--caller "$ALICE_ADDR" --account_id "$bob_minb_acct" \
--borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 1000000)" --to null
inv manager_deactivate_alice "$ADMIN" "$CONTROLLER" -- set_position_manager \
--manager "$ALICE_ADDR" --is_active false >/dev/null

local blend_pool blend_seeded
blend_pool=$(jq -r '.pools[0].address // empty' "$REPO_ROOT/configs/$NETWORK/blend.json")
if [ -n "$blend_pool" ] && [ "$blend_pool" != "null" ]; then
# Assert each allowlist transition: an approve or revoke must move the flag.
view blend_pool_initial "$CONTROLLER" -- is_blend_pool_approved --pool "$blend_pool" >/dev/null
inv blend_pool_approve "$ADMIN" "$CONTROLLER" -- approve_blend_pool --pool "$blend_pool" >/dev/null
assert_bool_view blend_pool_true true is_blend_pool_approved --pool "$blend_pool"
inv blend_pool_revoke "$ADMIN" "$CONTROLLER" -- revoke_blend_pool --pool "$blend_pool" >/dev/null
assert_bool_view blend_pool_false false is_blend_pool_approved --pool "$blend_pool"
inv blend_pool_reapprove "$ADMIN" "$CONTROLLER" -- approve_blend_pool --pool "$blend_pool" >/dev/null
assert_bool_view blend_pool_reapproved true is_blend_pool_approved --pool "$blend_pool"
# Migration financial/negative coverage belongs to the isolated Blend and SDK
# lanes; this case exercises only approval administration.
fi

    local xlm_sec_band
    xlm_sec_band=$(reflector_band XLM) || { log "XLM live price unavailable; cannot calibrate secondary sanity band"; return 1; }
    create_market XLM_SECONDARY "$SECONDARY_HUB_ID" "$XLM_SAC" 7 \
        "$(oracle_cfg_reflector XLM $xlm_sec_band)" \
        "$(asset_config_json 7000 7500 1000)"
    view market_index_secondary_xlm "$CONTROLLER" -- get_market_index \
        --hub_asset "$(hub_key "$SECONDARY_HUB_ID" "$XLM_SAC")" >/dev/null
    local secondary_acct
    secondary_acct=$(inv_create secondary_hub_supply "$CAROL" "$CONTROLLER" -- supply \
        --caller "$CAROL_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$SECONDARY_HUB_ID" "$XLM_SAC" 1000000000)" | tr -d '"')
    assert_bool_view secondary_account_exists true account_exists --account_id "$secondary_acct"

}

flow_admin_upgrade() {
    phase admin_upgrade
    local ctrl_hash out_f="$LOG_DIR/upload_ctrl.out" err_f="$LOG_DIR/upload_ctrl.err"
    run_deploy "$out_f" "$err_f" -- stellar contract upload --wasm "$WASM_DIR/controller.wasm" \
        --source "$ADMIN" "${NET_ARGS[@]}" || return 1
    ctrl_hash=$(sanitize_output "$out_f")
    if [ -n "$ctrl_hash" ]; then
        record upload_controller_wasm ok upload \
            "$(extract_signing_hash "$err_f")" \
            "" "" "" "" "$ctrl_hash"
 inv pool_upgrade "$ADMIN" "$CONTROLLER" -- upgrade_pool --new_wasm_hash "$POOL_HASH" >/dev/null
 view pool_address_after_pool_upgrade "$CONTROLLER" -- get_pool_address >/dev/null
 inv controller_upgrade "$ADMIN" "$CONTROLLER" -- upgrade --new_wasm_hash "$ctrl_hash" >/dev/null

        xfail upgraded_paused_gate 'Error\(Contract, #1000\)' "$ALICE" "$CONTROLLER" -- supply \
            --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 1000000000)"
        local ver
        ver=$(view app_version_view "$CONTROLLER" -- get_app_version | tr -d '"')
        inv controller_migrate "$ADMIN" "$CONTROLLER" -- migrate --new_version $((ver + 1)) >/dev/null
        inv unpause_after_upgrade "$ADMIN" "$CONTROLLER" -- unpause >/dev/null
    fi

    # Same-hash position NFT upgrade: it proves the owner-gated controller
    # entrypoint and the NFT's controller-only upgrade auth. Token ids are
    # account ids, so the owner read-back checks the NFT after the upgrade.
    inv nft_upgrade_via_controller "$ADMIN" "$CONTROLLER" -- upgrade_position_nft \
        --new_wasm_hash "$NFT_HASH" >/dev/null
    assert_view_eq_at "$POSITION_NFT" nft_owner_after_upgrade "$ADMIN_ADDR" \
        owner_of --token_id "${ADMIN_ACCT:-1}"

    # renew is permissionless on a live token and fails with #200 on a token that
    # was never minted.
    inv nft_renew "$ALICE" "$POSITION_NFT" -- renew --token_id "${ADMIN_ACCT:-1}" >/dev/null
    xfail nft_renew_missing 'Error\(Contract, #200\)' "$ALICE" "$POSITION_NFT" -- renew --token_id 4000000000

    local ledger
    ledger=$(curl -s -m 30 -X POST "$RPC_URL" -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' | jq -r '.result.sequence')
    # Each leg asserts who holds controller ownership. The controller has no
    # get_owner, so the probe is set_position_limits, an #[only_owner] entry
    # point. The probe re-sets the same valid limits, so it changes nothing.
    local limits='{"max_supply_positions":5,"max_borrow_positions":5}'

    inv ownership_transfer "$ADMIN" "$CONTROLLER" -- transfer_ownership \
        --new_owner "$CAROL_ADDR" --live_until_ledger $((ledger + 1000)) >/dev/null
    # A pending transfer leaves ADMIN as the owner until CAROL accepts.
    inv ownership_admin_still_owner "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits "$limits" >/dev/null

    inv ownership_accept "$CAROL" "$CONTROLLER" -- accept_ownership >/dev/null
    # After the accept, ADMIN is locked out and CAROL is the owner.
    xfail ownership_admin_locked_out "Missing signing key for account $CAROL_ADDR" "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits "$limits"
    inv ownership_carol_now_owner "$CAROL" "$CONTROLLER" -- set_position_limits \
        --limits "$limits" >/dev/null

    inv ownership_transfer_back "$CAROL" "$CONTROLLER" -- transfer_ownership \
        --new_owner "$ADMIN_ADDR" --live_until_ledger $((ledger + 1000)) >/dev/null
    inv ownership_accept_back "$ADMIN" "$CONTROLLER" -- accept_ownership >/dev/null
    # Later owner-gated steps sign as ADMIN, so ownership must return to ADMIN.
    inv ownership_admin_restored "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits "$limits" >/dev/null
}

# Covers the price-aggregator reads and set_sanity_band for `key`.
#
# `seed_oracle` and `remove_oracle` are not covered: they sit behind
# #[cfg(any(test, feature = "testing"))], and `make integration-wasm` builds
# without the `testing` feature, so the deployed wasm does not have them.
flow_price_aggregator_extra() {
    local key="${1:-}"
    [ -n "$key" ] || { _assert_fail pa_prerequisite "missing price key"; return 1; }

    # Reads run before the band change: a narrower sanity band can make later
    # reads of this key fail with #223 SanityBoundViolated.

    # price_spread returns the (low, high) pair behind a resolved price.
    local spread
    spread=$(view pa_price_spread "$PRICE_AGGREGATOR" -- price_spread --key "$key")
    if [ "$(jq -r 'if type == "array" then length else 0 end' <<<"$spread" 2>/dev/null)" = "2" ]; then
        record pa_price_spread_shape ok price_spread "" "" "" "" "" "$(jq -c . <<<"$spread")"
    else
        _assert_fail pa_price_spread_shape "price_spread did not return a 2-tuple: $spread"
    fi

    # Ownership gates every setter below, so assert the identity.
    assert_view_eq_at "$PRICE_AGGREGATOR" pa_get_owner "$ADMIN_ADDR" get_owner

    # `oracle` must return the config registered for this key.
    local orc
    orc=$(view pa_oracle "$PRICE_AGGREGATOR" -- oracle --key "$key")
    if [ -n "$orc" ] && [ "$(jq -r 'if type=="object" then "obj" else . end' <<<"$orc" 2>/dev/null)" = "obj" ]; then
        record pa_oracle_registered ok oracle "" "" "" "" "" "config present for key"
    else
        _assert_fail pa_oracle_registered "oracle returned no config for a registered key: $(head -c 160 <<<"$orc")"
    fi

    # prices and quotes are keyed maps: one requested key must yield at least one
    # entry.
    local keys_json px qt
    keys_json=$(jq -nc --argjson k "$key" '[$k]')
    px=$(view pa_prices "$PRICE_AGGREGATOR" -- prices --keys "$keys_json")
    qt=$(view pa_quotes "$PRICE_AGGREGATOR" -- quotes --keys "$keys_json")
    if [ "$(jq -r 'if type=="object" then (keys|length) elif type=="array" then length else 0 end' <<<"$px" 2>/dev/null)" -ge 1 ]; then
        record pa_prices_keyed ok prices "" "" "" "" "" "1 key -> 1 entry"
    else
        _assert_fail pa_prices_keyed "prices returned no entry for the requested key: $(head -c 160 <<<"$px")"
    fi
    if [ "$(jq -r 'if type=="object" then (keys|length) elif type=="array" then length else 0 end' <<<"$qt" 2>/dev/null)" -ge 1 ]; then
        record pa_quotes_keyed ok quotes "" "" "" "" "" "1 key -> 1 entry"
    else
        _assert_fail pa_quotes_keyed "quotes returned no entry for the requested key: $(head -c 160 <<<"$qt")"
    fi

    # The sanity band is the outer bound on any price the protocol will accept,
    # so it is owner-only. Width is capped for a single-source feed:
    # ceil((max-min)*10000/(max+min)) <= MAX_SINGLE_SOURCE_SANITY_BAND_BPS
    # (1000); +/-8% is 800 bps. Centred on the asset's *current* price rather
    # than on parity, so the band contains the price it is guarding.
    local band_min band_max band_px cur_min cur_max
    band_px=$(jq -r '[.. | objects | select(has("price_wad")) | .price_wad] | first // empty' <<<"$px" 2>/dev/null)
    [[ "$band_px" =~ ^[0-9]+$ ]] || {
        _assert_fail pa_band_centre "prices returned no price_wad: $(head -c 160 <<<"$px")"
        return 1
    }
    band_min=$((band_px / 100 * 92))
    band_max=$((band_px / 100 * 108))
    # set_sanity_band is a one-way ratchet: the new band must sit inside the
    # registered one or it reverts with SanityBandMustTighten (#227). A live
    # feed drifts between registration and here, so clamp into the registered
    # band -- the call must narrow, never widen. Clamping keeps the live price
    # inside the result (any healthy oracle already has reg_min <= px <= reg_max)
    # and can only lower the width in bps, so both band-width bounds still hold.
    cur_min=$(jq -r '.min_sanity_price_wad // empty' <<<"$orc" 2>/dev/null)
    cur_max=$(jq -r '.max_sanity_price_wad // empty' <<<"$orc" 2>/dev/null)
    if [[ "$cur_min" =~ ^[0-9]+$ ]] && [ "$cur_min" -gt "$band_min" ]; then band_min="$cur_min"; fi
    if [[ "$cur_max" =~ ^[0-9]+$ ]] && [ "$cur_max" -lt "$band_max" ]; then band_max="$cur_max"; fi
    inv pa_set_sanity_band "$ADMIN" "$PRICE_AGGREGATOR" -- set_sanity_band \
        --key "$key" --min_wad "$band_min" --max_wad "$band_max" >/dev/null
    view pa_prices_after_band "$PRICE_AGGREGATOR" -- prices --keys "$keys_json" >/dev/null \
        || _assert_fail pa_band_contains_live "the narrowed band rejects the live price"

    xfail pa_set_sanity_band_owner_guard "Missing signing key for account $ADMIN_ADDR" "$ALICE" "$PRICE_AGGREGATOR" -- set_sanity_band \
        --key "$key" --min_wad "$band_min" --max_wad "$band_max"
}

# Calls the pool directly. The pool's mutators are #[only_owner] and its owner
# is the controller, so a direct call from ADMIN must fail. Their happy paths
# run through the controller in other flows. The two reads are asserted on
# shape, not only on success.
flow_pool_surface() {
    phase pool_surface
    local hub_asset
    hub_asset=$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")

    # --- privileged: must reject a non-owner ---
    # require_auth never reverts in recording mode, so the only proof of the
    # owner gate is the CLI failing to sign for the pool's owner (the
    # controller). Every argument must therefore pass body validation: an
    # unused hub id keeps create_market clear of AssetAlreadySupported.
    xfail pool_create_market_not_owner "Missing signing key for account $CONTROLLER" "$ADMIN" "$POOL" -- create_market \
        --hub_id 4242 --params "$(market_params_json "$USDC_SAC" 7)"
    # InterestRateModel also carries is_flashloanable and flashloan_fee; without
    # them the CLI rejects the argument before the auth check.
    xfail pool_update_params_not_owner "Missing signing key for account $CONTROLLER" "$ADMIN" "$POOL" -- update_params \
        --hub_asset "$hub_asset" --model "$(market_params_json "$USDC_SAC" 7 | jq -c '{
            max_borrow_rate, base_borrow_rate, slope1, slope2, slope3,
            mid_utilization, optimal_utilization, max_utilization,
            reserve_factor, is_flashloanable, flashloan_fee
        }')"
    xfail pool_seize_positions_not_owner "Missing signing key for account $CONTROLLER" "$ADMIN" "$POOL" -- seize_positions \
        --entries '[]'
    xfail pool_net_settle_not_owner "Missing signing key for account $CONTROLLER" "$ADMIN" "$POOL" -- net_settle \
        --entry "$(jq -nc --argjson h "$PRIMARY_HUB_ID" --arg a "$USDC_SAC" \
            '{hub_asset:{hub_id:$h,asset:$a},amount:"0",
              supply_position:{scaled_amount:"0"},debt_position:{scaled_amount:"0"}}')"
    # amount 1 (not 0) so the body clears AmountMustBePositive and auth is the
    # only remaining failure.
    xfail pool_create_strategy_not_owner "Missing signing key for account $CONTROLLER" "$ADMIN" "$POOL" -- create_strategy \
        --receiver "$ADMIN_ADDR" --charge_fee false \
        --action "$(jq -nc --argjson h "$PRIMARY_HUB_ID" --arg a "$USDC_SAC" \
            '{position:{scaled_amount:"0"},amount:"1",hub_asset:{hub_id:$h,asset:$a}}')"

    # --- reads: assert shape, since these back hub-side valuation ---
    local sync idx
    sync=$(view pool_get_sync_data "$POOL" -- get_sync_data --hub_asset "$hub_asset")
    if [ "$(jq -r 'has("params") and has("state")' <<<"$sync" 2>/dev/null)" = "true" ]; then
        record pool_sync_data_shape ok get_sync_data "" "" "" "" "" "params+state present"
    else
        _assert_fail pool_sync_data_shape "get_sync_data missing params/state: $(head -c 160 <<<"$sync")"
    fi

    # One key in must yield one index out.
    idx=$(view pool_get_bulk_indexes "$POOL" -- get_bulk_indexes \
        --hub_assets "$(jq -nc --argjson h "$PRIMARY_HUB_ID" --arg a "$USDC_SAC" '[{hub_id:$h,asset:$a}]')")
    if [ "$(jq -r 'if type=="array" then length else 0 end' <<<"$idx" 2>/dev/null)" = "1" ]; then
        record pool_bulk_indexes_shape ok get_bulk_indexes "" "" "" "" "" "1 key -> 1 index"
    else
        _assert_fail pool_bulk_indexes_shape "get_bulk_indexes returned $(jq -c 'length' <<<"$idx" 2>/dev/null) entries for 1 key"
    fi
}

# GH-16 and GH-17 on the seed account ADMIN_ACCT, which holds XLM and USDC until
# teardown. GH-16: a limit below the account's position count keeps top-ups, and
# only top-ups, open. GH-17: the pool and the controller are refused as borrow
# and withdraw recipients before any transfer. The flow restores the limits.
flow_gap_hunt_admin() {
    phase gap_hunt_admin
    [ -n "${ADMIN_ACCT:-}" ] || die gap_hunt_admin "ADMIN_ACCT missing; flow_seed_liquidity must run first"
    assert_bool_view gh_seed_account_live true account_exists --account_id "$ADMIN_ACCT" \
        || die gap_hunt_admin "seed account $ADMIN_ACCT is gone (post-teardown resume?); rerun from lifecycle"
    local xlm_leg
    xlm_leg=$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 10000000)
    # `to` is Option<Address>: the CLI wants JSON, so an address goes as a
    # JSON string and None as null.
    local to_pool to_ctrl to_bob
    to_pool="\"$POOL\""
    to_ctrl="\"$CONTROLLER\""
    to_bob="\"$BOB_ADDR\""

    # --- GH-17: recipients inside the protocol are refused with #412 ---
    xfail gh17_borrow_to_pool 'Error\(Contract, #412\)' "$ADMIN" "$CONTROLLER" -- borrow \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" --borrows "$xlm_leg" --to "$to_pool"
    xfail gh17_borrow_to_controller 'Error\(Contract, #412\)' "$ADMIN" "$CONTROLLER" -- borrow \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" --borrows "$xlm_leg" --to "$to_ctrl"
    xfail gh17_withdraw_to_pool 'Error\(Contract, #412\)' "$ADMIN" "$CONTROLLER" -- withdraw \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" --withdrawals "$xlm_leg" --to "$to_pool"
    xfail gh17_withdraw_to_controller 'Error\(Contract, #412\)' "$ADMIN" "$CONTROLLER" -- withdraw \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" --withdrawals "$xlm_leg" --to "$to_ctrl"
    # An outside recipient still works: 1 XLM to BOB, then repaid by ADMIN.
    inv gh17_borrow_to_bob "$ADMIN" "$CONTROLLER" -- borrow \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" --borrows "$xlm_leg" --to "$to_bob" >/dev/null
    inv gh17_repay_after_borrow "$ADMIN" "$CONTROLLER" -- repay \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" \
        --payments "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 20000000)" >/dev/null

    # --- GH-16: a limit below the account's count keeps top-ups open ---
    # The seed account holds two supply positions (XLM, USDC); a limit of one
    # is below that. Topping up XLM opens no slot and passes; EURC would open
    # a slot and is refused with #109 before any token moves.
    inv gh16_lower_limits "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits '{"max_supply_positions":1,"max_borrow_positions":1}' >/dev/null
    inv gh16_topup_held_asset_over_limit "$ADMIN" "$CONTROLLER" -- supply \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$xlm_leg" >/dev/null
    inv gh16_third_party_topup_over_limit "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id "$ADMIN_ACCT" --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$xlm_leg" >/dev/null
    xfail gh16_new_slot_over_limit 'Error\(Contract, #109\)' "$ADMIN" "$CONTROLLER" -- supply \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$EURC_SAC" 10000000)"
    inv gh16_restore_limits "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits '{"max_supply_positions":5,"max_borrow_positions":5}' >/dev/null
}
