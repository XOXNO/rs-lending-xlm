# Admin / owner / keeper endpoint coverage: pause gates, position limits,
# params, oracle admin, revenue, keeper ops, e-mode admin lifecycle, and the
# upgrade / migrate / ownership round-trip.
#
# Self-contained on the real markets created by the lifecycle phase (XLM / USDC
# / the otherwise-idle EURC, used as the throwaway config/oracle/disable target)
# plus ADMIN_ACCT from flow_seed_liquidity — NO dependency on the mock-market
# liquidation phase, so this flow runs in its own parallel lane.
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

    # Market param + asset config edits on EURC (idle real market: edits here
    # disturb nothing else, and it is disabled at the end of this flow).
    inv update_pool_params "$ADMIN" "$CONTROLLER" -- upgrade_liquidity_pool_params \
        --asset "$EURC_SAC" --params "$(market_params_json "$EURC_SAC" 7 | jq -c 'del(.asset_id, .asset_decimals) | .reserve_factor_bps=1500')" >/dev/null
    inv edit_asset_config_admin "$ADMIN" "$CONTROLLER" -- edit_asset_config \
        --asset "$EURC_SAC" --cfg "$(asset_config_json 6500 7000 900)" >/dev/null
    # Read-back: the edit must land (LTV / threshold / bonus parsed from storage).
    assert_market_field market_cfg_ltv "$EURC_SAC" loan_to_value_bps 6500
    assert_market_field market_cfg_thr "$EURC_SAC" liquidation_threshold_bps 7000
    assert_market_field market_cfg_bonus "$EURC_SAC" liquidation_bonus_bps 900
    # validate_risk_bounds: threshold must exceed LTV (#113 when ltv >= threshold).
    xfail asset_cfg_bad_bounds 'Error\(Contract, #113\)' "$ADMIN" "$CONTROLLER" -- edit_asset_config \
        --asset "$EURC_SAC" --cfg "$(asset_config_json 9000 7000 900)"

    # Oracle tolerance: governance resolves the BPS inputs into the 4 ratio bands
    # (resolve_oracle_tolerance view), then the owner-only setter stores them.
    local tol_bands
    tol_bands=$(view oracle_tol_resolve "$GOVERNANCE" -- resolve_oracle_tolerance \
        --first_tolerance 300 --last_tolerance 600)
    inv set_oracle_tolerance "$ADMIN" "$CONTROLLER" -- set_oracle_tolerance \
        --asset "$EURC_SAC" --tolerance "$tol_bands" >/dev/null
    # Owner-gated: a non-owner caller fails the owner.require_auth() (host Auth error).
    xfail oracle_tol_owner_guard 'Error\(Auth' "$ALICE" "$CONTROLLER" -- set_oracle_tolerance \
        --asset "$EURC_SAC" --tolerance "$tol_bands"

    # Keeper ops (KEEPER role; granted to admin at construct).
    inv update_indexes "$ADMIN" "$CONTROLLER" -- update_indexes \
        --caller "$ADMIN_ADDR" --assets "[\"$XLM_SAC\",\"$USDC_SAC\",\"$EURC_SAC\"]" >/dev/null
    xfail update_indexes_non_keeper 'Error\(Contract, #2000\)' "$ALICE" "$CONTROLLER" -- update_indexes \
        --caller "$ALICE_ADDR" --assets "[\"$XLM_SAC\"]"
    # update_account_threshold (update positions risk): recompute thresholds for the
    # admin seed account. Gated (no `|| true`) — a failure is a suite failure.
    inv update_account_threshold "$ADMIN" "$CONTROLLER" -- update_account_threshold \
        --caller "$ADMIN_ADDR" --has_risks false \
        --account_ids "[${ADMIN_ACCT:-1}]" >/dev/null
    xfail uat_non_keeper 'Error\(Contract, #2000\)' "$ALICE" "$CONTROLLER" -- update_account_threshold \
        --caller "$ALICE_ADDR" --has_risks false --account_ids "[${ADMIN_ACCT:-1}]"

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
    # Role guard: only REVENUE may claim (#2000 for a non-REVENUE caller).
    xfail claim_revenue_non_role 'Error\(Contract, #2000\)' "$ALICE" "$CONTROLLER" -- claim_revenue \
        --caller "$ALICE_ADDR" --assets "[\"$USDC_SAC\"]"
    view pool_rates_view "$POOL" -- borrow_rate --asset "$USDC_SAC" >/dev/null
    view pool_util_view "$POOL" -- capital_utilisation --asset "$USDC_SAC" >/dev/null

    # E-mode admin lifecycle on a throwaway category (asset ops use idle EURC).
    local tmp_cat
    tmp_cat=$(inv emode_tmp_add "$ADMIN" "$CONTROLLER" -- add_e_mode_category \
        --ltv 8000 --threshold 8500 --bonus 300 | tr -d '"')
    # validate_risk_bounds on e-mode categories too (#113 when ltv >= threshold).
    xfail emode_bad_bounds 'Error\(Contract, #113\)' "$ADMIN" "$CONTROLLER" -- add_e_mode_category \
        --ltv 8600 --threshold 8500 --bonus 300
    inv emode_tmp_edit "$ADMIN" "$CONTROLLER" -- edit_e_mode_category \
        --id "$tmp_cat" --ltv 8100 --threshold 8600 --bonus 250 >/dev/null
    inv emode_tmp_add_asset "$ADMIN" "$CONTROLLER" -- add_asset_to_e_mode_category \
        --asset "$EURC_SAC" --category_id "$tmp_cat" --can_collateral true --can_borrow true >/dev/null
    inv emode_tmp_edit_asset "$ADMIN" "$CONTROLLER" -- edit_asset_in_e_mode_category \
        --asset "$EURC_SAC" --category_id "$tmp_cat" --can_collateral true --can_borrow false >/dev/null
    inv emode_tmp_remove_asset "$ADMIN" "$CONTROLLER" -- remove_asset_from_e_mode \
        --asset "$EURC_SAC" --category_id "$tmp_cat" >/dev/null
    inv emode_tmp_deprecate "$ADMIN" "$CONTROLLER" -- remove_e_mode_category --id "$tmp_cat" >/dev/null
    xfail emode_deprecated_supply 'Error\(Contract, #301\)' "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --e_mode_category "$tmp_cat" \
        --assets "$(pay_vec "$XLM_SAC" 1000000000)"

    # min_borrow_collateral_usd floor (update limits): set -> read-back -> blocks a
    # below-floor borrow (#126) -> reset -> read-back -> negative-floor reject (#116).
    # The reset always runs (no `set -e`), so a stale floor never leaks to later flows.
    local bob_minb_acct
    bob_minb_acct=$(inv minb_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --e_mode_category 0 \
        --assets "$(pay_vec "$XLM_SAC" 5000000000)" | tr -d '"')
    inv minb_set_high "$ADMIN" "$CONTROLLER" -- set_min_borrow_collateral_usd \
        --floor_wad 1000000000000000000000000000000000 >/dev/null
    assert_int_view_eq minb_read_high 1000000000000000000000000000000000 get_min_borrow_collateral_usd
    xfail minb_borrow_blocked 'Error\(Contract, #126\)' "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$bob_minb_acct" \
        --borrows "$(pay_vec "$USDC_SAC" 1000000)"
    inv minb_reset "$ADMIN" "$CONTROLLER" -- set_min_borrow_collateral_usd --floor_wad 0 >/dev/null
    assert_int_view_eq minb_read_zero 0 get_min_borrow_collateral_usd
    xfail minb_negative 'Error\(Contract, #116\)' "$ADMIN" "$CONTROLLER" -- set_min_borrow_collateral_usd \
        --floor_wad=-1

    # Oracle circuit-breaker: ORACLE role disables a market (Active -> Disabled);
    # re-disable rejects (#12 PairNotActive), non-ORACLE caller rejects (#2000).
    # EURC is unused after this point, so it stays disabled; the role-guard
    # negative targets active USDC (it never disables — the role check rejects first).
    inv disable_oracle "$ADMIN" "$CONTROLLER" -- disable_token_oracle \
        --caller "$ADMIN_ADDR" --asset "$EURC_SAC" >/dev/null
    assert_market_status disable_status "$EURC_SAC" Disabled
    xfail disable_non_active 'Error\(Contract, #12\)' "$ADMIN" "$CONTROLLER" -- disable_token_oracle \
        --caller "$ADMIN_ADDR" --asset "$EURC_SAC"
    xfail disable_non_oracle 'Error\(Contract, #2000\)' "$ALICE" "$CONTROLLER" -- disable_token_oracle \
        --caller "$ALICE_ADDR" --asset "$USDC_SAC"

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
