# Resource-frontier stress: how many DISTINCT oracle-priced positions fit in
# one HF-checked transaction before Soroban's per-tx budget rejects the
# simulation (Error(Budget,ExceededLimit)).
#
# Probes are simulation-only (sim_probe) so the frontier search burns no fees;
# the largest passing step is then SENT for an on-chain proof tx. Measured for
# single-source oracles first, then markets are reconfigured to the
# mainnet-faithful dual-source (Reflector primary + RedStone anchor) shape and
# re-probed: per-asset cost rises, frontier drops. Liquidation is probed last
# (it reads ALL position feeds, then seizes) — the binding op.

stress_code() { printf 'ST%02d' "$1"; }
stress_sac()  { local v="SAC_ST$(printf '%02d' "$1")"; echo "${!v}"; }

# Issues 20 SACs, creates 20 single-source mock markets at $1, mints to the
# stress + liquidator wallets, seeds debt-side liquidity (two accounts of 10).
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
    # Carol seeds liquidity for the debt-side markets (ST10..ST19), 5 per tx.
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

# Bulk-supply frontier: how many distinct assets fit in ONE supply tx.
# Probes a fresh-account supply of k = 2..10 distinct collaterals.
flow_stress_supply_frontier() {
    phase stress_supply_frontier
    local k args i
    for k in 2 4 6 8 10; do
        args=""
        for i in $(seq 0 $((k - 1))); do args+=" $(stress_sac $i) $((10000 * STRESS_UNIT))"; done
        sim_probe "probe_supply_${k}assets" "$DAVE" "$CONTROLLER" -- supply \
            --caller "$DAVE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --assets "$(pay_vec "$PRIMARY_HUB_ID" $args)"
        [ "$PROBE_STATUS" = exceeded ] && { log "supply frontier: $k distinct assets exceeds"; break; }
    done
}

# Probes borrow txs that add 1..10 distinct debt assets on top of a
# `colls`-collateral account — each probe recomputes HF over (colls + k)
# distinct oracle feeds. Single-source uses a 10-collateral account (wall
# expected in the teens); dual-source uses 4 collaterals so the lower wall
# still brackets inside the probe range.
#   flow_stress_borrow_frontier <single|dual>
flow_stress_borrow_frontier() {
    local mode="${1:-single}" colls acct_var
    phase stress_borrow_frontier
    if [ "$mode" = dual ]; then colls=4; acct_var=DAVE_DUAL_ACCT; else colls=10; acct_var=DAVE_ACCT; fi
    local args="" i acct
    if [ -z "${!acct_var:-}" ]; then
        for i in $(seq 0 $(( colls > 5 ? 4 : colls - 1 ))); do args+=" $(stress_sac $i) $((100000 * STRESS_UNIT))"; done
        acct=$(inv "stress_supply_${mode}_base" "$DAVE" "$CONTROLLER" -- supply \
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
    for k in $(seq 1 10); do
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
    # On-chain proof: send the largest passing borrow, then a withdraw probe
    # at max position count, then repay in full to reset debt to zero.
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

# Reconfigures every stress market to dual-source (mock RedStone anchor).
flow_stress_dualify() {
    phase stress_dualify
    [ -n "${STRESS_DUAL_DONE:-}" ] && return 0
    local i code sac
    for i in $(seq 0 $((STRESS_N - 1))); do
        code=$(stress_code "$i")
        sac=$(stress_sac "$i")
        set_rs_price "$code" "$WAD" "rs_px_$code"
        local resolved_dual
        resolved_dual=$(view "dualify_resolve_$code" "$GOVERNANCE" -- resolve_market_oracle_config \
            --asset "$sac" --cfg "$(oracle_cfg_mock_dual "$sac" "$code")") || continue
        inv "dualify_$code" "$ADMIN" "$PRICE_AGGREGATOR" -- set_oracle_config \
            --asset "$sac" --config "$resolved_dual" >/dev/null
    done
    save_state STRESS_DUAL_DONE 1
}

# Liquidation frontier under dual-source: k collaterals + 1 or k debt assets,
# crash, then probe partial liquidations for growing collateral and debt width.
flow_stress_liq_frontier() {
    phase stress_liq_frontier
local k i args acct var debt_args repay_args full_args
    for k in 3 4 5 6 8; do
        var="LIQF_ACCT_$k"
        if [ -z "${!var:-}" ]; then
            args=""
            for i in $(seq 0 $((k - 1))); do args+=" $(stress_sac $i) $((1000 * STRESS_UNIT))"; done
            acct=$(inv "liqf_supply_${k}coll" "$DAVE" "$CONTROLLER" -- supply \
                --caller "$DAVE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
                --assets "$(pay_vec "$PRIMARY_HUB_ID" $args)" | tr -d '"') || continue
            inv "liqf_borrow_${k}coll" "$DAVE" "$CONTROLLER" -- borrow \
                --caller "$DAVE_ADDR" --account_id "$acct" \
                --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$(stress_sac 19)" $((k * 600 * STRESS_UNIT)))" --to null >/dev/null || continue
            save_state "$var" "$acct"
        fi
    done
    if [ -z "${LIQF_ACCT_8C8D:-}" ]; then
        args=""
        for i in $(seq 0 7); do args+=" $(stress_sac $i) $((1000 * STRESS_UNIT))"; done
        acct=$(inv liqf_supply_8coll_8debt "$DAVE" "$CONTROLLER" -- supply \
            --caller "$DAVE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --assets "$(pay_vec "$PRIMARY_HUB_ID" $args)" | tr -d '"') || return 1
        debt_args=""
        for i in $(seq 10 17); do debt_args+=" $(stress_sac $i) $((600 * STRESS_UNIT))"; done
        inv liqf_borrow_8coll_8debt "$DAVE" "$CONTROLLER" -- borrow \
            --caller "$DAVE_ADDR" --account_id "$acct" \
            --borrows "$(pay_vec "$PRIMARY_HUB_ID" $debt_args)" --to null >/dev/null || return 1
save_state LIQF_ACCT_8C8D "$acct"
fi
if [ -z "${LIQF_ACCT_10C10D:-}" ]; then
args=""
for i in $(seq 0 9); do args+=" $(stress_sac $i) $((1000 * STRESS_UNIT))"; done
acct=$(inv liqf_supply_10coll_10debt "$DAVE" "$CONTROLLER" -- supply \
--caller "$DAVE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
--assets "$(pay_vec "$PRIMARY_HUB_ID" $args)" | tr -d '"') || return 1
debt_args=""
for i in $(seq 10 19); do debt_args+=" $(stress_sac $i) $((600 * STRESS_UNIT))"; done
inv liqf_borrow_10coll_10debt "$DAVE" "$CONTROLLER" -- borrow \
--caller "$DAVE_ADDR" --account_id "$acct" \
--borrows "$(pay_vec "$PRIMARY_HUB_ID" $debt_args)" --to null >/dev/null || return 1
save_state LIQF_ACCT_10C10D "$acct"
fi
# Crash all collateral-side prices 40% (primary + anchor in lock-step).
    for i in $(seq 0 9); do
        dual_px "$(stress_sac $i)" "$(stress_code $i)" $((WAD / 10 * 6)) "crash_$(stress_code $i)"
    done
    local best_k=0
    for k in 3 4 5 6 8; do
        var="LIQF_ACCT_$k"
        acct="${!var:-}"
        [ -z "$acct" ] && continue
        sim_probe "probe_liquidate_${k}coll" "$CAROL" "$CONTROLLER" -- liquidate \
            --liquidator "$CAROL_ADDR" --account_id "$acct" \
            --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$(stress_sac 19)" $((100 * STRESS_UNIT)))"
        [ "$PROBE_STATUS" = ok ] && best_k=$k
    done
    save_state LIQ_FRONTIER_COLL "$best_k"
    # On-chain proof at the largest liquidatable width.
    if [ "$best_k" -gt 0 ]; then
        var="LIQF_ACCT_$best_k"
        inv "stress_liquidate_proof_${best_k}coll" "$CAROL" "$CONTROLLER" -- liquidate \
            --liquidator "$CAROL_ADDR" --account_id "${!var}" \
            --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" "$(stress_sac 19)" $((100 * STRESS_UNIT)))" >/dev/null
    fi
    repay_args=""
    for i in $(seq 10 17); do repay_args+=" $(stress_sac $i) $((100 * STRESS_UNIT))"; done
    sim_probe probe_liquidate_8coll_8debt "$CAROL" "$CONTROLLER" -- liquidate \
        --liquidator "$CAROL_ADDR" --account_id "$LIQF_ACCT_8C8D" \
        --debt_payments "$(pay_vec "$PRIMARY_HUB_ID" $repay_args)"
    save_state LIQ_FRONTIER_8C8D "$PROBE_STATUS"
if [ "$PROBE_STATUS" = ok ]; then
inv stress_liquidate_proof_8coll_8debt "$CAROL" "$CONTROLLER" -- liquidate \
--liquidator "$CAROL_ADDR" --account_id "$LIQF_ACCT_8C8D" \
--debt_payments "$(pay_vec "$PRIMARY_HUB_ID" $repay_args)" >/dev/null
fi
full_args=""
for i in $(seq 10 19); do full_args+=" $(stress_sac $i) $((700 * STRESS_UNIT))"; done
sim_probe probe_liquidate_10coll_10debt_full "$CAROL" "$CONTROLLER" -- liquidate \
--liquidator "$CAROL_ADDR" --account_id "$LIQF_ACCT_10C10D" \
--debt_payments "$(pay_vec "$PRIMARY_HUB_ID" $full_args)"
save_state LIQ_FRONTIER_10C10D_FULL "$PROBE_STATUS"
if [ "$PROBE_STATUS" = ok ]; then
if INV_FAIL_STATUS=research inv stress_liquidate_proof_10coll_10debt_full "$CAROL" "$CONTROLLER" -- liquidate \
--liquidator "$CAROL_ADDR" --account_id "$LIQF_ACCT_10C10D" \
--debt_payments "$(pay_vec "$PRIMARY_HUB_ID" $full_args)" >/dev/null; then
save_state LIQ_FRONTIER_10C10D_FULL_LIVE ok
else
save_state LIQ_FRONTIER_10C10D_FULL_LIVE research
fi
fi
if [ "${LIQ_FRONTIER_10C10D_FULL_LIVE:-}" != ok ]; then
repay_args="$(stress_sac 19) $((100 * STRESS_UNIT))"
sim_probe probe_liquidate_10coll_10debt_one_debt "$CAROL" "$CONTROLLER" -- liquidate \
--liquidator "$CAROL_ADDR" --account_id "$LIQF_ACCT_10C10D" \
--debt_payments "$(pay_vec "$PRIMARY_HUB_ID" $repay_args)"
save_state LIQ_FRONTIER_10C10D_ONE_DEBT "$PROBE_STATUS"
if [ "$PROBE_STATUS" = ok ]; then
inv stress_liquidate_proof_10coll_10debt_one_debt "$CAROL" "$CONTROLLER" -- liquidate \
--liquidator "$CAROL_ADDR" --account_id "$LIQF_ACCT_10C10D" \
--debt_payments "$(pay_vec "$PRIMARY_HUB_ID" $repay_args)" >/dev/null
fi
fi
}
