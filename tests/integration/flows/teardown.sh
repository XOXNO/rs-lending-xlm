# Zero-state teardown. Runs last in a lane: drains the DeFindex strategy when
# one exists, repays every live account's debts from the owner wallet (topped
# up by ADMIN when short), withdraws every position, claims all pool revenue,
# then asserts on chain:
#
#   - position NFT total_supply == 0 (every emptied account was burned)
#   - per market: pool borrowed, supplied and revenue at most accounting dust
#   - pool + controller token balances at most rounding dust
#
# Each market then records its residue: the value the protocol still holds
# after every participant leaves.

TEARDOWN_POOL_DUST="${TEARDOWN_POOL_DUST:-10000}"
TEARDOWN_CTRL_DUST="${TEARDOWN_CTRL_DUST:-1000}"

# Accounting residue that survives a full wind-down, in asset units.
#
# Interest accrual mints protocol-fee shares into both `revenue` and `supplied`
# (contracts/pool/src/interest.rs). burn_claimable_revenue burns nothing while
# the floor-unscaled revenue is below one unit, so those shares stay.
# get_revenue floors them to 0, but get_supplied_amount rounds half-up and
# reports 1. So `supplied == 0` can fail on a market that accrued a fee.
#
# The cap stays small, so an accounting break of real size still fails.
TEARDOWN_ACCT_DUST="${TEARDOWN_ACCT_DUST:-10}"

# Maps an NFT owner address to the run wallet alias that signs for it.
_td_wallet_alias() {
    local addr="$1" role av
    for role in ADMIN ALICE BOB CAROL DAVE EVE FRANK; do
        av="${role}_ADDR"
        if [ "${!av:-}" = "$addr" ]; then printf '%s' "${!role}"; return 0; fi
    done
    return 1
}

# Prints the asset code when `sac` is a harness-issued mock (state SAC_<code>),
# which makes it mintable by ADMIN for repay shortfalls.
_td_mock_code_for() {
    local sac="$1" v
    for v in $(compgen -A variable | grep '^SAC_'); do
        if [ "${!v:-}" = "$sac" ]; then printf '%s' "${v#SAC_}"; return 0; fi
    done
    return 1
}

# Ensures `owner` holds `pay` of `sac`: ADMIN mints a mock asset, and sends a
# classic asset other than XLM from its own balance (trustline first). A
# borrower whose proceeds went to a receiver or a swap never held the debt
# asset, so the repay transfer would otherwise fail.
_td_ensure_funds() {
    local id="$1" alias="$2" owner="$3" sac="$4" pay="$5"
    local bal code line
    bal=$(balance "$sac" "$owner") || return 1
    _uint_ge "$bal" "$pay" && return 0
    printf '%s\t%s\t%s\t%s\n' "$id" "$sac" "$bal" "$pay" >> "$RUN_DIR/cleanup-funding.tsv"
    if code=$(_td_mock_code_for "$sac"); then
        mint_to "$sac" "$code" "$owner" "$pay"
        return 0
    fi
    [ "$sac" = "$XLM_SAC" ] && return 0
    local need
    need=$(raw_add "$(raw_sub "$pay" "$bal")" 1000) || return 1
    local line
    line=$(classic_line "$sac")
    # ADMIN can be short (flash-lane receiver funding drains it): buy the
    # asset with XLM through the aggregator first.
    local admin_bal
    admin_bal=$(balance "$sac" "$ADMIN_ADDR") || return 1
    if ! _uint_ge "$admin_bal" "$need"; then
        trustline "$ADMIN" "${line%%:*}" "${line##*:}"
        swap_xlm_to "$ADMIN" "$ADMIN_ADDR" "$sac" "${TEARDOWN_SWAP_XLM:-20000000000}" \
            "td_swap_${id}_${sac:0:6}" || true
    fi
    if [ "$owner" != "$ADMIN_ADDR" ]; then
        trustline "$alias" "${line%%:*}" "${line##*:}"
        sac_transfer "$ADMIN" "$sac" "$ADMIN_ADDR" "$owner" "$need" \
            "td_topup_${id}_${sac:0:6}"
    fi
}

# Pass 1: repay every debt of `id`. Debts must clear before any withdrawals so
# pool cash is whole and no lender's exit gets blocked by open utilization.
_td_repay_account() {
    local id="$1"
    local exists owner alias pos k hub sac debt pay bal code
    exists=$(view "td_exists_$id" "$CONTROLLER" -- account_exists --account_id "$id" | tr -d '"')
    [ "$exists" = "true" ] || return 0
    owner=$(view "td_owner_$id" "$POSITION_NFT" -- owner_of --token_id "$id" | tr -d '"')
    alias=$(_td_wallet_alias "$owner") || {
        record "td_owner_unknown_$id" ok owner_of "" "" "" "" "" \
            "account $id owner $owner is not a run wallet; left as residue"
        return 0
    }
    pos=$(view "td_positions_$id" "$CONTROLLER" -- get_account_positions --account_id "$id")
    local debt_keys=()
    while IFS= read -r k; do [ -n "$k" ] && debt_keys+=("$k"); done \
        < <(jq -r '.[1] | keys[]' <<<"$pos" 2>/dev/null)
    # The map key printed by `keys[]` is itself a JSON object literal.
    for k in ${debt_keys[@]+"${debt_keys[@]}"}; do
        hub=$(jq -r '.hub_id' <<<"$k")
        sac=$(jq -r '.asset' <<<"$k")
        debt=$(_view_int "td_debt_${id}_${sac:0:6}" get_borrow_amount \
            --account_id "$id" --hub_asset "$(hub_key "$hub" "$sac")")
        [[ "$debt" =~ ^[0-9]+$ ]] || { _assert_fail "td_debt_$id" "unreadable debt"; return 1; }
        [ "$debt" != 0 ] || continue
        # Interest accrues between the read and the transaction; the
        # pool refunds any excess, so the buffer costs nothing.
        pay=$(python3 -c 'import sys; d=int(sys.argv[1]); print(d+d//50+100)' "$debt") || return 1
        _td_ensure_funds "$id" "$alias" "$owner" "$sac" "$pay"
        inv "td_repay_${id}_${sac:0:6}" "$alias" "$CONTROLLER" -- repay \
            --caller "$owner" --account_id "$id" \
            --payments "$(pay_vec "$hub" "$sac" "$pay")" >/dev/null
        assert_int_view_eq "td_debt_cleared_${id}_${sac:0:6}" 0 get_borrow_amount \
            --account_id "$id" --hub_asset "$(hub_key "$hub" "$sac")"
    done
}

# Pass 2: withdraw every position of `id`; amount 0 means "everything", and the
# controller burns the account once its last position is gone.
_td_withdraw_account() {
    local id="$1"
    local exists owner alias pos withdrawals
    exists=$(view "td_exists2_$id" "$CONTROLLER" -- account_exists --account_id "$id" | tr -d '"') || return 1
    [ "$exists" = "true" ] || return 0
    owner=$(view "td_owner2_$id" "$POSITION_NFT" -- owner_of --token_id "$id" | tr -d '"') || return 1
    alias=$(_td_wallet_alias "$owner") || return 0
    pos=$(view "td_positions2_$id" "$CONTROLLER" -- get_account_positions --account_id "$id") || return 1
    # The supported account limit is five positions; never split/retry a failed submission.
    withdrawals=$(jq -ec '.[0] | keys | if length <= 5 then map([fromjson, "0"]) else error("too many positions") end' <<<"$pos") || {
        _assert_fail "td_positions_$id" "invalid withdrawal positions or more than five assets"
        return 1
    }
    if [ "$withdrawals" != '[]' ]; then
        inv "td_withdraw_$id" "$alias" "$CONTROLLER" -- withdraw \
            --caller "$owner" --account_id "$id" \
            --withdrawals "$withdrawals" --to null >/dev/null || return 1
    fi
    assert_bool_view "td_burned_$id" false account_exists --account_id "$id"
}

snapshot_before_cleanup() {
    local market hub asset cash held role address amount line issuer balances
    local file="$RUN_DIR/before-cleanup.jsonl"
    : > "$file"
    for market in $MARKETS; do
        hub="${market%%:*}"; asset="${market##*:}"
        cash=$(_view_pool_int "before_cleanup_cash_${hub}_${asset:0:8}" get_reserves --hub_asset "$(hub_key "$hub" "$asset")") || return 1
        held=$(balance "$asset" "$POOL") || return 1
        jq -nc --arg hub "$hub" --arg asset "$asset" --arg cash "$cash" --arg held "$held" \
            '{hub:$hub,asset:$asset,cash:$cash,pool_balance:$held}' >> "$file"
        # A SAC rejects balance() for an absent classic trustline. Prove
        # absence directly at the ledger, separately from numeric view reads.
        balances='{}'
        line=$(classic_line "$asset") || return 1
        if [[ "$line" = *:* ]]; then
            issuer="${line##*:}"
            local addresses=() av
            for role in ADMIN ALICE BOB CAROL DAVE EVE FRANK; do
                av="${role}_ADDR"; address="${!av:-}"
                [ -z "$address" ] || [ "$address" = "$issuer" ] || addresses+=("$address")
            done
            "${NODE_BIN:-node}" "$INTEG_DIR/sdk/balances.mjs" "$RPC_URL" "$asset" "${line%%:*}" "$issuer" "$LOG_DIR/pre_cleanup_${asset}_trustlines.json" "${addresses[@]}" || return 1
            balances=$(cat "$LOG_DIR/pre_cleanup_${asset}_trustlines.json") || return 1
        fi
        for role in ADMIN ALICE BOB CAROL DAVE EVE FRANK CONTROLLER; do
            if [ "$role" = CONTROLLER ]; then address="$CONTROLLER"; else local av="${role}_ADDR"; address="${!av:-}"; fi
            [ -n "$address" ] || continue
            if jq -e --arg a "$address" 'has($a)' <<<"$balances" >/dev/null; then
                amount=$(jq -er --arg a "$address" '.[$a].balance | select(type=="string" and test("^[0-9]+$"))' <<<"$balances") || return 1
            else
                amount=$(balance "$asset" "$address") || return 1
            fi
            jq -nc --arg role "$role" --arg asset "$asset" --arg amount "$amount" '{role:$role,asset:$asset,balance:$amount}' >> "$file"
        done
    done
    if ! python3 - "$file" <<'PYBACKING'
import json,sys
cash,balances,seen={},{},set()
for line in open(sys.argv[1]):
    row=json.loads(line)
    if 'hub' not in row: continue
    key=(row['hub'],row['asset'])
    if key in seen: continue
    seen.add(key)
    a=row['asset']; c=int(row['cash']); b=int(row['pool_balance'])
    assert c>=0 and b>=0
    cash[a]=cash.get(a,0)+c
    assert a not in balances or balances[a]==b, 'balance changed during snapshot'
    balances[a]=b
assert cash, 'empty market snapshot'
for a,c in cash.items():
    assert balances[a]>=c, f'{a}: actual cash {balances[a]} below all-hub accounting {c}'
PYBACKING
    then
        _assert_fail pre_cleanup_conservation "cash backing mismatch; preserved before-cleanup.jsonl"
        return 1
    fi
    record pre_cleanup_conservation ok assert "" "" "" "" "" "all-hub cash backed before any cleanup mint/top-up"
}

flow_teardown() {
    phase teardown
    require_var MARKETS
    require_var POSITION_NFT
    # Financial failures remain sticky; cleanup still runs afterward.
    snapshot_before_cleanup || _assert_fail pre_cleanup_snapshot "incomplete pre-cleanup snapshot"

    # The DeFindex strategy owns its controller account, so only the
    # strategy's own withdraw can empty it. Drain it first.
    if [ -n "${STRATEGY:-}" ]; then
        local sbal
        sbal=$(view td_dfx_balance "$STRATEGY" -- balance --from "$DAVE_ADDR" | tr -d '"')
        if [[ "$sbal" =~ ^[1-9][0-9]*$ ]]; then
            inv td_dfx_withdraw_all "$DAVE" "$STRATEGY" -- withdraw \
                --amount "$sbal" --from "$DAVE_ADDR" --to "$DAVE_ADDR" >/dev/null
        fi
    fi

    # Snapshot live token ids before any burn reshuffles the enumeration.
    local total ids=() i id
    total=$(view td_nft_total "$POSITION_NFT" -- total_supply | tr -d '"')
    [[ "$total" =~ ^[0-9]+$ ]] || die td_nft_total "total_supply unreadable: '$total'"
    log "teardown: $total live account(s)"
    i=0
    while [ "$i" -lt "$total" ]; do
        id=$(view "td_token_$i" "$POSITION_NFT" -- get_token_id --index "$i" | tr -d '"')
        [[ "$id" =~ ^[0-9]+$ ]] && ids+=("$id")
        i=$((i + 1))
    done

    for id in ${ids[@]+"${ids[@]}"}; do _td_repay_account "$id"; done
    for id in ${ids[@]+"${ids[@]}"}; do _td_withdraw_account "$id"; done

    # Revenue last: with borrowed at zero nothing accrues after the claim, so
    # the post-claim zero read is stable.
    local m hub sac rev
    for m in $MARKETS; do
        hub="${m%%:*}"; sac="${m##*:}"
        rev=$(_view_pool_int "td_rev_${hub}_${sac:0:6}" get_revenue \
            --hub_asset "$(hub_key "$hub" "$sac")")
        if [[ "$rev" =~ ^[0-9]+$ ]] && [ "$rev" -gt 0 ]; then
            inv "td_claim_${hub}_${sac:0:6}" "$ADMIN" "$CONTROLLER" -- claim_revenue \
                --caller "$ADMIN_ADDR" --assets "$(hub_vec "$hub" "$sac")" >/dev/null
        fi
    done

    # The zero-state proof, then the residue report: what the protocol still
    # holds per market after every participant has left.
    assert_view_eq_at "$POSITION_NFT" td_nft_zero 0 total_supply
    local borrowed revenue supplied reserves pool_bal ctrl_bal
    for m in $MARKETS; do
        hub="${m%%:*}"; sac="${m##*:}"
        borrowed=$(_view_pool_int "td_borrowed_${hub}_${sac:0:6}" get_borrowed_amount \
            --hub_asset "$(hub_key "$hub" "$sac")")
        supplied=$(_view_pool_int "td_supplied_${hub}_${sac:0:6}" get_supplied_amount \
            --hub_asset "$(hub_key "$hub" "$sac")")
        revenue=$(_view_pool_int "td_revenue_${hub}_${sac:0:6}" get_revenue \
            --hub_asset "$(hub_key "$hub" "$sac")")
        reserves=$(_view_pool_int "td_reserves_${hub}_${sac:0:6}" get_reserves \
            --hub_asset "$(hub_key "$hub" "$sac")")
        pool_bal=$(balance "$sac" "$POOL") || return 1
        ctrl_bal=$(balance "$sac" "$CONTROLLER") || return 1

        _uint_le "$borrowed" "$TEARDOWN_ACCT_DUST" \
            || _assert_fail "td_zero_borrowed_${hub}_${sac:0:6}" \
               "borrowed=$borrowed > dust cap $TEARDOWN_ACCT_DUST"
        _uint_le "$supplied" "$TEARDOWN_ACCT_DUST" \
            || _assert_fail "td_zero_supplied_${hub}_${sac:0:6}" \
               "supplied=$supplied > dust cap $TEARDOWN_ACCT_DUST"
        _uint_le "$revenue" "$TEARDOWN_ACCT_DUST" \
            || _assert_fail "td_zero_revenue_${hub}_${sac:0:6}" \
               "revenue=$revenue > dust cap $TEARDOWN_ACCT_DUST after claim"
        _uint_le "$pool_bal" "$TEARDOWN_POOL_DUST" \
            || _assert_fail "td_pool_dust_${hub}_${sac:0:6}" "pool balance $pool_bal > dust cap $TEARDOWN_POOL_DUST"
        _uint_le "$ctrl_bal" "$TEARDOWN_CTRL_DUST" \
            || _assert_fail "td_ctrl_dust_${hub}_${sac:0:6}" "controller balance $ctrl_bal > dust cap $TEARDOWN_CTRL_DUST"
        record "td_residue_${hub}_${sac:0:6}" ok residue "" "" "" "" "" \
            "supplied=$supplied borrowed=$borrowed revenue=$revenue reserves=$reserves pool_bal=$pool_bal ctrl_bal=$ctrl_bal"
    done
    log "teardown: zero state proven for markets: $MARKETS"
}
