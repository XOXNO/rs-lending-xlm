# Admin / owner / keeper endpoint coverage: pause gates, position limits,
# params, oracle admin, revenue, keeper ops, e-mode admin lifecycle, and the
# upgrade / migrate / ownership round-trip.
#
# Ordering: upgrade() PAUSES the protocol by design — run the
# upgrade/migrate/ownership block LAST, then unpause.

flow_admin() {
    phase admin
    # Pause gate: supply during pause reverts EnforcedPause (#1000).
    inv admin_pause "$ADMIN" "$CONTROLLER" -- pause >/dev/null
    xfail paused_supply 'Error\(Contract, #1000\)' "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id 0 --e_mode_category 0 \
        --assets "$(pay_vec "$XLM_SAC" 1000000000)"
    inv admin_unpause "$ADMIN" "$CONTROLLER" -- unpause >/dev/null
    xfail unpause_when_live 'Error\(Contract, #1001\)' "$ADMIN" "$CONTROLLER" -- unpause

    # Position limits: valid update + cap guard (#36 above POSITION_LIMIT_MAX=10).
    inv set_position_limits "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits '{"max_supply_positions":10,"max_borrow_positions":10}' >/dev/null
    xfail position_limits_too_high 'Error\(Contract, #36\)' "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits '{"max_supply_positions":11,"max_borrow_positions":11}'

    # Market param + asset config edits on a mock market.
    inv update_pool_params "$ADMIN" "$CONTROLLER" -- upgrade_liquidity_pool_params \
        --asset "$SAC_LIQA" --params "$(market_params_json "$SAC_LIQA" 7 | jq -c 'del(.asset_id, .asset_decimals) | .reserve_factor_bps=1500')" >/dev/null
    inv edit_asset_config_admin "$ADMIN" "$CONTROLLER" -- edit_asset_config \
        --asset "$SAC_LIQA" --cfg "$(asset_config_json 6500 7000 900)" >/dev/null
    view market_cfg_after_edit "$CONTROLLER" -- get_market_config --asset "$SAC_LIQA" >/dev/null

    # Oracle admin: tolerance edit + role guard.
    inv edit_oracle_tolerance "$ADMIN" "$CONTROLLER" -- edit_oracle_tolerance \
        --caller "$ADMIN_ADDR" --asset "$SAC_LIQA" --first_tolerance 300 --last_tolerance 600 >/dev/null
    xfail oracle_role_guard 'Error\(Contract, #2000\)' "$ALICE" "$CONTROLLER" -- edit_oracle_tolerance \
        --caller "$ALICE_ADDR" --asset "$SAC_LIQA" --first_tolerance 100 --last_tolerance 200

    # Keeper ops.
    inv update_indexes "$ADMIN" "$CONTROLLER" -- update_indexes \
        --caller "$ADMIN_ADDR" --assets "[\"$XLM_SAC\",\"$USDC_SAC\",\"$SAC_LIQA\"]" >/dev/null
    inv update_account_threshold "$ADMIN" "$CONTROLLER" -- update_account_threshold \
        --caller "$ADMIN_ADDR" --has_risks false \
        --account_ids "[${LIQ2_ACCT:-1}]" >/dev/null || true

    # Revenue: rewards in, revenue out (REVENUE role). Admin's USDC is spent
    # on seeding by this point — top up from carol for the reward deposit.
    sac_transfer "$CAROL" "$USDC_SAC" "$CAROL_ADDR" "$ADMIN_ADDR" 20000000 fund_admin_rewards
    local pool_rev_before
    pool_rev_before=$(_view_pool_int pool_revenue_pre protocol_revenue --asset "$USDC_SAC")
    inv add_rewards "$ADMIN" "$CONTROLLER" -- add_rewards \
        --caller "$ADMIN_ADDR" --rewards "$(pay_vec "$USDC_SAC" 10000000)" >/dev/null
    inv claim_revenue "$ADMIN" "$CONTROLLER" -- claim_revenue \
        --caller "$ADMIN_ADDR" --assets "[\"$USDC_SAC\"]" >/dev/null
    assert_pool_revenue_decreased pool_revenue_post "$USDC_SAC" "${pool_rev_before:-0}"
    view pool_rates_view "$POOL" -- borrow_rate --asset "$USDC_SAC" >/dev/null
    view pool_util_view "$POOL" -- capital_utilisation --asset "$USDC_SAC" >/dev/null

    # E-mode admin lifecycle on a throwaway category.
    local tmp_cat
    tmp_cat=$(inv emode_tmp_add "$ADMIN" "$CONTROLLER" -- add_e_mode_category \
        --ltv 8000 --threshold 8500 --bonus 300 | tr -d '"')
    inv emode_tmp_edit "$ADMIN" "$CONTROLLER" -- edit_e_mode_category \
        --id "$tmp_cat" --ltv 8100 --threshold 8600 --bonus 250 >/dev/null
    inv emode_tmp_add_asset "$ADMIN" "$CONTROLLER" -- add_asset_to_e_mode_category \
        --asset "$SAC_LIQD" --category_id "$tmp_cat" --can_collateral true --can_borrow true >/dev/null
    inv emode_tmp_edit_asset "$ADMIN" "$CONTROLLER" -- edit_asset_in_e_mode_category \
        --asset "$SAC_LIQD" --category_id "$tmp_cat" --can_collateral true --can_borrow false >/dev/null
    inv emode_tmp_remove_asset "$ADMIN" "$CONTROLLER" -- remove_asset_from_e_mode \
        --asset "$SAC_LIQD" --category_id "$tmp_cat" >/dev/null
    inv emode_tmp_deprecate "$ADMIN" "$CONTROLLER" -- remove_e_mode_category --id "$tmp_cat" >/dev/null
    xfail emode_deprecated_supply 'Error\(Contract, #301\)' "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --e_mode_category "$tmp_cat" \
        --assets "$(pay_vec "$SAC_LIQA" $((100 * LIQ_UNIT)))"

    # Token approval admin.
    inv revoke_token_admin "$ADMIN" "$CONTROLLER" -- revoke_token --token "$SAC_LIQG" >/dev/null
    inv approve_token_again "$ADMIN" "$CONTROLLER" -- approve_token --token "$SAC_LIQG" >/dev/null
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
        inv controller_upgrade "$ADMIN" "$CONTROLLER" -- upgrade --new_wasm_hash "$ctrl_hash" >/dev/null
        # upgrade() pauses by design — every user op now reverts until unpause.
        xfail upgraded_paused_gate 'Error\(Contract, #1000\)' "$ALICE" "$CONTROLLER" -- supply \
            --caller "$ALICE_ADDR" --account_id 0 --e_mode_category 0 \
            --assets "$(pay_vec "$XLM_SAC" 1000000000)"
        local ver
        ver=$(view app_version_view "$CONTROLLER" -- app_version | tr -d '"')
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
