# Admin / owner / keeper endpoint coverage: pause gates, position limits,
# params, oracle admin, revenue, keeper ops, spoke admin lifecycle, and the
# upgrade / migrate / ownership round-trip.
#
# Self-contained on lifecycle markets (XLM, USDC, idle EURC) plus
# ADMIN_ACCT flow_seed_liquidity. It does not depend on mock liquidation markets.
#
# Ordering: upgrade() pauses the protocol. Run upgrade, migrate, and ownership
# checks last, then unpause.

flow_admin() {
    phase admin
    # Pause gate: supply during pause reverts EnforcedPause (#1000).
    inv admin_pause "$ADMIN" "$CONTROLLER" -- pause >/dev/null
    xfail paused_supply 'Error\(Contract, #1000\)' "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 1000000000)"
    inv admin_unpause "$ADMIN" "$CONTROLLER" -- unpause >/dev/null
    xfail unpause_when_live 'Error\(Contract, #1001\)' "$ADMIN" "$CONTROLLER" -- unpause

    # Position limits: the EOA-owned controller setter is a thin owner-only writer.
    # The > POSITION_LIMIT_MAX bound (#36) is validated on the governance propose
    # path (see flows/governance.sh gov_propose_bad_limits), not on this direct
    # setter, so only the valid update runs here.
    inv set_position_limits "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits '{"max_supply_positions":10,"max_borrow_positions":10}' >/dev/null

    # Market param + asset config edits on EURC (idle real market: edits here
    # disturb nothing else, and it is disabled at the end of this flow).
    inv update_pool_params "$ADMIN" "$CONTROLLER" -- upgrade_liquidity_pool_params \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$EURC_SAC")" --params "$(market_params_json "$EURC_SAC" 7 | jq -c 'del(.asset_id, .asset_decimals, .supply_cap, .borrow_cap, .is_flashloanable, .flashloan_fee) | .reserve_factor=1500')" >/dev/null
    # edit_asset_in_spoke on the primary spoke is the canonical per-asset risk edit
    # (replaces the removed edit_asset_config).
    inv edit_asset_config_admin "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$EURC_SAC" "$PRIMARY_SPOKE_ID" true true 6500 7000 900)" >/dev/null
    # Read-back: the edit must land (LTV / threshold / bonus parsed from storage).
    assert_market_field market_cfg_ltv "$EURC_SAC" loan_to_value 6500
    assert_market_field market_cfg_thr "$EURC_SAC" liquidation_threshold 7000
    assert_market_field market_cfg_bonus "$EURC_SAC" liquidation_bonus 900
    # validate_risk_bounds: threshold must exceed LTV (#113 when ltv >= threshold).
    xfail asset_cfg_bad_bounds 'Error\(Contract, #113\)' "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$EURC_SAC" "$PRIMARY_SPOKE_ID" true true 9000 7000 900)"

    # Oracle tolerance: governance resolves the BPS inputs into the 4 ratio bands
    # (resolve_oracle_tolerance view), then the owner-only setter stores them.
    local tol_bands
    tol_bands=$(view oracle_tol_resolve "$GOVERNANCE" -- resolve_oracle_tolerance \
        --tolerance 300)
    inv set_oracle_tolerance "$ADMIN" "$CONTROLLER" -- set_oracle_tolerance \
        --asset "$EURC_SAC" --tolerance "$tol_bands" >/dev/null
    # Owner-gated: a non-owner caller can't satisfy the owner's require_auth(), so
    # the CLI reports a missing signing key for the owner account.
    xfail oracle_tol_owner_guard 'Missing signing key' "$ALICE" "$CONTROLLER" -- set_oracle_tolerance \
        --asset "$EURC_SAC" --tolerance "$tol_bands"

    # Keeper ops (permissionless; caller must sign).
    inv update_indexes "$ADMIN" "$CONTROLLER" -- update_indexes \
        --caller "$ADMIN_ADDR" --assets "$(hub_vec "$PRIMARY_HUB_ID" "$XLM_SAC" "$USDC_SAC" "$EURC_SAC")" >/dev/null
    inv update_indexes "$ALICE" "$CONTROLLER" -- update_indexes \
        --caller "$ALICE_ADDR" --assets "$(hub_vec "$PRIMARY_HUB_ID" "$XLM_SAC")" >/dev/null
    # update_account_threshold (update positions risk): recompute thresholds for the
    # admin seed account. Gated (no `|| true`) — a failure is a suite failure.
    inv update_account_threshold "$ADMIN" "$CONTROLLER" -- update_account_threshold \
        --caller "$ADMIN_ADDR" --has_risks false \
        --account_ids "[${ADMIN_ACCT:-1}]" >/dev/null
    inv update_account_threshold "$ALICE" "$CONTROLLER" -- update_account_threshold \
        --caller "$ALICE_ADDR" --has_risks false --account_ids "[${ADMIN_ACCT:-1}]" >/dev/null

    # Revenue: rewards in, revenue out (permissionless; caller must sign). Admin's USDC is spent
    # on seeding by this point — top up from carol for the reward deposit.
    sac_transfer "$CAROL" "$USDC_SAC" "$CAROL_ADDR" "$ADMIN_ADDR" 20000000 fund_admin_rewards
    local pool_rev_before
    pool_rev_before=$(_view_pool_int pool_revenue_pre get_revenue --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    inv add_rewards "$ADMIN" "$CONTROLLER" -- add_rewards \
        --caller "$ADMIN_ADDR" --rewards "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 10000000)" >/dev/null
    inv claim_revenue "$ADMIN" "$CONTROLLER" -- claim_revenue \
        --caller "$ADMIN_ADDR" --assets "$(hub_vec "$PRIMARY_HUB_ID" "$USDC_SAC")" >/dev/null
    assert_pool_revenue_decreased pool_revenue_post "$USDC_SAC" "${pool_rev_before:-0}"
    inv claim_revenue "$ALICE" "$CONTROLLER" -- claim_revenue \
        --caller "$ALICE_ADDR" --assets "$(hub_vec "$PRIMARY_HUB_ID" "$USDC_SAC")" >/dev/null
    view pool_rates_view "$POOL" -- get_borrow_rate --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" >/dev/null
    view pool_util_view "$POOL" -- get_utilisation --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" >/dev/null

    # Spoke admin lifecycle on a throwaway spoke (asset ops use idle EURC, which is
    # already listed on the primary spoke). Spoke creation takes no args; risk-bound
    # validation happens when an asset joins.
    local tmp_cat
    tmp_cat=$(inv spoke_tmp_add "$ADMIN" "$CONTROLLER" -- add_spoke | tr -d '"')
    inv spoke_tmp_add_asset "$ADMIN" "$CONTROLLER" -- add_asset_to_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$EURC_SAC" "$tmp_cat" true true 8000 8500 300)" >/dev/null
    # validate_risk_bounds on spoke assets (#113 when ltv >= threshold).
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

    # min_borrow_collateral_usd floor (update limits): set -> read-back -> blocks a
    # below-floor borrow (#126) -> reset -> read-back -> negative-floor reject (#116).
    # The reset always runs (no `set -e`), so a stale floor never leaks to later flows.
    local bob_minb_acct
    bob_minb_acct=$(inv minb_supply "$BOB" "$CONTROLLER" -- supply \
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

# Controller view and delegation coverage on a live account.
view pool_address_view "$CONTROLLER" -- get_pool_address >/dev/null
view market_index_xlm "$CONTROLLER" -- get_market_index \
--hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" >/dev/null
view total_borrow_bob_minb "$CONTROLLER" -- get_total_borrow_usd \
--account_id "$bob_minb_acct" >/dev/null
view max_supply_bob_minb "$CONTROLLER" -- max_supply \
--account_id "$bob_minb_acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" >/dev/null
view max_withdraw_bob_minb "$CONTROLLER" -- max_withdraw \
--account_id "$bob_minb_acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" >/dev/null
view max_borrow_bob_minb "$CONTROLLER" -- max_borrow \
--account_id "$bob_minb_acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" >/dev/null
inv manager_activate_alice "$ADMIN" "$CONTROLLER" -- set_position_manager \
--manager "$ALICE_ADDR" --is_active true >/dev/null
inv delegate_add_alice "$BOB" "$CONTROLLER" -- add_delegate \
--caller "$BOB_ADDR" --account_id "$bob_minb_acct" --delegate "$ALICE_ADDR" >/dev/null
inv delegated_borrow_usdc "$ALICE" "$CONTROLLER" -- borrow \
--caller "$ALICE_ADDR" --account_id "$bob_minb_acct" \
--borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 1000000)" --to null >/dev/null
inv delegate_remove_alice "$BOB" "$CONTROLLER" -- remove_delegate \
--caller "$BOB_ADDR" --account_id "$bob_minb_acct" --delegate "$ALICE_ADDR" >/dev/null
xfail delegated_borrow_removed 'Error\(Contract' "$ALICE" "$CONTROLLER" -- borrow \
--caller "$ALICE_ADDR" --account_id "$bob_minb_acct" \
--borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 1000000)" --to null
inv manager_deactivate_alice "$ADMIN" "$CONTROLLER" -- set_position_manager \
--manager "$ALICE_ADDR" --is_active false >/dev/null

# Blend allow-list coverage. Real migration is opt-in because it moves caller's
# live Blend position; absence of a position is environment, not refactor, risk.
local blend_pool
blend_pool=$(jq -r '.pools[0].address // empty' "$REPO_ROOT/configs/$NETWORK/blend.json")
if [ -n "$blend_pool" ] && [ "$blend_pool" != "null" ]; then
view blend_pool_initial "$CONTROLLER" -- is_blend_pool_approved --pool "$blend_pool" >/dev/null
inv blend_pool_approve "$ADMIN" "$CONTROLLER" -- approve_blend_pool --pool "$blend_pool" >/dev/null
view blend_pool_true "$CONTROLLER" -- is_blend_pool_approved --pool "$blend_pool" >/dev/null
inv blend_pool_revoke "$ADMIN" "$CONTROLLER" -- revoke_blend_pool --pool "$blend_pool" >/dev/null
view blend_pool_false "$CONTROLLER" -- is_blend_pool_approved --pool "$blend_pool" >/dev/null
inv blend_pool_reapprove "$ADMIN" "$CONTROLLER" -- approve_blend_pool --pool "$blend_pool" >/dev/null
if [ "${BLEND_MIGRATION_LIVE:-0}" = "1" ]; then
inv migrate_blend_live "$ALICE" "$CONTROLLER" -- migrate_from_blend \
--caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" --hub_id "$PRIMARY_HUB_ID" \
--blend_pool "$blend_pool" \
--collateral_assets "${BLEND_MIGRATE_COLLATERAL_ASSETS_JSON:-[]}" \
--supply_assets "${BLEND_MIGRATE_SUPPLY_ASSETS_JSON:-[]}" \
--debt_caps "${BLEND_MIGRATE_DEBT_CAPS_JSON:-[]}" >/dev/null
else
record migrate_blend_live environment-blocked migrate_from_blend "" "" "" "" "" "set BLEND_MIGRATION_LIVE=1 with real Blend position assets"
fi
fi

    # Secondary hub smoke: same asset can be listed and used independently by
    # explicit HubAssetKey, with no hub-0 listing assumption.
    create_market XLM_SECONDARY "$SECONDARY_HUB_ID" "$XLM_SAC" 7 \
        "$(oracle_cfg_reflector XLM 1000000000000000 100000000000000000000)" \
        "$(asset_config_json 7000 7500 1000)"
    view market_index_secondary_xlm "$CONTROLLER" -- get_market_index \
        --hub_asset "$(hub_key "$SECONDARY_HUB_ID" "$XLM_SAC")" >/dev/null
    local secondary_acct
    secondary_acct=$(inv secondary_hub_supply "$CAROL" "$CONTROLLER" -- supply \
        --caller "$CAROL_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$SECONDARY_HUB_ID" "$XLM_SAC" 1000000000)" | tr -d '"')
    assert_bool_view secondary_account_exists true account_exists --account_id "$secondary_acct"

    # Oracle circuit-breaker: owner disables a market (clears its oracle config);
    # re-disable rejects (#12 PairNotActive), which proves the first disable landed
    # (market status is no longer a discrete view). disable_token_oracle is
    # owner-only, so a non-owner caller can't satisfy the owner's require_auth() and
    # the CLI reports a missing signing key (same shape as oracle_tol_owner_guard).
    inv disable_oracle "$ADMIN" "$CONTROLLER" -- disable_token_oracle \
        --asset "$EURC_SAC" >/dev/null
    xfail disable_non_active 'Error\(Contract, #12\)' "$ADMIN" "$CONTROLLER" -- disable_token_oracle \
        --asset "$EURC_SAC"
    xfail disable_non_owner 'Missing signing key' "$ALICE" "$CONTROLLER" -- disable_token_oracle \
        --asset "$USDC_SAC"

    # Token approval admin (idle EURC: revoke then re-approve round-trip).
    inv revoke_token_admin "$ADMIN" "$CONTROLLER" -- revoke_token --token "$EURC_SAC" >/dev/null
    inv approve_token_again "$ADMIN" "$CONTROLLER" -- approve_token --token "$EURC_SAC" >/dev/null
}

# Upgrade / migrate / ownership — LAST block of the run.
flow_admin_upgrade() {
    phase admin_upgrade
    local ctrl_hash out_f="$LOG_DIR/upload_ctrl.out" err_f="$LOG_DIR/upload_ctrl.err"
    stellar contract upload --wasm "$WASM_DIR/controller.wasm" \
        --source "$ADMIN" --network "$NETWORK" >"$out_f" 2>"$err_f" || true
    ctrl_hash=$(tr -d '"\n' < "$out_f")
    if [ -n "$ctrl_hash" ]; then
        record upload_controller_wasm ok upload \
            "$(grep -oE 'Signing transaction: [0-9a-f]{64}' "$err_f" | tail -1 | awk '{print $3}')" \
            "" "" "" "" "$ctrl_hash"
 inv pool_upgrade "$ADMIN" "$CONTROLLER" -- upgrade_pool --new_wasm_hash "$POOL_HASH" >/dev/null
 view pool_address_after_pool_upgrade "$CONTROLLER" -- get_pool_address >/dev/null
 inv controller_upgrade "$ADMIN" "$CONTROLLER" -- upgrade --new_wasm_hash "$ctrl_hash" >/dev/null
        # upgrade() pauses the protocol; user operations revert until unpause.
        xfail upgraded_paused_gate 'Error\(Contract, #1000\)' "$ALICE" "$CONTROLLER" -- supply \
            --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 1000000000)"
        local ver
        ver=$(view app_version_view "$CONTROLLER" -- get_app_version | tr -d '"')
        inv controller_migrate "$ADMIN" "$CONTROLLER" -- migrate --new_version $((ver + 1)) >/dev/null
        inv unpause_after_upgrade "$ADMIN" "$CONTROLLER" -- unpause >/dev/null
    fi

    # Two-step ownership transfer round-trip (admin → carol → admin).
    local ledger
    ledger=$(curl -s -m 30 -X POST "$RPC_URL" -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' | jq -r '.result.sequence')
    inv ownership_transfer "$ADMIN" "$CONTROLLER" -- transfer_ownership \
        --new_owner "$CAROL_ADDR" --live_until_ledger $((ledger + 1000)) >/dev/null
    inv ownership_accept "$CAROL" "$CONTROLLER" -- accept_ownership >/dev/null
    inv ownership_transfer_back "$CAROL" "$CONTROLLER" -- transfer_ownership \
        --new_owner "$ADMIN_ADDR" --live_until_ledger $((ledger + 1000)) >/dev/null
    inv ownership_accept_back "$ADMIN" "$CONTROLLER" -- accept_ownership >/dev/null
}
