flow_real_markets() {
    phase real_markets
    local xlm_band eurc_band
    xlm_band=$(reflector_band XLM) || { log "XLM live price unavailable; cannot calibrate sanity band"; return 1; }
    # EURC is a EUR stablecoin quoted in USD, so its price tracks a floating FX
    # rate and a hardcoded band goes stale. Calibrate it like XLM. USDC below
    # keeps its fixed band: it is quoted in its own unit and sits on parity.
    eurc_band=$(reflector_band EURC) || { log "EURC live price unavailable; cannot calibrate sanity band"; return 1; }
    create_market XLM "$PRIMARY_HUB_ID" "$XLM_SAC" 7 \
        "$(oracle_cfg_reflector XLM $xlm_band)" \
        "$(asset_config_json 7000 7500 1000)"
    create_market USDC "$PRIMARY_HUB_ID" "$USDC_SAC" 7 \
        "$(oracle_cfg_reflector USDC 900000000000000000 1100000000000000000)" \
        "$(asset_config_json 7500 8000 500)"
    create_market EURC "$PRIMARY_HUB_ID" "$EURC_SAC" 7 \
        "$(oracle_cfg_reflector EURC $eurc_band)" \
        "$(asset_config_json 7500 8000 500)"
}

classic_line() {
    local sac="$1"
    view "classic_name_${sac:0:8}" "$sac" -- name | jq -er 'select(type == "string" and length > 0)'
}

flow_fund_usdc() {
    phase funding
    [ -n "${FUNDED_USDC:-}" ] && return 0
    local line code issuer
    line=$(classic_line "$USDC_SAC") || return 1
    code="${line%%:*}"; issuer="${line##*:}"
    trustline "$ADMIN" "$code" "$issuer" || return 1
    trustline "$ALICE" "$code" "$issuer" || return 1
    trustline "$BOB" "$code" "$issuer" || return 1
    trustline "$CAROL" "$code" "$issuer" || return 1

    swap_xlm_to "$ADMIN" "$ADMIN_ADDR" "$USDC_SAC" 50000000000 fund_swap_usdc || return 1
    local got
    got=$(balance "$USDC_SAC" "$ADMIN_ADDR") || return 1
    _uint_ge "$got" 4 || { _assert_fail funding_balance "insufficient USDC to fund four wallets"; return 1; }
    log "admin USDC balance: $got"
    local share
    share=$(python3 -c 'import sys; print(int(sys.argv[1]) // 4)' "$got") || return 1
    sac_transfer "$ADMIN" "$USDC_SAC" "$ADMIN_ADDR" "$ALICE_ADDR" "$share" fund_alice_usdc || return 1
    sac_transfer "$ADMIN" "$USDC_SAC" "$ADMIN_ADDR" "$BOB_ADDR" "$share" fund_bob_usdc || return 1
    sac_transfer "$ADMIN" "$USDC_SAC" "$ADMIN_ADDR" "$CAROL_ADDR" "$share" fund_carol_usdc || return 1

    line=$(classic_line "$EURC_SAC") || return 1
    trustline "$ALICE" "${line%%:*}" "${line##*:}" || return 1
    swap_xlm_to "$ALICE" "$ALICE_ADDR" "$EURC_SAC" 5000000000 fund_alice_eurc || return 1
    save_state FUNDED_USDC 1
}

flow_seed_liquidity() {
    phase seed_liquidity
    [ -n "${SEEDED:-}" ] && return 0
    local usdc_left acct
    usdc_left=$(balance "$USDC_SAC" "$ADMIN_ADDR")
    [ -z "$usdc_left" ] || [ "$usdc_left" -le 0 ] && { log "no USDC to seed"; return 1; }
    acct=$(inv_create seed_supply "$ADMIN" "$CONTROLLER" -- supply \
        --caller "$ADMIN_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 20000000000 "$USDC_SAC" "$usdc_left")" | tr -d '"') || return 1
    save_state ADMIN_ACCT "$acct"
    save_state SEEDED 1
}

# Independent integer reference at the indexes committed by the operation.
lifecycle_amount() {
    python3 - "$@" <<'PYCASH'
import json,sys
method,key,before,after,state=json.loads(sys.argv[1]),json.loads(sys.argv[2]),json.loads(sys.argv[3]),json.loads(sys.argv[4]),json.loads(sys.argv[5])['state']
amount,decimals=map(int,sys.argv[6:8]); R=10**27; U=10**(27-decimals)
assert method in ['supply','borrow','repay','withdraw'] and 0<=decimals<=18 and amount>=0
side=0 if method in ['supply','withdraw'] else 1
index=int(state['supply_index' if side==0 else 'borrow_index']); assert index>0
shares=lambda p: next((int(v['scaled_amount']) for k,v in p[side].items() if json.loads(k)==key),0)
old,new=shares(before),shares(after)
ceil=lambda n,d:(n+d-1)//d
half=lambda n,d:(n+d//2)//d
if method=='supply': paid=amount; delta=amount*U*R//index
elif method=='borrow': paid=amount; delta=ceil(amount*U*R,index)
elif method=='repay':
    debt=ceil(old*index,R*U); paid=min(amount,debt)
    delta=-old if amount>=debt else -(amount*U*R//index)
else:
    displayed=half(half(old*index,R),U)
    full=amount==0 or amount>=displayed
    paid=old*index//(R*U) if full else amount
    delta=-old if full else -ceil(amount*U*R,index)
assert new-old==delta, f'{method}: shares changed {new-old}, expected {delta}'
assert paid>=0
print(paid)
PYCASH
}

# Same submission helper, with caller/pool/controller cash and share predicates.
lifecycle_inv() { lifecycle_checked inv "$@"; }

lifecycle_checked() {
    local submit="$1"; shift
    local label="$1" signer="$2" contract="$3"; shift 3
    local args=("$@") method="$2" caller='' to=null acct=0 payments=''
    shift 2
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --caller) caller="$2";; --account_id) acct="$2";; --to) to="$2";;
            --assets|--borrows|--payments|--withdrawals) payments="$2";;
        esac
        shift 2
    done
    [ -n "$caller" ] && [ -n "$payments" ] || { _assert_fail "$label" 'missing lifecycle inputs'; return 1; }
    [ "$to" != null ] || to="$caller"
    local before='[{},{}]' after row asset key n=0 result value sync paid expected who decimals
    [ "$acct" = 0 ] || before=$(view "${label}_before" "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    local wallet=() pool=() controller=()
    while read -r row; do
        asset=$(jq -r '.[0].asset' <<<"$row")
        who="$caller"; case "$method" in borrow|withdraw) who="$to";; esac
        wallet[$n]=$(financial_balance "$asset" "$who") || return 1
        pool[$n]=$(balance "$asset" "$POOL") || return 1
        controller[$n]=$(balance "$asset" "$CONTROLLER") || return 1
        n=$((n+1))
    done < <(jq -c '.[]' <<<"$payments")
    result=$("$submit" "$label" "$signer" "$contract" "${args[@]}") || return 1
    if [ "$acct" = 0 ]; then
        acct=$(tr -d '\"[:space:]' <<<"$result")
        [[ "$acct" =~ ^[1-9][0-9]*$ ]] || { _assert_fail "$label" 'invalid created account'; return 1; }
    fi
    after=$(view "${label}_after" "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    n=0
    while read -r row; do
        key=$(jq -c '.[0]' <<<"$row"); asset=$(jq -r '.[0].asset' <<<"$row"); value=$(jq -r '.[1]' <<<"$row")
        decimals=$(view "${label}_decimals_$n" "$asset" -- decimals | tr -d '\"[:space:]') || return 1
        sync=$(view "${label}_committed_$n" "$POOL" -- get_sync_data --hub_asset "$key") || return 1
        paid=$(lifecycle_amount "\"$method\"" "$key" "$before" "$after" "$sync" "$value" "$decimals") || { _assert_fail "${label}_shares_$n" 'incorrect principal or refund at committed index'; return 1; }
        who="$caller"; expected="-$paid"
        case "$method" in borrow|withdraw) who="$to"; expected="$paid";; esac
        assert_delta "${label}_wallet_$n" "${wallet[$n]}" "$(financial_balance "$asset" "$who")" "$expected" || return 1
        assert_delta "${label}_pool_$n" "${pool[$n]}" "$(balance "$asset" "$POOL")" "$(raw_sub 0 "$expected")" || return 1
        assert_delta "${label}_controller_$n" "${controller[$n]}" "$(balance "$asset" "$CONTROLLER")" 0 || return 1
        record "${label}_shares_$n" ok assert "" "" "" "" "" 'exact committed-index principal and refund'
        n=$((n+1))
    done < <(jq -c '.[]' <<<"$payments")
    printf '%s\n' "$result"
}

flow_lifecycle() {
    phase lifecycle
    local acct
    acct=$(lifecycle_inv supply_create "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 10000000000)" | tr -d '"')
    save_state ALICE_ACCT "$acct"
    log "alice account = $acct"

    # Caller gate (require_authorized_caller): the CLI signs auth entries for
    # every address argument it holds a secret for, so the victim must be an
    # address whose secret lives only in a throwaway config dir. Friendbot
    # funds it so the simulated supply runs end to end; the only thing the
    # CLI cannot produce is the victim's signature.
    local victim_dir victim
    victim_dir=$(mktemp -d)
    stellar keys generate --config-dir "$victim_dir" victim >/dev/null 2>&1
    victim=$(stellar keys address --config-dir "$victim_dir" victim)
    rm -rf "$victim_dir"
    curl -s -m 30 "https://friendbot.stellar.org/?addr=$victim" >/dev/null 2>&1 || true
    xfail caller_auth_required "Missing signing key for account $victim" "$BOB" "$CONTROLLER" -- supply \
        --caller "$victim" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 10000000)"

    local usdc_bal usdc_half
    usdc_bal=$(balance "$USDC_SAC" "$ALICE_ADDR") || return 1
    _uint_ge "$usdc_bal" 2 || { _assert_fail supply_bulk "Alice needs a positive USDC fixture balance"; return 1; }
    usdc_half=$(python3 -c 'import sys;print(int(sys.argv[1])//2)' "$usdc_bal") || return 1
    lifecycle_inv supply_bulk "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id "$acct" --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 5000000000 "$USDC_SAC" "$usdc_half")" >/dev/null || return 1

    # The supply must register collateral. Nothing is borrowed yet, so the
    # account is healthy and its collateral must price above zero.
    assert_hf_at_least hf_alice "$acct" "$WAD"
    assert_int_view_positive coll_usd_alice get_total_collateral_usd --account_id "$acct"
    assert_int_view_positive coll_xlm_alice_supplied get_collateral_amount \
        --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")"
    view ltv_usd_alice "$CONTROLLER" -- get_ltv_collateral_usd --account_id "$acct" >/dev/null
    view attrs_alice "$CONTROLLER" -- get_account_attributes --account_id "$acct" >/dev/null
view positions_alice "$CONTROLLER" -- get_account_positions --account_id "$acct" >/dev/null
view indexes_view "$CONTROLLER" -- get_market_indexes_detailed \
--hub_assets "$(hub_vec "$PRIMARY_HUB_ID" "$XLM_SAC" "$USDC_SAC" "$EURC_SAC")" >/dev/null

    lifecycle_inv borrow_single "$ALICE" "$CONTROLLER" -- borrow \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 200000000)" --to null >/dev/null
    lifecycle_inv borrow_bulk "$ALICE" "$CONTROLLER" -- borrow \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 150000000 "$XLM_SAC" 1000000000)" --to null >/dev/null
    local borrow_usd
    borrow_usd=$(_view_int borrow_usd_alice get_total_borrow_usd --account_id "$acct")
    _uint_ge "$borrow_usd" 1 || _assert_fail borrow_usd_alice "total_borrow_usd=$borrow_usd want > 0"
    assert_hf_at_least hf_alice_post_borrow "$acct" "$WAD"
    assert_borrow_at_least debt_usdc_post_borrow "$acct" "$USDC_SAC" 200000000

    assert_bool_view account_exists_alice true account_exists --account_id "$acct"
    assert_int_view_eq pool_addr_view "$POOL" get_pool_address

    xfail supply_zero 'Error\(Contract, #14\)' "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id "$acct" --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 0)"

    # Both guards below must fail on collateral (#100). Alice's collateral
    # (1.5k XLM plus half her USDC) far exceeds her debt, so a fixed over-borrow
    # stays inside her limit, and a withdrawal large enough to breach LTV first
    # exhausts pool liquidity (#112). So the flow first borrows most of her
    # borrowing power and derives both amounts from it. The repays below read
    # the debt at runtime, so they also clear this borrow.
    local ltv_wad debt_wad headroom_usdc over_usdc
    ltv_wad=$(_view_int ltv_usd_pre_edge get_ltv_collateral_usd --account_id "$acct")
    debt_wad=$(_view_int borrow_usd_pre_edge get_total_borrow_usd --account_id "$acct")
    # USD is WAD-scaled (1e18) and USDC has 7 decimals, so 1 USDC unit == 1e11.
    # Borrow 90% of the headroom: enough to leave the limit within reach, with
    # room for a price tick between this read and the transaction.
    headroom_usdc=$(python3 -c 'import sys;print((int(sys.argv[1])-int(sys.argv[2]))*9//10**12)' "$ltv_wad" "$debt_wad")
    if [ -z "$headroom_usdc" ] || [ "$headroom_usdc" -lt 1000000 ]; then
        _assert_fail borrow_to_ltv_edge "no borrowing headroom to set up the LTV guards (got ${headroom_usdc:-<none>})"
    else
        lifecycle_inv borrow_to_ltv_edge "$ALICE" "$CONTROLLER" -- borrow \
            --caller "$ALICE_ADDR" --account_id "$acct" \
            --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" "$headroom_usdc")" --to null >/dev/null
    fi

    # Half the original headroom exceeds the ~10% that is left, and in USDC it
    # stays inside what the pool can lend, so the revert is #100 and not #112.
    over_usdc=$(python3 -c 'import sys;print((int(sys.argv[1])-int(sys.argv[2]))//(2*10**11))' "$ltv_wad" "$debt_wad")
    xfail borrow_over_ltv 'Error\(Contract, #100\)' "$ALICE" "$CONTROLLER" -- borrow \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" "$over_usdc")" --to null

    # Withdrawing all the XLM and half the USDC removes far more borrowing power
    # than the headroom left, and stays inside what the pool can pay out, so the
    # revert is #100 and not #112.
    local xlm_coll_pre usdc_coll_pre
    xlm_coll_pre=$(_view_int coll_xlm_pre_lock get_collateral_amount \
        --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")")
    usdc_coll_pre=$(_view_int coll_usdc_pre_lock get_collateral_amount \
        --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    xfail_sim withdraw_locked 'Error\(Contract, #100\)' "$ALICE" "$CONTROLLER" -- withdraw \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --withdrawals "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" "$xlm_coll_pre" "$USDC_SAC" $((usdc_coll_pre / 2)))" --to null

    local usdc_debt_pre_partial
usdc_debt_pre_partial=$(_view_int debt_usdc_pre_partial get_borrow_amount \
--account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    lifecycle_inv repay_partial "$ALICE" "$CONTROLLER" -- repay \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --payments "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 100000000)" >/dev/null
    assert_borrow_decreased debt_usdc_post_partial "$acct" "$USDC_SAC" "$usdc_debt_pre_partial"
    local usdc_debt xlm_debt
usdc_debt=$(view debt_usdc_alice "$CONTROLLER" -- get_borrow_amount \
--account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" | tr -d '"')
xlm_debt=$(view debt_xlm_alice "$CONTROLLER" -- get_borrow_amount \
--account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" | tr -d '"')
    lifecycle_inv repay_full_bulk "$ALICE" "$CONTROLLER" -- repay \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --payments "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" $((usdc_debt + 10000000)) "$XLM_SAC" $((xlm_debt + 10000000)))" >/dev/null
    assert_borrow_at_most debt_usdc_cleared "$acct" "$USDC_SAC" 0
    assert_borrow_at_most debt_xlm_cleared "$acct" "$XLM_SAC" 0

    leg_borrow_again() {
        lifecycle_inv borrow_again "$ALICE" "$CONTROLLER" -- borrow \
            --caller "$ALICE_ADDR" --account_id "$acct" \
            --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 120000000)" --to null >/dev/null
        local debt_after_borrow
debt_after_borrow=$(view debt_usdc_alice "$CONTROLLER" -- get_borrow_amount \
--account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" | tr -d '"')
        if [ -z "$debt_after_borrow" ] || [ "$debt_after_borrow" -lt 120000000 ]; then
            log "borrow_again: USDC debt too low ($debt_after_borrow) for cross-account repay"
            return 1
        fi
    }
    retry_leg leg_borrow_again
    leg_repay_cross_account() {
        lifecycle_inv repay_cross_account "$BOB" "$CONTROLLER" -- repay \
            --caller "$BOB_ADDR" --account_id "$acct" \
            --payments "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 130000000)" >/dev/null
    }
    retry_leg leg_repay_cross_account

    lifecycle_inv withdraw_partial "$ALICE" "$CONTROLLER" -- withdraw \
        --caller "$ALICE_ADDR" --account_id "$acct" \
        --withdrawals "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 5000000000)" --to null >/dev/null
    inv renew_account "$ALICE" "$CONTROLLER" -- renew_account \
        --caller "$ALICE_ADDR" --account_id "$acct" >/dev/null
local xlm_coll usdc_coll
xlm_coll=$(view coll_xlm_alice "$CONTROLLER" -- get_collateral_amount \
--account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" | tr -d '"')
usdc_coll=$(view coll_usdc_alice "$CONTROLLER" -- get_collateral_amount \
--account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" | tr -d '"')

    leg_withdraw_full_bulk() {
        lifecycle_inv withdraw_full_bulk "$ALICE" "$CONTROLLER" -- withdraw \
            --caller "$ALICE_ADDR" --account_id "$acct" \
            --withdrawals "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 0 "$USDC_SAC" 0)" --to null >/dev/null
    }
    retry_leg leg_withdraw_full_bulk

    # Amount 0 withdraws the full position, so both legs must end at zero.
    assert_int_view_eq withdraw_xlm_drained 0 get_collateral_amount \
        --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")"
    assert_int_view_eq withdraw_usdc_drained 0 get_collateral_amount \
        --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")"
}
