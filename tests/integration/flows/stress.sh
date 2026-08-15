stress_code() { printf 'ST%02d' "$1"; }
stress_sac()  { local v="SAC_ST$(printf '%02d' "$1")"; echo "${!v}"; }

flow_stress_setup() {
    phase stress_setup
    [ -n "${STRESS_SETUP_DONE:-}" ] && return 0
    deploy_mock_reflector
    deploy_mock_redstone
    local i code var sac
    for i in $(seq 0 $((STRESS_N - 1))); do
        code=$(stress_code "$i")
        var="SAC_$code"
        issue_sac "$var" "$code"
        sac="${!var}"
        trustline "$DAVE" "$code" "$ADMIN_ADDR"
        trustline "$CAROL" "$code" "$ADMIN_ADDR"
        mint_to "$sac" "$code" "$DAVE_ADDR"  $((1000000 * STRESS_UNIT))
        mint_to "$sac" "$code" "$CAROL_ADDR" $((1000000 * STRESS_UNIT))
        set_mock_price "$sac" "$WAD" "px_init_$code"
        create_market "$code" "$PRIMARY_HUB_ID" "$sac" 7 "$(oracle_cfg_mock_single "$sac")" "$(asset_config_json 7000 7500 800)"
    done

    local args1="" args2=""
    for i in 10 11 12 13 14; do args1+=" $(stress_sac $i) $((200000 * STRESS_UNIT))"; done
    for i in 15 16 17 18 19; do args2+=" $(stress_sac $i) $((200000 * STRESS_UNIT))"; done
    inv stress_seed_liq_1 "$CAROL" "$CONTROLLER" -- supply \
        --caller "$CAROL_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" $args1)" >/dev/null || return 1
    inv stress_seed_liq_2 "$CAROL" "$CONTROLLER" -- supply \
        --caller "$CAROL_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" $args2)" >/dev/null || return 1
    save_state STRESS_SETUP_DONE 1
}

# Probes exist to find the CPU/budget frontier: `sim-exceeded` is the expected
# terminal state and is not a failure. A *contract* error is recorded as
# `sim-error` and does count, so every probe has to stay inside
# POSITION_LIMIT_MAX (5) — past it the probe trips #109 PositionLimitExceeded
# and measures the validator instead of the budget.
flow_stress_supply_frontier() {
    phase stress_supply_frontier
    local k args i
    for k in 1 2 3 4 5; do
        args=""
        for i in $(seq 0 $((k - 1))); do args+=" $(stress_sac $i) $((10000 * STRESS_UNIT))"; done
        sim_probe "probe_supply_${k}assets" "$DAVE" "$CONTROLLER" -- supply \
            --caller "$DAVE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --assets "$(pay_vec "$PRIMARY_HUB_ID" $args)"
        [ "$PROBE_STATUS" = exceeded ] && { log "supply frontier: $k distinct assets exceeds"; break; }
    done
}

flow_stress_borrow_frontier() {
    local mode="${1:-single}" colls acct_var
    phase stress_borrow_frontier
    # Single mode is the worst case, so it sits at the limit itself (5). It used
    # to be 10, which needed a second supply into the same account — that second
    # call is what failed with #109 once the limit dropped. At 5 the `colls > 5`
    # branch below is dead and the account is built in one call.
    if [ "$mode" = dual ]; then colls=4; acct_var=DAVE_DUAL_ACCT; else colls=5; acct_var=DAVE_ACCT; fi
    local args="" i acct
    if [ -z "${!acct_var:-}" ]; then
        for i in $(seq 0 $(( colls > 5 ? 4 : colls - 1 ))); do args+=" $(stress_sac $i) $((100000 * STRESS_UNIT))"; done
        acct=$(inv_create "stress_supply_${mode}_base" "$DAVE" "$CONTROLLER" -- supply \
            --caller "$DAVE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --assets "$(pay_vec "$PRIMARY_HUB_ID" $args)" | tr -d '"') || return 1
        save_state "$acct_var" "$acct"
        if [ "$colls" -gt 5 ]; then
            args=""
            for i in $(seq 5 $((colls - 1))); do args+=" $(stress_sac $i) $((100000 * STRESS_UNIT))"; done
            inv "stress_supply_${mode}_rest" "$DAVE" "$CONTROLLER" -- supply \
                --caller "$DAVE_ADDR" --account_id "$acct" --spoke_id "$PRIMARY_SPOKE_ID" \
                --assets "$(pay_vec "$PRIMARY_HUB_ID" $args)" >/dev/null
        fi
    fi
    acct="${!acct_var}"
    local k best_k=0
    # Borrow positions are capped at 5 independently of the supply side.
    for k in $(seq 1 5); do
        args=""
        for i in $(seq 10 $((9 + k))); do args+=" $(stress_sac $i) $((1000 * STRESS_UNIT))"; done
        sim_probe "probe_borrow_${mode}_$((colls + k))feeds" "$DAVE" "$CONTROLLER" -- borrow \
            --caller "$DAVE_ADDR" --account_id "$acct" \
            --borrows "$(pay_vec "$PRIMARY_HUB_ID" $args)" --to null
        if [ "$PROBE_STATUS" = ok ]; then
            best_k=$k
        elif [ "$PROBE_STATUS" = exceeded ]; then
            log "borrow frontier ($mode): $((colls + k)) feeds exceeds; largest passing probe $((colls + best_k)) feeds"
            break
        fi
    done
    local mode_key
    mode_key=$(printf '%s' "$mode" | tr '[:lower:]' '[:upper:]')
    save_state "BORROW_FRONTIER_${mode_key}" "$((colls + best_k))"

    if [ "$best_k" -gt 0 ]; then
        args=""
        for i in $(seq 10 $((9 + best_k))); do args+=" $(stress_sac $i) $((1000 * STRESS_UNIT))"; done
        inv "stress_borrow_${mode}_proof" "$DAVE" "$CONTROLLER" -- borrow \
            --caller "$DAVE_ADDR" --account_id "$acct" \
            --borrows "$(pay_vec "$PRIMARY_HUB_ID" $args)" --to null >/dev/null
        sim_probe "probe_withdraw_${mode}_maxfeeds" "$DAVE" "$CONTROLLER" -- withdraw \
            --caller "$DAVE_ADDR" --account_id "$acct" \
            --withdrawals "$(pay_vec "$PRIMARY_HUB_ID" "$(stress_sac 0)" $((1000 * STRESS_UNIT)))" --to null
        args=""
        for i in $(seq 10 $((9 + best_k))); do args+=" $(stress_sac $i) $((1100 * STRESS_UNIT))"; done
        inv "stress_repay_${mode}_reset" "$DAVE" "$CONTROLLER" -- repay \
            --caller "$DAVE_ADDR" --account_id "$acct" \
            --payments "$(pay_vec "$PRIMARY_HUB_ID" $args)" >/dev/null
    fi
}

flow_stress_dualify() {
    phase stress_dualify
    [ -n "${STRESS_DUAL_DONE:-}" ] && return 0
    local i code sac
    for i in $(seq 0 $((STRESS_N - 1))); do
        code=$(stress_code "$i")
        sac=$(stress_sac "$i")
        set_rs_price "$code" "$WAD" "rs_px_$code"
        local resolved_dual
        local dual_key dual_oracle_file dual_resolved_file
        dual_key=$(price_key_token "$sac")
        dual_oracle_file=$(mktemp)
        dual_resolved_file=$(mktemp)
        printf '%s' "$(oracle_cfg_mock_dual "$sac" "$code")" > "$dual_oracle_file"
        resolved_dual=$(view "dualify_resolve_$code" "$GOVERNANCE" -- resolve_asset_oracle \
            --key "$dual_key" --oracle-file-path "$dual_oracle_file" | jq -c '.') || {
            rm -f "$dual_oracle_file" "$dual_resolved_file"
            continue
        }
        printf '%s' "$resolved_dual" > "$dual_resolved_file"
        inv "dualify_$code" "$ADMIN" "$PRICE_AGGREGATOR" -- set_oracle \
            --key "$dual_key" --oracle-file-path "$dual_resolved_file" >/dev/null || {
            rm -f "$dual_oracle_file" "$dual_resolved_file"
            continue
        }
        rm -f "$dual_oracle_file" "$dual_resolved_file"
    done
    save_state STRESS_DUAL_DONE 1
}

flow_stress_liq_frontier() {
    phase stress_liq_frontier
local k i args acct var debt_args repay_args
    for k in 3 4 5; do
        var="LIQF_ACCT_$k"
        if [ -z "${!var:-}" ]; then
            args=""
            for i in $(seq 0 $((k - 1))); do args+=" $(stress_sac $i) $((1000 * STRESS_UNIT))"; done
            acct=$(inv_create "liqf_supply_${k}coll" "$DAVE" "$CONTROLLER" -- supply \
                --caller "$DAVE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
                --assets "$(pay_vec "$PRIMARY_HUB_ID" $args)" | tr -d '"') || continue
            inv "liqf_borrow_${k}coll" "$DAVE" "$CONTROLLER" -- borrow \
                --caller "$DAVE_ADDR" --account_id "$acct" \
                --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$(stress_sac 19)" $((k * 600 * STRESS_UNIT)))" --to null >/dev/null || continue
            save_state "$var" "$acct"
        fi
    done
    # One maximal account instead of the old 8C8D and 10C10D pair: at a limit of
    # 5 both collapse to the same shape, and 5 collateral against 5 debts IS the
    # worst case a liquidation has to clear.
    if [ -z "${LIQF_ACCT_MAX:-}" ]; then
        args=""
        for i in $(seq 0 4); do args+=" $(stress_sac $i) $((1000 * STRESS_UNIT))"; done
        acct=$(inv_create liqf_supply_5coll_5debt "$DAVE" "$CONTROLLER" -- supply \
            --caller "$DAVE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --assets "$(pay_vec "$PRIMARY_HUB_ID" $args)" | tr -d '"') || return 1
        debt_args=""
        for i in $(seq 10 14); do debt_args+=" $(stress_sac $i) $((600 * STRESS_UNIT))"; done
        inv liqf_borrow_5coll_5debt "$DAVE" "$CONTROLLER" -- borrow \
            --caller "$DAVE_ADDR" --account_id "$acct" \
            --borrows "$(pay_vec "$PRIMARY_HUB_ID" $debt_args)" --to null >/dev/null || return 1
        save_state LIQF_ACCT_MAX "$acct"
    fi

    # Only collateral 0-4 is in play now, so there is no reason to move 5-9.
    for i in $(seq 0 4); do
        dual_px "$(stress_sac $i)" "$(stress_code $i)" $((WAD / 10 * 6)) "crash_$(stress_code $i)"
    done
    local best_k=0
    for k in 3 4 5; do
        var="LIQF_ACCT_$k"
        acct="${!var:-}"
        [ -z "$acct" ] && continue
        sim_probe "probe_liquidate_${k}coll" "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_transfer)" \
            --liquidator "$CAROL_ADDR" --account_id "$acct" \
            --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$(stress_sac 19)" $((100 * STRESS_UNIT)))"
        [ "$PROBE_STATUS" = ok ] && best_k=$k
    done
    save_state LIQ_FRONTIER_COLL "$best_k"

    if [ "$best_k" -gt 0 ]; then
        var="LIQF_ACCT_$best_k"
        inv "stress_liquidate_proof_${best_k}coll" "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_transfer)" \
            --liquidator "$CAROL_ADDR" --account_id "${!var}" \
            --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$(stress_sac 19)" $((100 * STRESS_UNIT)))" >/dev/null
    fi
    # Worst case, all five debts repaid at once against five collaterals.
    repay_args=""
    for i in $(seq 10 14); do repay_args+=" $(stress_sac $i) $((100 * STRESS_UNIT))"; done
    sim_probe probe_liquidate_5coll_5debt_full "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_transfer)" \
        --liquidator "$CAROL_ADDR" --account_id "$LIQF_ACCT_MAX" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" $repay_args)"
    save_state LIQ_FRONTIER_MAX_FULL "$PROBE_STATUS"
    if [ "$PROBE_STATUS" = ok ]; then
        # `research` rather than a hard failure: a simulation that fits can still
        # miss live, and that gap is the measurement, not a regression.
        if INV_FAIL_STATUS=research inv stress_liquidate_proof_5coll_5debt_full "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_transfer)" \
            --liquidator "$CAROL_ADDR" --account_id "$LIQF_ACCT_MAX" \
            --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" $repay_args)" >/dev/null; then
            save_state LIQ_FRONTIER_MAX_FULL_LIVE ok
        else
            save_state LIQ_FRONTIER_MAX_FULL_LIVE research
        fi
    fi

    # If the full sweep does not land, fall back to the single-debt leg so the
    # lane still records where the boundary actually sits.
    if [ "${LIQ_FRONTIER_MAX_FULL_LIVE:-}" != ok ]; then
        repay_args="$(stress_sac 14) $((100 * STRESS_UNIT))"
        sim_probe probe_liquidate_5coll_one_debt "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_transfer)" \
            --liquidator "$CAROL_ADDR" --account_id "$LIQF_ACCT_MAX" \
            --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" $repay_args)"
        save_state LIQ_FRONTIER_MAX_ONE_DEBT "$PROBE_STATUS"
        if [ "$PROBE_STATUS" = ok ]; then
            inv stress_liquidate_proof_5coll_one_debt "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_transfer)" \
                --liquidator "$CAROL_ADDR" --account_id "$LIQF_ACCT_MAX" \
                --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" $repay_args)" >/dev/null
        fi
    fi
}
