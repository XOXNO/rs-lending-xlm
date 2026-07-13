# DeFindex strategy adapter (contracts/defindex-strategy) e2e on top of the
# controller. One strategy WASM is deployed per underlying asset; each vault
# (an EOA here, standing in for a DeFindex vault) maps to one controller
# account the strategy owns. Flows: deposit -> controller.supply, withdraw ->
# controller.withdraw, balance -> get_collateral_amount, harvest ->
# price_per_share from the supply index.
#
# Venue-free (no aggregator/oracle pricing): the position is debt-free, so
# supply/withdraw are oracle-independent. Runs on its own dedicated, stable
# mock market (DFX) so it is isolated from the liquidation price crashes.

DFX_UNIT=10000000   # 1.0 token at 7 decimals

# Strategy-targeted view helpers (the shared assert.sh helpers target $CONTROLLER).
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

# Deploys the strategy bound to SAC_DFX. init_args is a Vec<Val> = [controller];
# the CLI takes a Vec<Val> as a JSON array of ScVal elements (address form).
deploy_dfx_strategy() {
    [ -n "${STRATEGY:-}" ] && return 0
    local out_f="$LOG_DIR/deploy_strategy.out" err_f="$LOG_DIR/deploy_strategy.err"
    stellar contract deploy --wasm "$WASM_DIR/defindex_strategy.wasm" \
        --source "$ADMIN" --network "$NETWORK" \
        -- --asset "$SAC_DFX" --init_args "[{\"address\":\"$CONTROLLER\"}]" \
        >"$out_f" 2>"$err_f"
    local strat txh
    strat=$(sanitize_output "$out_f")
    txh=$(extract_signing_hash "$err_f")
    [ -z "$strat" ] && { log "strategy deploy failed: $(tail_err_note "$err_f" 200)"; return 1; }
    save_state STRATEGY "$strat"
    record deploy_defindex_strategy ok deploy "$txh" "" "" "" "" "$strat"
    log "defindex strategy = $strat"
}

flow_defindex_strategy() {
    phase defindex
    # Dedicated stable ($1) mock market + funded vault (DAVE is otherwise unused
    # in this lane). The strategy only supplies/withdraws, so no debt-side seed.
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

    # Configured underlying. The vault's lending-account lifecycle is not exposed
    # by public strategy views, so the checks below are balance-based.
    assert_dfx_eq dfx_asset "$SAC_DFX" asset

    # Deposit guard: non-positive amount (#460 AmountNotPositive).
    xfail dfx_deposit_zero 'Error\(Contract, #460\)' "$DAVE" "$STRATEGY" -- deposit \
        --amount 0 --from "$DAVE_ADDR"

    # Deposit -> controller.supply. Returns the vault's collateral balance.
    local deposit=$((1000 * DFX_UNIT)) reported
    reported=$(inv dfx_deposit "$DAVE" "$STRATEGY" -- deposit \
        --amount "$deposit" --from "$DAVE_ADDR" | tr -d '"') || return 1
    log "deposit reported balance = $reported"
    assert_dfx_uint_ge dfx_balance_post_deposit "$reported" balance --from "$DAVE_ADDR"

    # Harvest publishes price_per_share from the supply index (no auth, no debt).
    inv dfx_harvest "$DAVE" "$STRATEGY" -- harvest --from "$DAVE_ADDR" >/dev/null || return 1

    # Withdraw guards (all revert before any transfer; no state mutation).
    xfail dfx_withdraw_zero 'Error\(Contract, #460\)' "$DAVE" "$STRATEGY" -- withdraw \
        --amount 0 --from "$DAVE_ADDR" --to "$DAVE_ADDR"
    xfail dfx_withdraw_over 'Error\(Contract, #461\)' "$DAVE" "$STRATEGY" -- withdraw \
        --amount $((reported * 2 + DFX_UNIT)) --from "$DAVE_ADDR" --to "$DAVE_ADDR"
    # No mapped account for CAROL (never deposited) -> InsufficientBalance (#461).
    xfail dfx_withdraw_no_pos 'Error\(Contract, #461\)' "$CAROL" "$STRATEGY" -- withdraw \
        --amount "$DFX_UNIT" --from "$CAROL_ADDR" --to "$CAROL_ADDR"

    # Partial withdraw pays the recipient directly; balance drops.
    local part=$((300 * DFX_UNIT))
    inv dfx_withdraw_partial "$DAVE" "$STRATEGY" -- withdraw \
        --amount "$part" --from "$DAVE_ADDR" --to "$DAVE_ADDR" >/dev/null || return 1
    assert_dfx_uint_lt dfx_balance_post_partial "$reported" balance --from "$DAVE_ADDR"

    # Terminal exit: amount == balance maps to controller withdraw-all (0), which
    # closes + deregisters the account; balance reports 0.
    local remaining
    remaining=$(_dfx_view dfx_balance_pre_full balance --from "$DAVE_ADDR")
    inv dfx_withdraw_full "$DAVE" "$STRATEGY" -- withdraw \
        --amount "$remaining" --from "$DAVE_ADDR" --to "$DAVE_ADDR" >/dev/null || return 1
    assert_dfx_eq dfx_balance_closed 0 balance --from "$DAVE_ADDR"

    # Re-deposit after a full exit opens a fresh account; balance is positive again.
    inv dfx_redeposit "$DAVE" "$STRATEGY" -- deposit \
        --amount $((500 * DFX_UNIT)) --from "$DAVE_ADDR" >/dev/null || return 1
    assert_dfx_uint_ge dfx_balance_reopened 1 balance --from "$DAVE_ADDR"
}
