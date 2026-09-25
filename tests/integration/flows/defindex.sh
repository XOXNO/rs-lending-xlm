DFX_UNIT=10000000

_dfx_view() { view "$1" "$STRATEGY" -- "${@:2}" | tr -d '"' | tr -d '[:space:]'; }

assert_dfx_eq() {
    local label="$1" want="$2"; shift 2
    local got; got=$(_dfx_view "$label" "$@")
    [ "$got" = "$want" ] || _assert_fail "$label" "got '$got' want '$want'"
}

assert_dfx_uint_ge() {
    local label="$1" min="$2"; shift 2
    local got; got=$(_dfx_view "$label" "$@")
    _uint_ge "$got" "$min" || _assert_fail "$label" "got '$got' want >= $min"
}

assert_dfx_uint_lt() {
    local label="$1" max="$2"; shift 2
    local got; got=$(_dfx_view "$label" "$@")
    _uint_lt "$got" "$max" || _assert_fail "$label" "got '$got' want < $max"
}

deploy_dfx_strategy() {
    [ -n "${STRATEGY:-}" ] && return 0
    local out_f="$LOG_DIR/deploy_strategy.out" err_f="$LOG_DIR/deploy_strategy.err"
    # init_args is [controller, hub_id, spoke_id]; the constructor validates
    # the (hub, asset) market exists, so the DFX market must be listed first.
    run_deploy "$out_f" "$err_f" -- stellar contract deploy --wasm "$WASM_DIR/defindex_strategy.wasm" \
        --source "$ADMIN" "${NET_ARGS[@]}" \
        -- --asset "$SAC_DFX" \
        --init_args "[{\"address\":\"$CONTROLLER\"},{\"u32\":$PRIMARY_HUB_ID},{\"u32\":$PRIMARY_SPOKE_ID}]" \
        || return 1
    local strat txh
    strat=$(sanitize_output "$out_f")
    txh=$(extract_signing_hash "$err_f")
    # A FAIL row, not just a log line: this deploy is raw CLI, so nothing else
    # records the failure and the lane would pass with the strategy surface
    # skipped.
    [ -z "$strat" ] && { _assert_fail deploy_defindex_strategy "strategy deploy failed: $(tail_err_note "$err_f" 200)"; return 1; }
    save_state STRATEGY "$strat"
    record deploy_defindex_strategy ok deploy "$txh" "" "" "" "" "$strat" deployment "$strat"
    log "defindex strategy = $strat"
}

flow_defindex_strategy() {
    phase defindex

    if [ -z "${DFX_SETUP_DONE:-}" ]; then
        deploy_mock_reflector
        issue_sac SAC_DFX DFX
        trustline "$DAVE" DFX "$ADMIN_ADDR"
        mint_to "$SAC_DFX" DFX "$DAVE_ADDR" $((100000 * DFX_UNIT))
        set_mock_price "$SAC_DFX" "$WAD" px_init_DFX
        create_market DFX "$PRIMARY_HUB_ID" "$SAC_DFX" 7 "$(oracle_cfg_mock_single "$SAC_DFX")" \
            "$(asset_config_json 7000 7500 800)" || return 1
        save_state DFX_SETUP_DONE 1
    fi
    deploy_dfx_strategy || return 1

    assert_dfx_eq dfx_asset "$SAC_DFX" asset

    xfail dfx_deposit_zero 'Error\(Contract, #460\)' "$DAVE" "$STRATEGY" -- deposit \
        --amount 0 --from "$DAVE_ADDR"

    trustline "$CAROL" DFX "$ADMIN_ADDR" || return 1
    mint_to "$SAC_DFX" DFX "$CAROL_ADDR" "$((1000 * DFX_UNIT))" || return 1
    local deposit=$((1000 * DFX_UNIT)) reported payer pool_before strategy_before controller_before
    payer=$(balance "$SAC_DFX" "$DAVE_ADDR") || return 1
    pool_before=$(balance "$SAC_DFX" "$POOL") || return 1
    strategy_before=$(balance "$SAC_DFX" "$STRATEGY") || return 1
    controller_before=$(balance "$SAC_DFX" "$CONTROLLER") || return 1
    reported=$(inv dfx_deposit "$DAVE" "$STRATEGY" -- deposit \
        --amount "$deposit" --from "$DAVE_ADDR" | tr -d '"') || return 1
    # No debt in DFX: the supply index is exactly RAY, hence zero tolerance.
    assert_raw_within dfx_expected_credit "$reported" "$deposit" 0 || return 1
    assert_dfx_eq dfx_balance_post_deposit "$deposit" balance --from "$DAVE_ADDR" || return 1
    assert_delta dfx_deposit_payer "$payer" "$(balance "$SAC_DFX" "$DAVE_ADDR")" "-$deposit" || return 1
    assert_delta dfx_deposit_pool "$pool_before" "$(balance "$SAC_DFX" "$POOL")" "$deposit" || return 1
    assert_delta dfx_deposit_strategy "$strategy_before" "$(balance "$SAC_DFX" "$STRATEGY")" 0 || return 1
    assert_delta dfx_deposit_controller "$controller_before" "$(balance "$SAC_DFX" "$CONTROLLER")" 0 || return 1
    inv dfx_second_vault "$CAROL" "$STRATEGY" -- deposit --amount "$((100 * DFX_UNIT))" --from "$CAROL_ADDR" >/dev/null || return 1
    assert_dfx_eq dfx_vault_isolation "$deposit" balance --from "$DAVE_ADDR" || return 1

    inv dfx_harvest "$DAVE" "$STRATEGY" -- harvest --from "$DAVE_ADDR" --data null >/dev/null || return 1
    assert_dfx_eq dfx_harvest_credit "$deposit" balance --from "$DAVE_ADDR" || return 1
    assert_delta dfx_harvest_payer "$(raw_sub "$payer" "$deposit")" "$(balance "$SAC_DFX" "$DAVE_ADDR")" 0 || return 1
    xfail dfx_unauthorized "Missing signing key for account $DAVE_ADDR" "$BOB" "$STRATEGY" -- withdraw --amount "$DFX_UNIT" --from "$DAVE_ADDR" --to "$DAVE_ADDR" || return 1

    xfail dfx_withdraw_zero 'Error\(Contract, #460\)' "$DAVE" "$STRATEGY" -- withdraw \
        --amount 0 --from "$DAVE_ADDR" --to "$DAVE_ADDR"
    xfail dfx_withdraw_over 'Error\(Contract, #461\)' "$DAVE" "$STRATEGY" -- withdraw \
        --amount $((reported * 2 + DFX_UNIT)) --from "$DAVE_ADDR" --to "$DAVE_ADDR"
    xfail dfx_withdraw_no_pos 'Error\(Contract, #461\)' "$BOB" "$STRATEGY" -- withdraw \
        --amount "$DFX_UNIT" --from "$BOB_ADDR" --to "$BOB_ADDR"

    local part=$((300 * DFX_UNIT)) recipient
    recipient=$(balance "$SAC_DFX" "$CAROL_ADDR") || return 1
    pool_before=$(balance "$SAC_DFX" "$POOL") || return 1
    inv dfx_withdraw_partial "$DAVE" "$STRATEGY" -- withdraw \
        --amount "$part" --from "$DAVE_ADDR" --to "$CAROL_ADDR" >/dev/null || return 1
    assert_dfx_eq dfx_balance_post_partial "$(raw_sub "$deposit" "$part")" balance --from "$DAVE_ADDR" || return 1
    assert_delta dfx_partial_recipient "$recipient" "$(balance "$SAC_DFX" "$CAROL_ADDR")" "$part" || return 1
    assert_delta dfx_partial_pool "$pool_before" "$(balance "$SAC_DFX" "$POOL")" "-$part" || return 1
    assert_dfx_eq dfx_other_vault_unchanged "$((100 * DFX_UNIT))" balance --from "$CAROL_ADDR" || return 1

    local remaining
    remaining=$(_dfx_view dfx_balance_pre_full balance --from "$DAVE_ADDR") || return 1
    assert_raw_within dfx_expected_remaining "$remaining" "$(raw_sub "$deposit" "$part")" 0 || return 1
    payer=$(balance "$SAC_DFX" "$DAVE_ADDR") || return 1
    pool_before=$(balance "$SAC_DFX" "$POOL") || return 1
    inv dfx_withdraw_full "$DAVE" "$STRATEGY" -- withdraw \
        --amount "$remaining" --from "$DAVE_ADDR" --to "$DAVE_ADDR" >/dev/null || return 1
    assert_dfx_eq dfx_balance_closed 0 balance --from "$DAVE_ADDR" || return 1
    assert_delta dfx_full_recipient "$payer" "$(balance "$SAC_DFX" "$DAVE_ADDR")" "$remaining" || return 1
    assert_delta dfx_full_pool "$pool_before" "$(balance "$SAC_DFX" "$POOL")" "-$remaining" || return 1
    recipient=$(balance "$SAC_DFX" "$CAROL_ADDR") || return 1
    pool_before=$(balance "$SAC_DFX" "$POOL") || return 1
    inv dfx_second_close "$CAROL" "$STRATEGY" -- withdraw --amount "$((100 * DFX_UNIT))" --from "$CAROL_ADDR" --to "$CAROL_ADDR" >/dev/null || return 1
    assert_dfx_eq dfx_second_closed 0 balance --from "$CAROL_ADDR" || return 1
    assert_delta dfx_second_recipient "$recipient" "$(balance "$SAC_DFX" "$CAROL_ADDR")" "$((100 * DFX_UNIT))" || return 1
    assert_delta dfx_second_pool "$pool_before" "$(balance "$SAC_DFX" "$POOL")" "-$((100 * DFX_UNIT))" || return 1

    inv dfx_redeposit "$DAVE" "$STRATEGY" -- deposit \
        --amount $((500 * DFX_UNIT)) --from "$DAVE_ADDR" >/dev/null || return 1
    assert_dfx_uint_ge dfx_balance_reopened 1 balance --from "$DAVE_ADDR" || return 1
    flow_defindex_contract_vault
}

# A contract address owns this vault; Dave authorizes the caller contract only.
# The caller's direct invocation authorizes strategy access and its exact nested
# SAC transfer entry authorizes the strategy to pull the deposit.
flow_defindex_contract_vault() {
    local out_f="$LOG_DIR/dfx_vault.out" err_f="$LOG_DIR/dfx_vault.err" runner txh
    run_deploy "$out_f" "$err_f" -- stellar contract deploy \
        --source "$ADMIN" "${NET_ARGS[@]}" --wasm "$FIXTURE_WASM_DIR/script_runner.wasm" || return 1
    runner=$(sanitize_output "$out_f")
    is_contract_id "$runner" || { _assert_fail dfx_contract_deploy "invalid vault contract id"; return 1; }
    txh=$(extract_signing_hash "$err_f")
    record dfx_contract_deploy ok deploy "$txh" "" "" "" "" "$runner" deployment "$runner"
    inv dfx_contract_configure "$DAVE" "$runner" -- configure_vault --owner "$DAVE_ADDR" >/dev/null || return 1

    local amount=$((100 * DFX_UNIT)) ops pool_before strategy_before controller_before recipient own_credit
    inv dfx_contract_fund "$ADMIN" "$SAC_DFX" -- mint --to "$runner" --amount "$amount" >/dev/null || return 1
    assert_raw_within dfx_contract_funded "$(balance "$SAC_DFX" "$runner")" "$amount" 0 || return 1
    own_credit=$(_dfx_view dfx_contract_eoa_before balance --from "$DAVE_ADDR") || return 1
    pool_before=$(balance "$SAC_DFX" "$POOL") || return 1
    strategy_before=$(balance "$SAC_DFX" "$STRATEGY") || return 1
    controller_before=$(balance "$SAC_DFX" "$CONTROLLER") || return 1
    ops=$(jq -cn --arg s "$STRATEGY" --arg amount "$amount" '[{StrategyDeposit:{strategy:$s,amount:$amount}}]')
    inv dfx_contract_deposit "$DAVE" "$runner" -- run --controller "$CONTROLLER" --nft "$POSITION_NFT" --ops "$ops" >/dev/null || return 1
    assert_dfx_eq dfx_contract_credit "$amount" balance --from "$runner" || return 1
    assert_raw_within dfx_contract_deposit_payer "$(balance "$SAC_DFX" "$runner")" 0 0 || return 1
    assert_delta dfx_contract_deposit_pool "$pool_before" "$(balance "$SAC_DFX" "$POOL")" "$amount" || return 1
    assert_delta dfx_contract_deposit_strategy "$strategy_before" "$(balance "$SAC_DFX" "$STRATEGY")" 0 || return 1
    assert_delta dfx_contract_deposit_controller "$controller_before" "$(balance "$SAC_DFX" "$CONTROLLER")" 0 || return 1

    pool_before=$(balance "$SAC_DFX" "$POOL") || return 1
    ops=$(jq -cn --arg s "$STRATEGY" '[{StrategyHarvest:$s}]')
    inv dfx_contract_harvest "$DAVE" "$runner" -- run --controller "$CONTROLLER" --nft "$POSITION_NFT" --ops "$ops" >/dev/null || return 1
    assert_dfx_eq dfx_contract_harvest_credit "$amount" balance --from "$runner" || return 1
    assert_delta dfx_contract_harvest_pool "$pool_before" "$(balance "$SAC_DFX" "$POOL")" 0 || return 1
    assert_raw_within dfx_contract_harvest_payer "$(balance "$SAC_DFX" "$runner")" 0 0 || return 1

    recipient=$(balance "$SAC_DFX" "$DAVE_ADDR") || return 1
    ops=$(jq -cn --arg s "$STRATEGY" --arg amount "$amount" --arg to "$DAVE_ADDR" '[{StrategyWithdraw:{strategy:$s,amount:$amount,to:$to}}]')
    xfail dfx_contract_unauthorized "Missing signing key for account $DAVE_ADDR" "$BOB" "$runner" -- run \
        --controller "$CONTROLLER" --nft "$POSITION_NFT" --ops "$ops" || return 1
    assert_dfx_eq dfx_contract_denied_credit "$amount" balance --from "$runner" || return 1
    assert_delta dfx_contract_denied_pool "$pool_before" "$(balance "$SAC_DFX" "$POOL")" 0 || return 1
    assert_delta dfx_contract_denied_recipient "$recipient" "$(balance "$SAC_DFX" "$DAVE_ADDR")" 0 || return 1

    inv dfx_contract_withdraw "$DAVE" "$runner" -- run --controller "$CONTROLLER" --nft "$POSITION_NFT" --ops "$ops" >/dev/null || return 1
    assert_dfx_eq dfx_contract_closed 0 balance --from "$runner" || return 1
    assert_delta dfx_contract_withdraw_recipient "$recipient" "$(balance "$SAC_DFX" "$DAVE_ADDR")" "$amount" || return 1
    assert_delta dfx_contract_withdraw_pool "$pool_before" "$(balance "$SAC_DFX" "$POOL")" "-$amount" || return 1
    assert_raw_within dfx_contract_final_payer "$(balance "$SAC_DFX" "$runner")" 0 0 || return 1
    assert_delta dfx_contract_final_strategy "$strategy_before" "$(balance "$SAC_DFX" "$STRATEGY")" 0 || return 1
    assert_delta dfx_contract_final_controller "$controller_before" "$(balance "$SAC_DFX" "$CONTROLLER")" 0 || return 1
    assert_dfx_eq dfx_contract_eoa_isolated "$own_credit" balance --from "$DAVE_ADDR"
}
