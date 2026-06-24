# Governance timelock e2e against the governance-owned controller (GOV_CONTROLLER,
# deployed by deploy_protocol). Covers the production trust model the EOA fast
# path bypasses: deploy_controller ownership, the resolver views, and the full
# propose -> await -> execute lifecycle plus cancel and a non-PROPOSER guard.
#
# Short timelock (INTEG_MIN_DELAY ledgers, ~5s/ledger) keeps the await a real but
# fast delay. The execute args are the EXACT ScVal Vec<Val> the proposer scheduled:
# derived by encoding the controller's typed setter with --build-only and
# decoding invoke_contract.args (same method as configs/script.sh executeOp).

GOV_ZERO32="0000000000000000000000000000000000000000000000000000000000000000"
GOV_SALT_CANCEL="1111111111111111111111111111111111111111111111111111111111111111"
GOV_SALT_EXEC="2222222222222222222222222222222222222222222222222222222222222222"
GOV_SALT_DENY="3333333333333333333333333333333333333333333333333333333333333333"

# Raw operation state (Unset|Waiting|Ready|Done) for an op id.
gov_state() {
    stellar contract invoke --id "$GOVERNANCE" --source "$ADMIN" --network "$NETWORK" --send=no \
        -- get_operation_state --operation_id "$1" 2>/dev/null | tr -d '"[:space:]'
}

# Asserts the timelock op state, recording a gate-visible row either way.
gov_assert_state() {
    local label="$1" op_id="$2" want="$3" got
    got=$(gov_state "$op_id")
    if [ "$got" = "$want" ]; then
        record "$label" read get_operation_state "" "" "" "" "" "state=$got"
    else
        _assert_fail "$label" "op state=$got want $want"
    fi
}

# Polls get_operation_state until Ready/Done (or times out). Echoes the final state.
gov_await_ready() {
    local op_id="$1" tries="${2:-30}" st i
    for ((i = 0; i < tries; i++)); do
        st=$(gov_state "$op_id")
        if [ "$st" = "Ready" ] || [ "$st" = "Done" ]; then echo "$st"; return 0; fi
        sleep 5
    done
    echo "$st"
    return 1
}

# Derives the scheduled ScVal Vec<Val> args for a GOV_CONTROLLER setter by
# encoding the typed invoke (--build-only) and decoding invoke_contract.args.
gov_scval_args() {
    local fn="$1"; shift
    local txb
    txb=$(stellar contract invoke --id "$GOV_CONTROLLER" --source "$ADMIN" --network "$NETWORK" \
        --build-only --send=no -- "$fn" "$@" 2>/dev/null) || return 1
    printf '%s' "$txb" | stellar tx decode \
        | jq -c 'first(.. | objects | select(has("invoke_contract")) | .invoke_contract.args)'
}

flow_governance() {
    phase governance

    # deploy_controller: governance owns GOV_CONTROLLER; a second deploy reverts (#5).
    local gov_ctrl
    gov_ctrl=$(view gov_controller_view "$GOVERNANCE" -- controller | tr -d '"[:space:]')
    if [ "$gov_ctrl" != "$GOV_CONTROLLER" ]; then
        _assert_fail gov_controller_match "controller()=$gov_ctrl want $GOV_CONTROLLER"
    fi
    xfail gov_deploy_twice 'Error\(Contract, #5\)' "$ADMIN" "$GOVERNANCE" -- deploy_controller \
        --wasm_hash "$CTRL_HASH"

    # Unpause the fresh (paused-by-default) governance-owned controller so the
    # timelocked setter is not pause-gated; then exercise the resolver views.
    inv gov_unpause "$ADMIN" "$GOVERNANCE" -- unpause >/dev/null
    view gov_resolve_tol "$GOVERNANCE" -- resolve_oracle_tolerance \
        --first_tolerance 200 --last_tolerance 500 >/dev/null

    # Propose then cancel: state moves from Waiting to Unset.
    local op_cancel
    op_cancel=$(inv gov_propose_cancel "$ADMIN" "$GOVERNANCE" -- propose_set_position_limits \
        --proposer "$ADMIN_ADDR" \
        --limits '{"max_supply_positions":6,"max_borrow_positions":6}' \
        --salt "$GOV_SALT_CANCEL" | tr -d '"[:space:]')
    gov_assert_state gov_state_waiting "$op_cancel" Waiting
    inv gov_cancel "$ADMIN" "$GOVERNANCE" -- cancel \
        --canceller "$ADMIN_ADDR" --operation_id "$op_cancel" >/dev/null
    gov_assert_state gov_state_unset "$op_cancel" Unset

    # Full lifecycle: propose -> await real delay -> execute (open, executor=None).
    local op_exec st args_f
    op_exec=$(inv gov_propose_exec "$ADMIN" "$GOVERNANCE" -- propose_set_position_limits \
        --proposer "$ADMIN_ADDR" \
        --limits '{"max_supply_positions":8,"max_borrow_positions":8}' \
        --salt "$GOV_SALT_EXEC" | tr -d '"[:space:]')
    st=$(gov_await_ready "$op_exec")
    if [ "$st" != "Ready" ] && [ "$st" != "Done" ]; then
        _assert_fail gov_await_ready "op $op_exec never reached Ready (state=$st)"
    fi
    args_f="$LOG_DIR/gov_exec_args.json"
    gov_scval_args set_position_limits \
        --limits '{"max_supply_positions":8,"max_borrow_positions":8}' > "$args_f"
    inv gov_execute "$ADMIN" "$GOVERNANCE" -- execute \
        --executor null --target "$GOV_CONTROLLER" --function set_position_limits \
        --args-file-path "$args_f" --predecessor "$GOV_ZERO32" --salt "$GOV_SALT_EXEC" >/dev/null
    gov_assert_state gov_state_done "$op_exec" Done
    # A post-execution replay of the same op reverts (already Done, #4002 pre-ready).
    xfail gov_execute_replay 'Error\(' "$ADMIN" "$GOVERNANCE" -- execute \
        --executor null --target "$GOV_CONTROLLER" --function set_position_limits \
        --args-file-path "$args_f" --predecessor "$GOV_ZERO32" --salt "$GOV_SALT_EXEC"

    # Non-PROPOSER cannot schedule (#2000 before anything is queued).
    xfail gov_propose_non_proposer 'Error\(Contract, #2000\)' "$ALICE" "$GOVERNANCE" -- propose_set_position_limits \
        --proposer "$ALICE_ADDR" \
        --limits '{"max_supply_positions":5,"max_borrow_positions":5}' \
        --salt "$GOV_SALT_DENY"

    # Owner-immediate emergency brake forwards to the governance-owned controller.
    inv gov_pause "$ADMIN" "$GOVERNANCE" -- pause >/dev/null
}
