GOV_ZERO32="0000000000000000000000000000000000000000000000000000000000000000"
GOV_SALT_CANCEL="1111111111111111111111111111111111111111111111111111111111111111"
GOV_SALT_EXEC="2222222222222222222222222222222222222222222222222222222222222222"
GOV_SALT_DENY="3333333333333333333333333333333333333333333333333333333333333333"
GOV_SALT_BADLIMITS="4444444444444444444444444444444444444444444444444444444444444444"
GOV_SALT_SELF_DELAY="5555555555555555555555555555555555555555555555555555555555555555"
GOV_SALT_BADCURVE="6666666666666666666666666666666666666666666666666666666666666666"
GOV_SALT_UNPAUSE="7777777777777777777777777777777777777777777777777777777777777777"
GOV_SALT_SELF_SENSITIVE="8888888888888888888888888888888888888888888888888888888888888888"
GOV_SALT_CANCELLER_RESET="9999999999999999999999999999999999999999999999999999999999999999"

gov_state() {
    stellar contract invoke --id "$GOVERNANCE" --source "$ADMIN" "${NET_ARGS[@]}" --send=no \
        -- get_operation_state --operation_id "$1" 2>/dev/null | tr -d '"[:space:]'
}

gov_assert_state() {
    local label="$1" op_id="$2" want="$3" got
    got=$(gov_state "$op_id")
    if [ "$got" = "$want" ]; then
        record "$label" read get_operation_state "" "" "" "" "" "state=$got"
    else
        _assert_fail "$label" "op state=$got want $want"
    fi
}

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

gov_scval_args() {
    local fn="$1"; shift
    local txb
    txb=$(stellar contract invoke --id "$GOV_CONTROLLER" --source "$ADMIN" "${NET_ARGS[@]}" \
        --build-only --send=no -- "$fn" "$@" 2>/dev/null) || return 1
    printf '%s' "$txb" | stellar tx decode \
        | jq -c 'first(.. | objects | select(has("invoke_contract")) | .invoke_contract.args)'
}

flow_governance() {
    phase governance

    local gov_ctrl
    gov_ctrl=$(view gov_controller_view "$GOVERNANCE" -- controller | tr -d '"[:space:]')
    if [ "$gov_ctrl" != "$GOV_CONTROLLER" ]; then
        _assert_fail gov_controller_match "controller()=$gov_ctrl want $GOV_CONTROLLER"
    fi
    xfail gov_deploy_twice 'Error\(Contract, #5\)' "$ADMIN" "$GOVERNANCE" -- deploy_controller \
        --wasm_hash "$CTRL_HASH"

    local op_unpause st_unpause unpause_args_f
    op_unpause=$(inv gov_propose_unpause "$ADMIN" "$GOVERNANCE" -- propose \
        --proposer "$ADMIN_ADDR" \
        --op '"Unpause"' \
        --salt "$GOV_SALT_UNPAUSE" | tr -d '"[:space:]')
    st_unpause=$(gov_await_ready "$op_unpause")
    if [ "$st_unpause" != "Ready" ] && [ "$st_unpause" != "Done" ]; then
        _assert_fail gov_await_ready_unpause "op $op_unpause never reached Ready (state=$st_unpause)"
    fi
    unpause_args_f="$LOG_DIR/gov_unpause_args.json"
    printf '[]' > "$unpause_args_f"
    inv gov_execute_unpause "$ADMIN" "$GOVERNANCE" -- execute \
        --executor null --target "$GOV_CONTROLLER" --function unpause \
        --args-file-path "$unpause_args_f" --predecessor "$GOV_ZERO32" --salt "$GOV_SALT_UNPAUSE" >/dev/null
view gov_min_delay "$GOVERNANCE" -- get_min_delay >/dev/null
view gov_has_role_admin_executor "$GOVERNANCE" -- has_role \
--account "$ADMIN_ADDR" --role EXECUTOR >/dev/null
view gov_resolve_tol "$GOVERNANCE" -- resolve_oracle_tolerance \
--tolerance 200 >/dev/null

    local op_cancel
    op_cancel=$(inv gov_propose_cancel "$ADMIN" "$GOVERNANCE" -- propose \
        --proposer "$ADMIN_ADDR" \
        --op '{"SetPositionLimits":{"max_supply_positions":6,"max_borrow_positions":6}}' \
        --salt "$GOV_SALT_CANCEL" | tr -d '"[:space:]')
    gov_assert_state gov_state_waiting "$op_cancel" Waiting
    inv gov_cancel "$ADMIN" "$GOVERNANCE" -- cancel \
        --canceller "$ADMIN_ADDR" --operation_id "$op_cancel" >/dev/null
    gov_assert_state gov_state_unset "$op_cancel" Unset

    local op_exec st args_f
    op_exec=$(inv gov_propose_exec "$ADMIN" "$GOVERNANCE" -- propose \
        --proposer "$ADMIN_ADDR" \
        --op '{"SetPositionLimits":{"max_supply_positions":8,"max_borrow_positions":8}}' \
        --salt "$GOV_SALT_EXEC" | tr -d '"[:space:]')
    st=$(gov_await_ready "$op_exec")
    if [ "$st" != "Ready" ] && [ "$st" != "Done" ]; then
        _assert_fail gov_await_ready "op $op_exec never reached Ready (state=$st)"
    fi
args_f="$LOG_DIR/gov_exec_args.json"
gov_scval_args set_position_limits \
--limits '{"max_supply_positions":8,"max_borrow_positions":8}' > "$args_f"
view gov_hash_exec "$GOVERNANCE" -- hash_operation \
--target "$GOV_CONTROLLER" --function set_position_limits \
--args-file-path "$args_f" --predecessor "$GOV_ZERO32" --salt "$GOV_SALT_EXEC" >/dev/null
view gov_op_ledger_exec "$GOVERNANCE" -- get_operation_ledger \
--operation_id "$op_exec" >/dev/null
inv gov_execute "$ADMIN" "$GOVERNANCE" -- execute \
--executor null --target "$GOV_CONTROLLER" --function set_position_limits \
--args-file-path "$args_f" --predecessor "$GOV_ZERO32" --salt "$GOV_SALT_EXEC" >/dev/null
    gov_assert_state gov_state_unset_after_exec "$op_exec" Unset

    xfail gov_execute_replay 'Error\(' "$ADMIN" "$GOVERNANCE" -- execute \
        --executor null --target "$GOV_CONTROLLER" --function set_position_limits \
        --args-file-path "$args_f" --predecessor "$GOV_ZERO32" --salt "$GOV_SALT_EXEC"

    xfail gov_propose_non_proposer 'Error\(Contract, #2000\)' "$ALICE" "$GOVERNANCE" -- propose \
        --proposer "$ALICE_ADDR" \
        --op '{"SetPositionLimits":{"max_supply_positions":5,"max_borrow_positions":5}}' \
        --salt "$GOV_SALT_DENY"

xfail gov_propose_bad_limits 'Error\(Contract, #36\)' "$ADMIN" "$GOVERNANCE" -- propose \
--proposer "$ADMIN_ADDR" \
--op '{"SetPositionLimits":{"max_supply_positions":11,"max_borrow_positions":11}}' \
--salt "$GOV_SALT_BADLIMITS"

xfail gov_propose_bad_liquidation_curve 'Error\(Contract, #134\)' "$ADMIN" "$GOVERNANCE" -- propose \
--proposer "$ADMIN_ADDR" \
--op '{"SetSpokeLiquidationCurve":{"spoke_id":1,"target_hf_wad":"1020000000000000000","hf_for_max_bonus_wad":"510000000000000000","liquidation_bonus_factor_bps":10001}}' \
--salt "$GOV_SALT_BADCURVE"

local delay_now delay_next op_self delay_got
delay_now=$(view gov_min_delay_pre_self "$GOVERNANCE" -- get_min_delay | tr -d '"[:space:]')
delay_next=$((delay_now + 1))
op_self=$(inv gov_self_propose_delay "$ADMIN" "$GOVERNANCE" -- propose \
    --proposer "$ADMIN_ADDR" \
    --op "{\"UpdateGovDelay\":$delay_next}" \
    --salt "$GOV_SALT_SELF_DELAY" | tr -d '"[:space:]')
gov_assert_state gov_self_state_waiting "$op_self" Waiting
st=$(gov_await_ready "$op_self")
if [ "$st" != "Ready" ] && [ "$st" != "Done" ]; then
    _assert_fail gov_self_await_ready "op $op_self never reached Ready (state=$st)"
fi
inv gov_self_execute_delay "$ADMIN" "$GOVERNANCE" -- execute_self \
    --executor null \
    --op "{\"UpdateGovDelay\":$delay_next}" \
    --salt "$GOV_SALT_SELF_DELAY" >/dev/null

delay_got=$(view gov_min_delay_post_self "$GOVERNANCE" -- get_min_delay | tr -d '"[:space:]')
if [ "$delay_got" != "$delay_next" ]; then
    _assert_fail gov_min_delay_post_self "got '$delay_got', want '$delay_next'"
fi

local op_sensitive
op_sensitive=$(inv gov_self_propose_grant "$ADMIN" "$GOVERNANCE" -- propose \
    --proposer "$ADMIN_ADDR" \
    --op '{"GrantGovRole":{"account":"'"$DAVE_ADDR"'","role":"EXECUTOR"}}' \
    --salt "$GOV_SALT_SELF_SENSITIVE" | tr -d '"[:space:]')
gov_assert_state gov_self_sensitive_waiting "$op_sensitive" Waiting
inv gov_self_cancel_grant "$ADMIN" "$GOVERNANCE" -- cancel \
    --canceller "$ADMIN_ADDR" --operation_id "$op_sensitive" >/dev/null
gov_assert_state gov_self_sensitive_unset "$op_sensitive" Unset

xfail gov_execute_immediate_absent 'execute_immediate|unknown|not found|No such' \
"$ADMIN" "$GOVERNANCE" -- execute_immediate \
--caller "$ADMIN_ADDR" \
--op '{"GrantGovRole":{"account":"'"$DAVE_ADDR"'","role":"EXECUTOR"}}'
xfail gov_set_controller_absent 'set_controller|unknown|not found|No such' \
"$ADMIN" "$GOVERNANCE" -- set_controller --addr "$CONTROLLER"

    flow_gov_recovery_and_roles

    inv gov_pause "$ADMIN" "$GOVERNANCE" -- pause --caller "$ADMIN_ADDR" >/dev/null
}

# The governance surface the main flow never reached: its own price aggregator,
# the oracle-gated sanity band, immediate role revocation, and the Recovery-tier
# canceller reset.
#
# ADMIN holds every default operational role from the constructor, so it is both
# owner and ORACLE/GUARDIAN here.
flow_gov_recovery_and_roles() {
    # `set_sanity_band` forwards to `price_aggregator_client`, so governance
    # needs its own aggregator before the band can be set. That is also the only
    # way to reach `deploy_price_aggregator` / `price_aggregator`.
    local gov_pa=""
    if [ -n "${PA_HASH:-}" ]; then
        gov_pa=$(inv gov_deploy_price_agg "$ADMIN" "$GOVERNANCE" -- deploy_price_aggregator \
            --wasm_hash "$PA_HASH" | tr -d '"[:space:]')
    fi
    if [ -z "$gov_pa" ] || ! is_contract_id "$gov_pa"; then
        log "gov_recovery: no governance price aggregator (PA_HASH=${PA_HASH:-unset}); skipping sanity band"
    else
        record gov_price_aggregator_deployed ok deploy_price_aggregator "" "" "" "" "" "$gov_pa"
        view gov_price_aggregator_getter "$GOVERNANCE" -- price_aggregator >/dev/null

        # Deploying a second one must be refused, or the registered aggregator
        # could be swapped out from under the controller.
        xfail gov_deploy_price_agg_twice 'Error\(Contract' "$ADMIN" "$GOVERNANCE" -- deploy_price_aggregator \
            --wasm_hash "$PA_HASH"

        inv gov_set_sanity_band "$ADMIN" "$GOVERNANCE" -- set_sanity_band \
            --caller "$ADMIN_ADDR" --key "$(price_key_token "$SAC_LIQA")" \
            --min_wad $((WAD / 100)) --max_wad $((WAD * 100)) >/dev/null

        # Without the oracle role the band must not move: it is the bound that
        # decides which prices the protocol will accept at all.
        xfail gov_set_sanity_band_no_role 'Error\(Contract' "$ALICE" "$GOVERNANCE" -- set_sanity_band \
            --caller "$ALICE_ADDR" --key "$(price_key_token "$SAC_LIQA")" \
            --min_wad $((WAD / 100)) --max_wad $((WAD * 100))
    fi

    # --- immediate role revocation (owner only, guardian/oracle only) ---
    view gov_has_guardian_pre "$GOVERNANCE" -- has_role \
        --account "$ADMIN_ADDR" --role GUARDIAN >/dev/null
    inv gov_revoke_guardian "$ADMIN" "$GOVERNANCE" -- revoke_role_immediate \
        --account "$ADMIN_ADDR" --role GUARDIAN >/dev/null
    # Revoked for real: a guardian-gated call must now fail.
    xfail gov_guardian_gone 'Error\(Contract' "$ADMIN" "$GOVERNANCE" -- create_hub \
        --caller "$ADMIN_ADDR"

    # Only guardian and oracle are revocable this way; EXECUTOR must not be.
    xfail gov_revoke_executor_rejected 'Error\(Contract' "$ADMIN" "$GOVERNANCE" -- revoke_role_immediate \
        --account "$ADMIN_ADDR" --role EXECUTOR

    # --- canceller reset (Recovery tier) ---
    # TIMELOCK_RECOVERY_MIN_DELAY_LEDGERS is 518_400 (~30 days at 5s/ledger), so
    # the execute half is unreachable inside a run. What is reachable — and what
    # actually matters — is that it schedules, and that executing early is
    # refused rather than silently applied.
    local cancellers op_reset
    cancellers=$(jq -nc --arg a "$DAVE_ADDR" '[$a]')
    op_reset=$(inv gov_propose_canceller_reset "$ADMIN" "$GOVERNANCE" -- propose_canceller_reset \
        --new_cancellers "$cancellers" --salt "$GOV_SALT_CANCELLER_RESET" | tr -d '"[:space:]')
    if [ -n "$op_reset" ]; then
        record gov_canceller_reset_scheduled ok propose_canceller_reset "" "" "" "" "" "op=$op_reset"
        gov_assert_state gov_canceller_reset_waiting "$op_reset" Waiting
    else
        _assert_fail gov_canceller_reset_scheduled "propose_canceller_reset returned no operation id"
    fi

    xfail gov_execute_canceller_reset_early 'Error\(Contract' "$ADMIN" "$GOVERNANCE" -- execute_canceller_reset \
        --executor null --new_cancellers "$cancellers" --salt "$GOV_SALT_CANCELLER_RESET"
}
