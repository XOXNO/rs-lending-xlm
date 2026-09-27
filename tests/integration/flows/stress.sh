stress_code() { printf 'ST%02d' "$1"; }
stress_sac()  { local v="SAC_ST$(printf '%02d' "$1")"; echo "${!v}"; }

flow_stress_setup() {
    phase stress_setup
    [ -n "${STRESS_SETUP_DONE:-}" ] && return 0
    local i code trust=() mint=()
    for i in $(seq 0 $((STRESS_N - 1))); do
        code=$(stress_code "$i")
        trust+=("trust:$code:$ADMIN_ADDR")
        mint+=("pay:$DAVE_ADDR:$code:$ADMIN_ADDR:$((1000000 * STRESS_UNIT))" "pay:$CAROL_ADDR:$code:$ADMIN_ADDR:$((1000000 * STRESS_UNIT))")
    done
    classic_batch stress_trust_dave change_trust "$DAVE" "${trust[@]}" || return 1
    classic_batch stress_trust_carol change_trust "$CAROL" "${trust[@]}" || return 1
    classic_batch stress_mint_classic payment "$ADMIN" "${mint[@]}" || return 1
    group_each stress_fixtures 10 "$(seq 0 $((STRESS_N - 1)))" stress_fixture_job || return 1
    stress_create_markets || return 1

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

stress_fixture_job() {
    local i="$1" code var
    code=$(stress_code "$i"); var="SAC_$code"
    MOCK=''; MOCKRS=''
    deploy_mock_reflector || return 1
    deploy_mock_redstone || return 1
    save_state "STRESS_REF_$i" "$MOCK" || return 1
    save_state "STRESS_RS_$i" "$MOCKRS" || return 1
    issue_sac "$var" "$code" || return 1
    set_mock_price "${!var}" "$WAD" "px_init_$code" || return 1
    set_rs_price "$code" "$WAD" "rs_px_$code"
}

stress_create_markets() {
    local i code sac var pending='' resolved
    for i in $(seq 0 $((STRESS_N - 1))); do
        var="MKT_$(stress_code "$i")_DONE"
        [ -n "${!var:-}" ] || pending+="$i"$'\n'
    done
    [ -n "$pending" ] || return 0
    group_each stress_markets 10 "$pending" stress_market_job || return 1
    group_begin stress_resolve 8 reads || return 1
    for i in $pending; do group_spawn stress_resolve_job "$i"; done
    group_end || return 1
    resolved="$GROUP_LAST"
    group_begin stress_listing 21 || return 1
    group_spawn stress_set_oracles_job "$resolved" $pending
    for i in $pending; do group_spawn stress_activate_job "$i"; done
    group_end || return 1
    for i in $pending; do
        code=$(stress_code "$i"); sac=$(stress_sac "$i")
        market_wait_listed "$PRIMARY_HUB_ID" "$sac" \
            || die "confirm_market_$code" "market $code primary spoke listing not active after create -> oracle -> activate (read replica lag exhausted)"
        case " ${MARKETS:-} " in
            *" ${PRIMARY_HUB_ID}:${sac} "*) ;;
            *) save_state MARKETS "${MARKETS:+$MARKETS }${PRIMARY_HUB_ID}:${sac}" || return 1 ;;
        esac
        save_state "MKT_${code}_DONE" 1 || return 1
    done
}

stress_market_job() {
    local code sac
    code=$(stress_code "$1"); sac=$(stress_sac "$1")
    if market_listing_exists "$PRIMARY_HUB_ID" "$sac"; then
        record "create_market_$code" ok create_liquidity_pool "" "" "" "" "" "listing already exists (resume); skipping create"
        return
    fi
    inv_owner "create_market_$code" "$CONTROLLER" -- create_liquidity_pool \
        --hub_id "$PRIMARY_HUB_ID" --asset "$sac" --params "$(market_params_json "$sac" 7)" >/dev/null
}

stress_resolve_job() {
    local sac resolved
    sac=$(stress_sac "$1")
    stress_select_oracles "$1" || return 1
    oracle_cfg_mock_single "$sac" > "$E2E_JOB_DIR/oracle.json" || return 1
    resolved=$(view "resolve_oracle_$(stress_code "$1")" "$GOVERNANCE" -- resolve_asset_oracle \
        --key "$(price_key_token "$sac")" --oracle-file-path "$E2E_JOB_DIR/oracle.json") || return 1
    jq -c . <<<"$resolved" > "$E2E_JOB_DIR/resolved.json"
}

stress_set_oracles_job() {
    local dir="$1" i n=0
    shift
    for i in "$@"; do
        n=$((n + 1))
        inv "set_oracle_$(stress_code "$i")" "$ADMIN" "$PRICE_AGGREGATOR" -- set_oracle \
            --key "$(price_key_token "$(stress_sac "$i")")" --oracle-file-path "$dir/$n/resolved.json" >/dev/null || return 1
    done
}

stress_activate_job() {
    inv_owner "activate_$(stress_code "$1")" "$CONTROLLER" -- add_asset_to_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$(stress_sac "$1")" "$PRIMARY_SPOKE_ID" true true 7000 7500 800)" >/dev/null
}

stress_crash_job() {
    stress_select_oracles "$1" || return 1
    dual_px "$(stress_sac "$1")" "$(stress_code "$1")" $((WAD / 10 * 6)) "crash_$(stress_code "$1")"
}

# Required probes stay inside the supported dimensions; budget failure blocks the gate. A contract error is recorded as `sim-error` and counts, so
# every probe stays within POSITION_LIMIT_MAX (5). Past it, the probe reverts
# with #109 PositionLimitExceeded and measures the validator, not the budget.
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
    [ "$PROBE_STATUS" = ok ] || return 1
    return 0
}

flow_stress_borrow_frontier() {
    local mode="${1:-single}" colls acct_var
    phase stress_borrow_frontier
    colls=5
    case "$mode" in
        single) acct_var=DAVE_ACCT ;;
        dual) acct_var=DAVE_DUAL_ACCT ;;
        composed) acct_var=DAVE_COMPOSED_ACCT ;;
        *) _assert_fail stress_mode "unknown resource scenario $mode"; return 1 ;;
    esac
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

    [ "$best_k" -eq 5 ] || { _assert_fail stress_dimensions "all five debts must fit"; return 1; }
    if [ "$best_k" -eq 5 ]; then
        args=""
        for i in $(seq 10 $((9 + best_k))); do args+=" $(stress_sac $i) $((1000 * STRESS_UNIT))"; done
        inv "stress_borrow_${mode}_proof" "$DAVE" "$CONTROLLER" -- borrow \
            --caller "$DAVE_ADDR" --account_id "$acct" \
            --borrows "$(pay_vec "$PRIMARY_HUB_ID" $args)" --to null >/dev/null || return 1
        local withdrawals=""
        for i in $(seq 0 4); do withdrawals+=" $(stress_sac $i) $((1000 * STRESS_UNIT))"; done
        inv "proof_withdraw_${mode}_maxfeeds" "$DAVE" "$CONTROLLER" -- withdraw \
            --caller "$DAVE_ADDR" --account_id "$acct" \
            --withdrawals "$(pay_vec "$PRIMARY_HUB_ID" $withdrawals)" --to null >/dev/null || return 1
        args=""
        for i in $(seq 10 $((9 + best_k))); do args+=" $(stress_sac $i) $((1100 * STRESS_UNIT))"; done
        inv "stress_repay_${mode}_reset" "$DAVE" "$CONTROLLER" -- repay \
            --caller "$DAVE_ADDR" --account_id "$acct" \
            --payments "$(pay_vec "$PRIMARY_HUB_ID" $args)" >/dev/null || return 1
    fi
}

flow_stress_dualify() {
    phase stress_dualify
    [ -n "${STRESS_DUAL_DONE:-}" ] && return 0
    group_each stress_dualify 20 "$(seq 0 $((STRESS_N - 1)))" stress_dualify_job || return 1
    save_state STRESS_DUAL_DONE 1
}

stress_dualify_job() {
    local code sac key resolved
    code=$(stress_code "$1"); sac=$(stress_sac "$1"); key=$(price_key_token "$sac")
    stress_select_oracles "$1" || return 1
    oracle_cfg_mock_dual "$sac" "$code" > "$E2E_JOB_DIR/oracle.json" || return 1
    resolved=$(view "dualify_resolve_$code" "$GOVERNANCE" -- resolve_asset_oracle \
        --key "$key" --oracle-file-path "$E2E_JOB_DIR/oracle.json") || return 1
    jq -c . <<<"$resolved" > "$E2E_JOB_DIR/resolved.json" || return 1
    inv_owner "dualify_$code" "$PRICE_AGGREGATOR" -- set_oracle \
        --key "$key" --oracle-file-path "$E2E_JOB_DIR/resolved.json" >/dev/null
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
    # One maximal account: 5 collaterals against 5 debts is the largest
    # liquidation that the position limit (5) allows.
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

    # Only collaterals 0-4 back these accounts, so only their prices move.
    group_each stress_crash 10 "$(seq 0 4)" stress_crash_job || return 1
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
    [ "$PROBE_STATUS" = ok ] || { _assert_fail stress_max_liquidation "required 5+5 simulation failed"; return 1; }
    inv stress_liquidate_proof_5coll_5debt_full "$CAROL" "$CONTROLLER" -- liquidate --seize_mode "$(seize_transfer)" \
        --liquidator "$CAROL_ADDR" --account_id "$LIQF_ACCT_MAX" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" $repay_args)" >/dev/null || return 1
    local receiver
    receiver=$(inv stress_liquidate_credit_5coll_5debt "$CAROL" "$CONTROLLER" -- liquidate --seize_mode '{"Credit":0}' \
        --liquidator "$CAROL_ADDR" --account_id "$LIQF_ACCT_MAX" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" $repay_args)" | tr -d '\"[:space:]') || return 1
    [[ "$receiver" =~ ^[1-9][0-9]*$ ]] || { _assert_fail stress_credit "invalid receiver $receiver"; return 1; }
    assert_view_eq_at "$POSITION_NFT" stress_credit_owner "$CAROL_ADDR" owner_of --token_id "$receiver"

}

stress_select_oracles() {
    local ref="STRESS_REF_$1" rs="STRESS_RS_$1"
    MOCK="${!ref}"; MOCKRS="${!rs}"
    is_contract_id "$MOCK" && is_contract_id "$MOCKRS" || { _assert_fail stress_provider "missing independent provider for asset $1"; return 1; }
}

# Ten roots resolve through ten reference keys and twenty separate providers.
# This exercises Scaled composition costs; it does not stand in for LP costs.
flow_stress_composed() {
    phase stress_composed
    local i code sac dual quote roots
    roots=$(printf '%s\n' 0 1 2 3 4 10 11 12 13 14)
    for i in $roots; do
        code=$(stress_code "$i"); sac=$(stress_sac "$i")
        stress_select_oracles "$i" || return 1
        dual=$(oracle_cfg_mock_dual "$sac" "$code") || return 1
        # A USD-base Reflector is a bare quote; Ref-scaled factors use RedStone.
        quote=$(jq -c '.asset_decimals=0 | .sources=[.sources[0]] | .min_sanity_price_wad="960000000000000000" | .max_sanity_price_wad="1040000000000000000"' <<<"$dual") || return 1
        inv "stress_composed_quote_$code" "$ADMIN" "$PRICE_AGGREGATOR" -- set_oracle \
            --key "{\"Ref\":\"Q$code\"}" --oracle "$quote" >/dev/null || return 1
    done
    group_each stress_composed_token 10 "$roots" stress_composed_token_job || return 1
    flow_stress_borrow_frontier composed || return 1
    # Keep the following liquidation scenario's dual-feed price shock unchanged.
    group_each stress_composed_restore 10 "$roots" stress_composed_restore_job || return 1
}

stress_composed_token_job() {
    local code sac dual scaled
    code=$(stress_code "$1"); sac=$(stress_sac "$1")
    stress_select_oracles "$1" || return 1
    dual=$(oracle_cfg_mock_dual "$sac" "$code") || return 1
    scaled=$(jq -c --arg ref "Q$code" '.sources=[{Scaled:{factor:.sources[1].Feed,quote:{Ref:$ref},min_factor_wad:"500000000000000000",max_factor_wad:"1500000000000000000"}}] | .min_sanity_price_wad="960000000000000000" | .max_sanity_price_wad="1040000000000000000"' <<<"$dual") || return 1
    inv_owner "stress_composed_token_$code" "$PRICE_AGGREGATOR" -- set_oracle \
        --key "$(price_key_token "$sac")" --oracle "$scaled" >/dev/null
}

stress_composed_restore_job() {
    local code sac dual
    code=$(stress_code "$1"); sac=$(stress_sac "$1")
    stress_select_oracles "$1" || return 1
    dual=$(oracle_cfg_mock_dual "$sac" "$code") || return 1
    inv_owner "stress_composed_restore_$code" "$PRICE_AGGREGATOR" -- set_oracle \
        --key "$(price_key_token "$sac")" --oracle "$dual" >/dev/null
}

# Prepare once, change the same account using another signer, then submit the
# stale envelope after three ledgers. No resimulation or submission retry.
flow_stress_delayed() {
    [ -z "${E2E_JOB:-}" ] || { job_refuse flow_stress_delayed stress_delayed_borrow; return 1; }
    phase stress_delayed
    local acct="${DAVE_DUAL_ACCT:?dual resource scenario must run first}"
    local label=stress_delayed_borrow i args="" tops="" key after start now attempt hash rc=0 st positions sequence
    local -a balances collateral
    for i in 0 1 2 3 4; do
        key=$(hub_key "$PRIMARY_HUB_ID" "$(stress_sac "$i")")
        collateral[$i]=$(_view_int "stress_delayed_coll_before_$i" get_collateral_amount --account_id "$acct" --hub_asset "$key") || return 1
        tops+=" $(stress_sac "$i") $STRESS_UNIT"
        balances[$i]=$(balance "$(stress_sac "$((i+10))")" "$DAVE_ADDR") || return 1
        args+=" $(stress_sac "$((i+10))") $((1000 * STRESS_UNIT))"
    done
    cli_read stellar contract invoke --id "$CONTROLLER" --source "$DAVE" "${NET_ARGS[@]}" --build-only -- borrow \
        --caller "$DAVE_ADDR" --account_id "$acct" --borrows "$(pay_vec "$PRIMARY_HUB_ID" $args)" --to null \
        >"$LOG_DIR/$label.built.xdr" 2>"$LOG_DIR/$label.prepare.err" || return 1
    stellar tx simulate --source "$DAVE" "${NET_ARGS[@]}" --instruction-leeway "${INSTRUCTION_LEEWAY:-20000000}" \
        <"$LOG_DIR/$label.built.xdr" >"$LOG_DIR/$label.prepared.xdr" 2>>"$LOG_DIR/$label.prepare.err" || return 1
    stellar tx sign --sign-with-key "$DAVE" "${NET_ARGS[@]}" \
        <"$LOG_DIR/$label.prepared.xdr" >"$LOG_DIR/$label.signed.xdr" 2>>"$LOG_DIR/$label.prepare.err" || return 1
    hash=$(stellar tx hash --network-passphrase "$NETWORK_PASSPHRASE" <"$LOG_DIR/$label.signed.xdr") || return 1
    is_wasm_hash "$hash" || return 1
    start=$(latest_ledger) || return 1
    inv stress_shared_topup "$CAROL" "$CONTROLLER" -- supply --caller "$CAROL_ADDR" --account_id "$acct" \
        --spoke_id "$PRIMARY_SPOKE_ID" --assets "$(pay_vec "$PRIMARY_HUB_ID" $tops)" >/dev/null || return 1
    now=$start
    for ((attempt=1; attempt<=20; attempt++)); do
        now=$(latest_ledger) || return 1
        [ "$now" -ge "$((start+3))" ] && break
        sleep 3
    done
    [ "$now" -ge "$((start+3))" ] || { _assert_fail stress_delayed_ledgers "ledger did not advance by three"; return 1; }
    printf '{"prepared_after_ledger":%s,"submitted_after_ledger":%s,"hash":"%s"}\n' "$start" "$now" "$hash" >"$LOG_DIR/$label.delay.json"
    sequence=$(wc -l < "$ACTIONS_TSV")
    begin_attempt "$label" borrow "$sequence" 1 "$LOG_DIR/$label.out" "$LOG_DIR/$label.err" "$CONTROLLER" || return 1
    printf 'Signing transaction: %s\n' "$hash" >"$LOG_DIR/$label.err" || return 1
    stellar tx send "${NET_ARGS[@]}" <"$LOG_DIR/$label.signed.xdr" \
        >"$LOG_DIR/$label.out" 2>>"$LOG_DIR/$label.err" || rc=$?
    record_attempt "$label" borrow "$sequence" 1 "$rc" "$hash" "$LOG_DIR/$label.out" "$LOG_DIR/$label.err" "$CONTROLLER" || return 1
    st=$(tx_status "$hash")
    if [ "$rc" -ne 0 ] || [ "$st" != SUCCESS ] || ! fetch_resources "$hash"; then
        record "$label" FAIL borrow "$hash" "" "" "" "" "delayed submission: status=$st cli=$rc; no rebuild/retry"
        return 1
    fi
    record "$label" ok borrow "$hash" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" "prepared at $start, submitted after $now; intervening shared-account topup" transaction "$CONTROLLER"
    for i in 0 1 2 3 4; do
        after=$(balance "$(stress_sac "$((i+10))")" "$DAVE_ADDR") || return 1
        assert_delta "stress_delayed_receipt_$i" "${balances[$i]}" "$after" "$((1000 * STRESS_UNIT))" || return 1
        key=$(hub_key "$PRIMARY_HUB_ID" "$(stress_sac "$i")")
        after=$(_view_int "stress_delayed_coll_after_$i" get_collateral_amount --account_id "$acct" --hub_asset "$key") || return 1
        assert_delta "stress_shared_collateral_$i" "${collateral[$i]}" "$after" "$STRESS_UNIT" || return 1
    done
    positions=$(view stress_delayed_positions "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    jq -e 'length==2 and (.[0]|length)==5 and (.[1]|length)==5' <<<"$positions" >/dev/null \
        || { _assert_fail stress_delayed_dimensions "delayed proof must contain five collateral and five debt positions"; return 1; }
    record stress_delayed_dimensions ok assert "" "" "" "" "" "5 collateral + 5 debt; shared-account topup preserved after stale-envelope commit"
    args=""
    for i in 10 11 12 13 14; do args+=" $(stress_sac "$i") $((1100 * STRESS_UNIT))"; done
    inv stress_delayed_repay "$DAVE" "$CONTROLLER" -- repay --caller "$DAVE_ADDR" --account_id "$acct" \
        --payments "$(pay_vec "$PRIMARY_HUB_ID" $args)" >/dev/null || return 1
}
