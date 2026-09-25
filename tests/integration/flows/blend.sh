# Live Blend TestnetV2 migrate_from_blend coverage.
# Request types (Blend v2): 0 supply, 1 withdraw, 2 supply-collateral,
# 3 withdraw-collateral, 4 borrow, 5 repay.
# Observed TestnetV2 XLM: c_factor/l_factor 0.90, rates scaled by 1e12.

blend_pool_id() {
    jq -r '.pools[0].address // empty' "$REPO_ROOT/configs/$NETWORK/blend.json"
}

blend_addr_json() {
    jq -nc --arg a "$1" '[$a]'
}

blend_addr_dup_json() {
    jq -nc --arg a "$1" '[$a,$a]'
}

blend_debt_json() {
    jq -nc --arg a "$1" --arg c "$2" '[[$a,$c]]'
}

blend_positions() {
    local label="$1" addr="$2"
    view "$label" "$BLEND_POOL" -- get_positions --address "$addr"
}

blend_maps_empty() {
    echo "$1" | jq -e 'type == "object" and .collateral == {} and .liabilities == {} and .supply == {}' >/dev/null
}

blend_map_sum() {
    python3 -c 'import json,sys; print(sum(int(v) for v in json.loads(sys.argv[1])[sys.argv[2]].values()))' "$1" "$2"
}

blend_has_map() {
    local s
    s=$(blend_map_sum "$1" "$2")
    [ -n "$s" ] && [ "$s" != "0" ]
}

blend_restore_xlm() {
    inv "${1:-blend_restore_xlm}" "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$XLM_SAC" "$PRIMARY_SPOKE_ID" true true 7000 7500 1000)" >/dev/null
}

blend_clear_xlm_flags() {
    clear_listing_flags "$1" "$PRIMARY_HUB_ID" "$XLM_SAC" "$PRIMARY_SPOKE_ID"
}

blend_restore_min_borrow() {
    inv "${1:-blend_restore_min_borrow}" "$ADMIN" "$CONTROLLER" -- set_min_borrow_collateral_usd \
        --floor_wad "${2:-0}" >/dev/null
}

blend_ctrl_xlm() {
    balance "$XLM_SAC" "$CONTROLLER"
}

blend_assert_ctrl_xlm_clean() {
    local label="$1" before="${2:-0}"
    local after
    after=$(blend_ctrl_xlm)
    assert_delta "$label" "$before" "$after" 0

}

blend_seed() {
    local label="$1" wallet="$2" addr="$3" coll="$4" supply="$5" debt="$6"
    local req
    req=$(jq -nc --arg xlm "$XLM_SAC" --argjson c "$coll" --argjson s "$supply" --argjson d "$debt" '
        []
        + (if $c > 0 then [{request_type:2, address:$xlm, amount:($c|tostring)}] else [] end)
        + (if $s > 0 then [{request_type:0, address:$xlm, amount:($s|tostring)}] else [] end)
        + (if $d > 0 then [{request_type:4, address:$xlm, amount:($d|tostring)}] else [] end)
    ')
    if [ "$req" = "[]" ]; then
        log "blend seed $label skipped: all amounts zero"
        return 1
    fi
    if inv "$label" "$wallet" "$BLEND_POOL" -- submit \
        --from "$addr" --spender "$addr" --to "$addr" --requests "$req" >/dev/null; then
        return 0
    fi

    return 1
}

# Capture shares, identity and unrelated state, never projected asset amounts.
blend_snapshot() {
    local label="$1" caller="$2" acct="$3" other="$4" owner="$5"
    local source positions attrs unrelated owner_source controller_usdc caller_usdc controller_xlm pool_xlm caller_xlm owner_xlm owner_usdc pool_usdc
    source=$(blend_positions "${label}_source" "$caller") || return 1
    positions='[{},{}]'; attrs="{\"mode\":0,\"spoke_id\":$PRIMARY_SPOKE_ID}"
    if [ "$acct" != 0 ]; then
        positions=$(view "${label}_positions" "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
        attrs=$(view "${label}_attributes" "$CONTROLLER" -- get_account_attributes --account_id "$acct") || return 1
    fi
    unrelated=$(view "${label}_unrelated" "$CONTROLLER" -- get_account_positions --account_id "$other") || return 1
    owner_source=$(blend_positions "${label}_owner_source" "$owner") || return 1
    controller_usdc=$(balance "$USDC_SAC" "$CONTROLLER") || return 1
    caller_usdc=$(balance "$USDC_SAC" "$caller") || return 1
    controller_xlm=$(balance "$XLM_SAC" "$CONTROLLER") || return 1
    pool_xlm=$(balance "$XLM_SAC" "$POOL") || return 1
    caller_xlm=$(balance "$XLM_SAC" "$caller") || return 1
    owner_xlm=$(balance "$XLM_SAC" "$owner") || return 1
    owner_usdc=$(balance "$USDC_SAC" "$owner") || return 1
    pool_usdc=$(balance "$USDC_SAC" "$POOL") || return 1
    local extra_balances='{}' token c p u o
    # The real Blend USDC token is distinct from the protocol's USDC SAC.
    if [ -n "${BLEND_USDC:-}" ]; then
        token="$BLEND_USDC"
        c=$(balance "$token" "$CONTROLLER") || return 1
        p=$(balance "$token" "$POOL") || return 1
        u=$(balance "$token" "$caller") || return 1
        o=$(balance "$token" "$owner") || return 1
        extra_balances=$(jq -nc --arg t "$token" --arg c "$c" --arg p "$p" --arg u "$u" --arg o "$o" \
            '{($t):{controller:$c,pool:$p,caller:$u,owner:$o}}') || return 1
    fi
    jq -nc --argjson extra_balances "$extra_balances" --argjson source "$source" --argjson positions "$positions" --argjson attrs "$attrs" \
        --argjson unrelated "$unrelated" --argjson owner_source "$owner_source" --arg owner "$owner" \
        --arg controller_usdc "$controller_usdc" --arg caller_usdc "$caller_usdc" \
        --arg controller_xlm "$controller_xlm" --arg pool_xlm "$pool_xlm" --arg caller_xlm "$caller_xlm" \
        --arg owner_xlm "$owner_xlm" --arg owner_usdc "$owner_usdc" --arg pool_usdc "$pool_usdc" \
        '{extra_balances:$extra_balances,source:$source,positions:$positions,attrs:$attrs,unrelated:$unrelated,owner_source:$owner_source,owner:$owner,
          controller_usdc:$controller_usdc,caller_usdc:$caller_usdc,controller_xlm:$controller_xlm,pool_xlm:$pool_xlm,caller_xlm:$caller_xlm,
          owner_xlm:$owner_xlm,owner_usdc:$owner_usdc,pool_usdc:$pool_usdc}'
}

# All rates below come from the committed migration receipt. Blend get_reserve
# accrues in simulation, so its later b_rate/d_rate cannot price this transaction.
blend_assert_migration() {
    local label="$1"; shift
    if ! python3 - "$INTEG_DIR" "$@" <<'PYBLEND'
import json,sys
from pathlib import Path
sys.path.insert(0,sys.argv[1])
from receipts import decode
before_file,after_file,receipt_file,reserves_file,coll_json,supply_json,debt_json,requested,returned,hub,asset,blend,pool,caller=sys.argv[2:]
b=json.loads(Path(before_file).read_text()); a=json.loads(Path(after_file).read_text())
receipt=json.loads(Path(receipt_file).read_text())['result']
meta=decode('TransactionMeta',receipt['resultMetaXdr'])
result=decode('TransactionResult',receipt['resultXdr'])
def scval(v):
    kind,value=next(iter(v.items()))
    if kind=='map': return {scval(p['key']):scval(p['val']) for p in value}
    if kind=='vec': return [scval(x) for x in value]
    if kind in ('i128','u128','i64','u64','u32','i32'): return int(value)
    if kind in ('symbol','address','bool','string'): return value
    raise AssertionError(f'unsupported migration evidence ScVal: {kind}')
reserves=json.loads(Path(reserves_file).read_text())
assert len({r['asset'] for r in reserves})==len(reserves)
assert len({r['index'] for r in reserves})==len(reserves)
reserve_by_asset={r['asset']:r for r in reserves}
coll,supply,debt=map(json.loads,(coll_json,supply_json,debt_json))
assert len(coll)==len(set(coll)) and len(supply)==len(set(supply))
assert len(debt)==len({x[0] for x in debt})
caps={token:int(cap) for token,cap in debt}
tokens=set(coll+supply+list(caps))
assert tokens and tokens<=reserve_by_asset.keys(), 'unknown requested reserve'
rates={}; indexes={}
for operation in meta['v4']['operations']:
    for change in operation['changes']:
        entry=change.get('updated',change.get('created',{})).get('data',{}).get('contract_data')
        if entry and entry['contract']==blend:
            for token in tokens:
                if entry['key']=={'vec':[{'symbol':'ResData'},{'address':token}]}:
                    rates[token]=scval(entry['val'])
    for event in operation['events']:
        if event['contract_id']!=pool or event['type']!='contract': continue
        body=event['body']['v0']
        if [scval(t) for t in body['topics']]!=['market','batch_state_update']: continue
        for row in scval(body['data']):
            if row[0]==int(hub) and row[1] in tokens: indexes[row[1]]=row[3:5]
assert rates.keys()==tokens and indexes.keys()==tokens, 'missing committed Blend/hub rates'
ceil=lambda n,d:(n+d-1)//d
credits={t:0 for t in tokens}; paid={t:0 for t in tokens}
for name,requested_assets in [('collateral',coll),('supply',supply),('liabilities',caps)]:
    expected={k:int(v) for k,v in b['source'][name].items()}
    for token in requested_assets:
        index=str(reserve_by_asset[token]['index'])
        shares=expected.pop(index,0); assert shares>0, f'missing requested source {name}: {token}'
        br,dr=int(rates[token]['b_rate']),int(rates[token]['d_rate'])
        assert min(br,dr,*indexes[token])>0
        if name=='liabilities':
            paid[token]=ceil(shares*dr,10**12)
            assert 0<paid[token]<caps[token], f'cap must bound repayment with nonzero refund: {token}'
        else: credits[token]+=shares*br//10**12
    assert {k:int(v) for k,v in a['source'][name].items()}==expected, f'requested source {name} not swept or unrelated reserve changed'
assert int(returned)>0 and (requested=='0' or returned==requested), 'destination account identity changed'
assert a['owner']==b['owner'] and a['attrs']==b['attrs'], 'destination owner/mode/spoke changed'
assert a['unrelated']==b['unrelated'], 'unrelated hub account changed'
if caller!=b['owner']:
    assert a['owner_source']==b['owner_source'], 'delegate swept owner Blend funds'
    assert a['owner_xlm']==b['owner_xlm'], 'delegate changed owner wallet balance'
for side in range(2):
    def positions(snapshot):
        return {(json.loads(k)['hub_id'],json.loads(k)['asset']):v for k,v in snapshot['positions'][side].items()}
    old,new=positions(b),positions(a)
    for token in tokens:
        decimals=reserve_by_asset[token]['decimals']; assert 0<=decimals<=27
        unit=10**(27-decimals); ray=10**27; si,bi=indexes[token]; cap=caps.get(token,0)
        delta=credits[token]*unit*ray//si if side==0 else ceil(cap*unit*ray,bi)-(cap-paid[token])*unit*ray//bi
        key=(int(hub),token)
        old_target=old.pop(key,{'scaled_amount':'0'}); new_target=new.pop(key,{'scaled_amount':'0'})
        assert int(new_target['scaled_amount'])-int(old_target['scaled_amount'])==delta, f'incorrect scaled {side} credit/refund: {token}'
    assert old==new, 'unrelated destination market changed'
for name in ('controller_usdc','caller_usdc','controller_xlm','owner_usdc','pool_usdc'):
    assert a[name]==b[name], f'unrelated/residual balance changed: {name}'
assert int(a['pool_xlm'])-int(b['pool_xlm'])==credits.get(asset,0)-paid.get(asset,0), 'hub XLM backing differs from Blend receipts/debt repayment'
assert int(a['caller_xlm'])-int(b['caller_xlm'])==-int(result['fee_charged']), 'migration took or refunded unexpected caller XLM'
assert a.get('extra_balances',{}).keys()==b.get('extra_balances',{}).keys()
assert tokens-{asset}<=b.get('extra_balances',{}).keys(), 'missing token balance snapshots'
for token,old in b.get('extra_balances',{}).items():
    new=a['extra_balances'][token]
    for holder in ('controller','caller','owner'):
        assert new[holder]==old[holder], f'unexpected token balance change: {token}/{holder}'
    assert int(new['pool'])-int(old['pool'])==credits.get(token,0)-paid.get(token,0), f'wrong hub backing: {token}'

PYBLEND
    then
        _assert_fail "$label" "Blend migration differs from committed rates, identity, sweep or balance reference"
        return 1
    fi
    record "$label" ok assert "" "" "" "" "" "exact committed-index shares; requested source swept; identity and unrelated balances preserved"
}

# Every successful migration goes through the same financial assertions.
blend_migrate() {
    blend_migrate_inv "$@"
}

blend_migrate_inv() { blend_migrate_checked inv "$@"; }

blend_migrate_checked() {
    local submit="$1"; shift
    local label="$1" wallet="$2" addr="$3" account_id="$4"
    local coll_json="$5" supply_json="$6" debt_json="$7"
    local before="$LOG_DIR/$label.before.json" after="$LOG_DIR/$label.after.json" owner="$addr" other="$ADMIN_ACCT" acct hash
    if [ "$account_id" != 0 ]; then
        owner=$(view "${label}_owner_before" "$POSITION_NFT" -- owner_of --token_id "$account_id" | tr -d '"') || return 1
    fi
    if [ -n "${ALICE_BLEND_ACCT:-}" ] && [ "$ALICE_BLEND_ACCT" != "$account_id" ]; then other="$ALICE_BLEND_ACCT"; fi
    blend_snapshot "${label}_before" "$addr" "$account_id" "$other" "$owner" >"$before" || return 1
    acct=$("$submit" "$label" "$wallet" "$CONTROLLER" -- migrate_from_blend \
        --caller "$addr" --account_id "$account_id" --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$coll_json" --supply_assets "$supply_json" --debt_caps "$debt_json" | tr -d '"[:space:]') || return 1
    [[ "$acct" =~ ^[1-9][0-9]*$ ]] || { _assert_fail "${label}_identity" "invalid account id '$acct'"; return 1; }
    owner=$(view "${label}_owner_after" "$POSITION_NFT" -- owner_of --token_id "$acct" | tr -d '"') || return 1
    blend_snapshot "${label}_after" "$addr" "$acct" "$other" "$owner" >"$after" || return 1
    if [ "$submit" = inv ]; then
        hash=$(extract_signing_hash "$LOG_DIR/$label.err") || return 1
    else
        hash=$(cat "$LOG_DIR/$label.hash") || return 1
    fi
    [[ "$hash" =~ ^[0-9a-f]{64}$ ]] || { _assert_fail "${label}_receipt" "missing signed transaction hash"; return 1; }
    blend_assert_migration "${label}_financial" "$before" "$after" "$LOG_DIR/$hash.receipt.json" \
        "$RUN_DIR/blend-reserves.json" "$coll_json" "$supply_json" "$debt_json" "$account_id" "$acct" \
        "$PRIMARY_HUB_ID" "$XLM_SAC" "$BLEND_POOL" "$POOL" "$addr" || return 1
    printf '%s\n' "$acct"
}

flow_blend_hub_liquidity() {
    phase blend_hub_liquidity
    if [ -n "${SEEDED:-}" ]; then
        log "hub already seeded (acct=${ADMIN_ACCT:-n/a})"
        return 0
    fi
    local acct
    acct=$(inv_create seed_xlm_hub "$ADMIN" "$CONTROLLER" -- supply \
        --caller "$ADMIN_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 20000000000)" | tr -d '"') || return 1
    save_state ADMIN_ACCT "$acct"
    save_state SEEDED 1
}

flow_blend_allowlist() {
    phase blend_allowlist
    BLEND_POOL="$(blend_pool_id)"
    [ -n "$BLEND_POOL" ] && [ "$BLEND_POOL" != "null" ] \
        || die blend_pool "no pools[0].address in configs/$NETWORK/blend.json"
    save_state BLEND_POOL "$BLEND_POOL"
    log "blend pool = $BLEND_POOL"

    # Zero balances on classic SACs still require an EOA trustline. These
    # unrelated balances are part of every migration conservation snapshot.
    local line wallet
    line=$(classic_line "$USDC_SAC") || return 1
    [[ "$line" = *:* ]] || { _assert_fail blend_trustlines "invalid USDC classic asset"; return 1; }
    for wallet in "$ADMIN" "$ALICE" "$BOB" "$CAROL" "$DAVE" "${EVE:-}" "${FRANK:-}"; do
        [ -n "$wallet" ] || continue
        trustline "$wallet" "${line%%:*}" "${line##*:}" || return 1
    done

    # Reserve names in config are descriptive; the pool's on-chain list binds
    # each position-map index to its real token address (including Blend USDC).
    local reserves asset reserve index=0 mapping='[]'
    reserves=$(view blend_reserve_addresses "$BLEND_POOL" -- get_reserve_list) || return 1
    while read -r asset; do
        reserve=$(view "blend_reserve_$index" "$BLEND_POOL" -- get_reserve --asset "$asset") || return 1
        jq -e --arg a "$asset" --argjson i "$index" '.asset == $a and .config.index == $i and (.config.decimals | type == "number")' <<<"$reserve" >/dev/null \
            || { _assert_fail blend_reserve_mapping "reserve list/index mismatch"; return 1; }
        mapping=$(jq -nc --argjson m "$mapping" --argjson r "$reserve" '$m + [{asset:$r.asset,index:$r.config.index,decimals:$r.config.decimals}]') || return 1
        index=$((index + 1))
    done < <(jq -r '.[]' <<<"$reserves")
    jq -e --arg x "$XLM_SAC" 'length > 1 and ([.[].asset] | length == (unique | length)) and any(.asset == $x)' <<<"$mapping" >/dev/null \
        || { _assert_fail blend_reserve_mapping "missing XLM or invalid distinct reserve addresses"; return 1; }
    printf '%s\n' "$mapping" > "$RUN_DIR/blend-reserves.json"
    record blend_reserve_mapping ok assert "" "" "" "" "" "$mapping"

    view blend_pool_initial "$CONTROLLER" -- is_blend_pool_approved --pool "$BLEND_POOL" >/dev/null
    inv blend_pool_approve "$ADMIN" "$CONTROLLER" -- approve_blend_pool --pool "$BLEND_POOL" >/dev/null
    assert_bool_view blend_pool_true true is_blend_pool_approved --pool "$BLEND_POOL"
    inv blend_pool_revoke "$ADMIN" "$CONTROLLER" -- revoke_blend_pool --pool "$BLEND_POOL" >/dev/null
    assert_bool_view blend_pool_false false is_blend_pool_approved --pool "$BLEND_POOL"
    inv blend_pool_reapprove "$ADMIN" "$CONTROLLER" -- approve_blend_pool --pool "$BLEND_POOL" >/dev/null
    assert_bool_view blend_pool_reapproved true is_blend_pool_approved --pool "$BLEND_POOL"
}

flow_blend_rejects() {
    phase blend_rejects
    local empty='[]'
    local xlm_coll xlm_debt
    xlm_coll=$(blend_addr_json "$XLM_SAC")
    xlm_debt=$(blend_debt_json "$XLM_SAC" 1)

    xfail blend_empty_params 'Error\(Contract, #16\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$empty" --supply_assets "$empty" --debt_caps "$empty"

    xfail blend_duplicate_debt 'Error\(Contract, #7\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$empty" --supply_assets "$empty" \
        --debt_caps "$(jq -nc --arg a "$XLM_SAC" '[[$a,"1"],[$a,"1"]]')"

    xfail blend_zero_debt_cap 'Error\(Contract, #14\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$empty" --supply_assets "$empty" \
        --debt_caps "$(blend_debt_json "$XLM_SAC" 0)"

    inv blend_pool_revoke_for_unapproved "$ADMIN" "$CONTROLLER" -- revoke_blend_pool \
        --pool "$BLEND_POOL" >/dev/null
    xfail blend_unapproved 'Error\(Contract, #42\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"
    inv blend_pool_reapprove_after_unapproved "$ADMIN" "$CONTROLLER" -- approve_blend_pool \
        --pool "$BLEND_POOL" >/dev/null

    xfail blend_unlisted_collateral 'Error\(Contract, #216\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$(blend_addr_json "$BLEND_POOL")" \
        --supply_assets "$empty" --debt_caps "$empty"

    xfail blend_unlisted_debt 'Error\(Contract, #216\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$empty" --supply_assets "$empty" \
        --debt_caps "$(blend_debt_json "$BLEND_POOL" 1)"

    xfail blend_missing_account 'Error\(Contract, #24\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 999999 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    xfail blend_spoke_zero 'Error\(Contract, #300\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id 0 \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    xfail blend_spoke_unknown 'Error\(Contract, #300\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id 999 \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    xfail blend_hub_unknown 'Error\(Contract, #43\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id 99 --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    xfail blend_spoke_no_xlm 'Error\(Contract, #307\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$SECONDARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    inv blend_pause "$ADMIN" "$CONTROLLER" -- pause >/dev/null
    xfail blend_paused 'Error\(Contract, #1000\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"
    inv blend_unpause "$ADMIN" "$CONTROLLER" -- unpause >/dev/null

    inv blend_pause_xlm "$ADMIN" "$CONTROLLER" -- set_spoke_asset_flags \
        --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" \
        --paused true --frozen false --no_seize false >/dev/null
    xfail blend_collateral_paused 'Error\(Contract, #315\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty" \
        || { blend_clear_xlm_flags blend_unpause_xlm; return 1; }
    blend_clear_xlm_flags blend_unpause_xlm

    inv blend_freeze_xlm "$ADMIN" "$CONTROLLER" -- set_spoke_asset_flags \
        --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" \
        --paused false --frozen true --no_seize false >/dev/null
    xfail blend_collateral_frozen 'Error\(Contract, #316\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty" \
        || { blend_clear_xlm_flags blend_unfreeze_xlm; return 1; }
    blend_clear_xlm_flags blend_unfreeze_xlm

    inv blend_xlm_not_coll "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$XLM_SAC" "$PRIMARY_SPOKE_ID" false true 7000 7500 1000)" >/dev/null
    xfail blend_not_collateral 'Error\(Contract, #104\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty" \
        || { blend_restore_xlm blend_restore_xlm_after_not_coll; return 1; }
    blend_restore_xlm blend_restore_xlm_after_not_coll

    inv blend_xlm_not_borr "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$XLM_SAC" "$PRIMARY_SPOKE_ID" true false 7000 7500 1000)" >/dev/null
    xfail blend_debt_not_borrowable 'Error\(Contract, #107\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$empty" --supply_assets "$empty" --debt_caps "$xlm_debt" \
        || { blend_restore_xlm blend_restore_xlm_after_not_borr; return 1; }
    blend_restore_xlm blend_restore_xlm_after_not_borr
}

flow_blend_migrate() {
    phase blend_migrate
    local reserve
    reserve=$(view blend_xlm_reserve "$BLEND_POOL" -- get_reserve --asset "$XLM_SAC") || return 1
    jq -e --arg a "$XLM_SAC" --argjson r "$reserve" \
        'any(.[]; .asset == $a and .asset == $r.asset and .index == $r.config.index and .decimals == $r.config.decimals)' \
        "$RUN_DIR/blend-reserves.json" >/dev/null \
        || { _assert_fail blend_xlm_reserve "XLM reserve no longer matches verified pool mapping"; return 1; }

    local coll_amt=2000000000
    local supply_amt=500000000
    local debt_amt=300000000
    local debt_cap=360000000
    local extra_coll=200000000
    local unhealthy_debt=1500000000
    local empty='[]'
    local xlm_coll xlm_supply xlm_debt xlm_dup
    xlm_coll=$(blend_addr_json "$XLM_SAC")
    xlm_supply=$(blend_addr_json "$XLM_SAC")
    xlm_debt=$(blend_debt_json "$XLM_SAC" "$debt_cap")
    xlm_dup=$(blend_addr_dup_json "$XLM_SAC")

    blend_restore_min_borrow blend_min_borrow_off 0
    local ctrl_xlm_before
    ctrl_xlm_before=$(blend_ctrl_xlm)
    _is_uint "${ctrl_xlm_before:-}" || { _assert_fail blend_controller_snapshot "invalid controller balance"; return 1; }
    log "controller XLM before migrates=$ctrl_xlm_before"

    # Real Blend burns bTokens on withdraw; a listed asset with 0 balance
    # reverts InvalidBTokenBurnAmount (#1217). The mock no-ops this path.
    xfail blend_zero_blend_balance 'Error\(Contract, #1217\)' "$EVE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$EVE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    # Zero Blend liability + hub debt cap: live Blend rejects a dToken burn of 0
    # (#1219). Position is unchanged, then a coll-only migrate must still work.
    blend_seed blend_seed_eve_coll "$EVE" "$EVE_ADDR" "$coll_amt" 0 0 || return 1
    xfail blend_zero_liab_cap 'Error\(Contract, #1219\)' "$EVE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$EVE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$xlm_debt"
    local eve_acct
    eve_acct=$(blend_migrate migrate_eve_coll_only "$EVE" "$EVE_ADDR" 0 \
        "$xlm_coll" "$empty" "$empty") || return 1
    save_state EVE_BLEND_ACCT "$eve_acct"
    assert_bool_view migrate_eve_exists true account_exists --account_id "$eve_acct"
    assert_int_view_positive migrate_eve_coll get_collateral_amount \
        --account_id "$eve_acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")"
    local eve_debt
    eve_debt=$(_view_int migrate_eve_debt get_borrow_amount --account_id "$eve_acct" \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")")
    if [ "$eve_debt" = "0" ]; then
        record migrate_eve_debt_free ok get_borrow_amount "" "" "" "" "" "0"
    else
        _assert_fail migrate_eve_debt_free "coll-only migrate left hub debt $eve_debt"
    fi
    assert_hf_at_least migrate_eve_hf "$eve_acct" "$WAD"

    local alice_has_debt=1
    blend_seed blend_seed_alice_debtcoll "$ALICE" "$ALICE_ADDR" "$coll_amt" "$supply_amt" "$debt_amt" || return 1
    local alice_pos
    alice_pos=$(blend_positions blend_alice_seeded "$ALICE_ADDR")
    echo "$alice_pos" | jq -e '.collateral != {} or .supply != {} or .liabilities != {}' >/dev/null \
        || die blend_alice_seeded "Alice Blend position empty after seed"
    if [ "$alice_has_debt" -eq 1 ] && ! blend_has_map "$alice_pos" liabilities; then
        log "alice seed reported debt success but Blend liabilities empty"
        _assert_fail blend_debt_seed "required Blend debt is missing"; return 1
    fi
    log "alice blend liabilities=$(blend_map_sum "$alice_pos" liabilities) seed_debt=$debt_amt cap=$debt_cap has_debt=$alice_has_debt"

    # No caller-mismatch xfail here: stellar-cli signs an auth entry for any
    # address argument whose secret is in the local keystore, whatever --source
    # is, so a lane wallet cannot act as the victim. A victim without a local
    # secret has no Blend position to migrate, so lifecycle.sh proves the same
    # caller gate (require_authorized_caller) on supply instead.
    local alice_acct
    if [ "$alice_has_debt" -eq 1 ]; then
        alice_acct=$(blend_migrate migrate_alice_debtcoll "$ALICE" "$ALICE_ADDR" 0 \
            "$xlm_coll" "$xlm_supply" "$xlm_debt") || return 1
    else
        alice_acct=$(blend_migrate migrate_alice_collsupply "$ALICE" "$ALICE_ADDR" 0 \
            "$xlm_coll" "$xlm_supply" "$empty") || return 1
    fi
    save_state ALICE_BLEND_ACCT "$alice_acct"
    assert_bool_view migrate_alice_exists true account_exists --account_id "$alice_acct"
    assert_hf_at_least migrate_alice_hf "$alice_acct" "$WAD"
    assert_int_view_positive migrate_alice_coll get_collateral_amount \
        --account_id "$alice_acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")"

    local alice_after
    alice_after=$(blend_positions blend_alice_swept "$ALICE_ADDR")
    if blend_maps_empty "$alice_after"; then
        record blend_alice_swept_empty ok get_positions "" "" "" "" "" "blend maps empty"
    else
        _assert_fail blend_alice_swept_empty "requested Blend positions remain: $alice_after"

    fi

    blend_seed blend_seed_bob_coll "$BOB" "$BOB_ADDR" "$coll_amt" 0 0 || return 1
    local bob_acct
    bob_acct=$(blend_migrate migrate_bob_coll_only "$BOB" "$BOB_ADDR" 0 \
        "$xlm_coll" "$empty" "$empty") || return 1
    save_state BOB_BLEND_ACCT "$bob_acct"
    assert_bool_view migrate_bob_exists true account_exists --account_id "$bob_acct"
    assert_int_view_positive migrate_bob_coll get_collateral_amount \
        --account_id "$bob_acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")"
    local bob_debt
    bob_debt=$(_view_int migrate_bob_debt get_borrow_amount --account_id "$bob_acct" \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")")
    if [ "$bob_debt" = "0" ]; then
        record migrate_bob_debt_free ok get_borrow_amount "" "" "" "" "" "0"
    else
        _assert_fail migrate_bob_debt_free "coll-only migrate left hub debt $bob_debt"
    fi
    assert_hf_at_least migrate_bob_hf "$bob_acct" "$WAD"

    local bob_positions_mid
    bob_positions_mid=$(view bob_coll_mid "$CONTROLLER" -- get_account_positions --account_id "$bob_acct" | jq -Sc .) || return 1
    # Already-swept Blend position: the live pool rejects the zero bToken burn.
    xfail blend_remigrate_empty 'Error\(Contract, #1217\)' "$BOB" "$CONTROLLER" -- migrate_from_blend \
        --caller "$BOB_ADDR" --account_id "$bob_acct" --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_dup" --supply_assets "$empty" --debt_caps "$empty"
    local bob_positions_same
    bob_positions_same=$(view bob_coll_after_empty "$CONTROLLER" -- get_account_positions --account_id "$bob_acct" | jq -Sc .) || return 1
    [ "$bob_positions_same" = "$bob_positions_mid" ] \
        || { _assert_fail bob_coll_unchanged_empty "failed remigrate changed stored shares"; return 1; }
    record bob_coll_unchanged_empty ok assert "" "" "" "" "" "failed remigration preserved positions"

    inv blend_manager_alice "$ADMIN" "$CONTROLLER" -- set_position_manager \
        --manager "$ALICE_ADDR" --is_active true >/dev/null
    inv blend_bob_add_delegate "$BOB" "$CONTROLLER" -- add_delegate \
        --caller "$BOB_ADDR" --account_id "$bob_acct" --delegate "$ALICE_ADDR" >/dev/null
    # migrate_from_blend always sweeps `caller`'s Blend position. A delegate
    # therefore moves the delegate's Blend funds into the owner's hub account.
    blend_seed blend_seed_alice_extra "$ALICE" "$ALICE_ADDR" "$extra_coll" 0 0 || return 1
    blend_migrate_inv migrate_bob_via_delegate "$ALICE" "$ALICE_ADDR" "$bob_acct" \
        "$xlm_coll" "$empty" "$empty" >/dev/null || return 1
    inv blend_bob_remove_delegate "$BOB" "$CONTROLLER" -- remove_delegate \
        --caller "$BOB_ADDR" --account_id "$bob_acct" --delegate "$ALICE_ADDR" >/dev/null
    inv blend_manager_alice_off "$ADMIN" "$CONTROLLER" -- set_position_manager \
        --manager "$ALICE_ADDR" --is_active false >/dev/null
    xfail blend_delegate_removed 'Error\(Contract, #44\)' "$ALICE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$ALICE_ADDR" --account_id "$bob_acct" --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    blend_seed blend_seed_carol_supply "$CAROL" "$CAROL_ADDR" 0 "$supply_amt" 0 || return 1
    local carol_acct
    carol_acct=$(blend_migrate migrate_carol_supply_only "$CAROL" "$CAROL_ADDR" 0 \
        "$empty" "$xlm_supply" "$empty") || return 1
    save_state CAROL_BLEND_ACCT "$carol_acct"
    assert_bool_view migrate_carol_exists true account_exists --account_id "$carol_acct"
    assert_int_view_positive migrate_carol_coll get_collateral_amount \
        --account_id "$carol_acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")"
    assert_hf_at_least migrate_carol_hf "$carol_acct" "$WAD"

    local dave_hub
    dave_hub=$(inv_create dave_hub_supply "$DAVE" "$CONTROLLER" -- supply \
        --caller "$DAVE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 100000000)" | tr -d '"') || return 1
    blend_seed blend_seed_dave_coll "$DAVE" "$DAVE_ADDR" "$coll_amt" 0 0 || return 1
    local dave_acct
    dave_acct=$(blend_migrate_inv migrate_dave_existing "$DAVE" "$DAVE_ADDR" "$dave_hub" \
        "$xlm_coll" "$empty" "$empty") || return 1
    assert_hf_at_least migrate_dave_hf "$dave_hub" "$WAD"
    save_state DAVE_BLEND_ACCT "$dave_hub"

    xfail blend_spoke_mismatch 'Error\(Contract, #310\)' "$DAVE" "$CONTROLLER" -- migrate_from_blend \
        --caller "$DAVE_ADDR" --account_id "$dave_hub" --spoke_id "$SECONDARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    xfail blend_wrong_owner 'Error\(Contract, #44\)' "$BOB" "$CONTROLLER" -- migrate_from_blend \
        --caller "$BOB_ADDR" --account_id "$alice_acct" --spoke_id "$PRIMARY_SPOKE_ID" \
        --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
        --collateral_assets "$xlm_coll" --supply_assets "$empty" --debt_caps "$empty"

    # Frank starts healthy on Blend (30 XLM debt, 200 XLM collateral), so the
    # no-collateral and min-borrow gates are distinguishable from hub #100.
    # A further Blend borrow then enters the window between c_factor 0.90 and
    # hub LTV 0.70.
    # The hub has no "cap too low" guard: a debt cap only bounds the
    # flash-borrowed repay, so Blend's dust floor (#1219) rejects a 1-stroop cap
    # and the hub never books a partial migration.
    local frank_has_debt=0 frank_unhealthy=0
    blend_seed blend_seed_frank_debt "$FRANK" "$FRANK_ADDR" "$coll_amt" 0 "$debt_amt"
    rc=$?
    [ "$rc" -eq 0 ] && frank_has_debt=1
    if [ "$frank_has_debt" -eq 1 ]; then
        xfail blend_cap_too_low 'Error\(Contract, #1219\)' "$FRANK" "$CONTROLLER" -- migrate_from_blend \
            --caller "$FRANK_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
            --collateral_assets "$xlm_coll" --supply_assets "$empty" \
            --debt_caps "$(blend_debt_json "$XLM_SAC" 1)"
        xfail blend_debt_without_collateral 'Error\(Contract, #100\)' "$FRANK" "$CONTROLLER" -- migrate_from_blend \
            --caller "$FRANK_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
            --collateral_assets "$empty" --supply_assets "$empty" \
            --debt_caps "$(blend_debt_json "$XLM_SAC" "$unhealthy_debt")"
        inv blend_min_borrow_high "$ADMIN" "$CONTROLLER" -- set_min_borrow_collateral_usd \
            --floor_wad 1000000000000000000000000000000000 >/dev/null
        xfail blend_min_borrow 'Error\(Contract, #126\)' "$FRANK" "$CONTROLLER" -- migrate_from_blend \
            --caller "$FRANK_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
            --collateral_assets "$xlm_coll" --supply_assets "$empty" \
            --debt_caps "$xlm_debt"
        blend_restore_min_borrow blend_min_borrow_reset 0
        blend_seed blend_seed_frank_unhealthy "$FRANK" "$FRANK_ADDR" 0 0 $((unhealthy_debt - debt_amt))
        rc=$?
        if [ "$rc" -eq 0 ]; then
            frank_unhealthy=1
            xfail blend_unhealthy_end 'Error\(Contract, #100\)' "$FRANK" "$CONTROLLER" -- migrate_from_blend \
                --caller "$FRANK_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
                --hub_id "$PRIMARY_HUB_ID" --blend_pool "$BLEND_POOL" \
                --collateral_assets "$xlm_coll" --supply_assets "$empty" \
                --debt_caps "$(blend_debt_json "$XLM_SAC" $((unhealthy_debt + unhealthy_debt / 5)))"
        else
            record blend_unhealthy_end environment-blocked submit "" "" "" "" "" \
                "Blend rejected extra borrow into the hub-unhealthy window"
        fi
    else
        record blend_frank_debt_seed environment-blocked submit "" "" "" "" "" \
            "Blend pool had no borrow liquidity; skipped cap-too-low, min-borrow, unhealthy"
    fi

    if [ "$alice_has_debt" -eq 0 ]; then
        record blend_alice_debt_seed environment-blocked submit "" "" "" "" "" \
            "Blend pool had no borrow liquidity for Alice; same-asset debt loop skipped"
    fi

    blend_assert_ctrl_xlm_clean blend_controller_xlm_clean "$ctrl_xlm_before"
}

# Obtain the pool's own USDC by borrowing against real XLM. No issuer key,
# faucet token or similarly named protocol USDC can satisfy this fixture.
flow_blend_multireserve() {
    phase blend_multireserve
    [ "$NETWORK" = testnet ] || return 1
    local asset symbol decimals actual_decimals token='' count=0 line code issuer
    while IFS= read -r asset; do
        [ "$asset" = "$XLM_SAC" ] && continue
        symbol=$(view "blend_symbol_${asset:0:8}" "$asset" -- symbol | jq -r .) || return 1
        if [ "$symbol" = USDC ]; then token="$asset"; count=$((count + 1)); fi
    done < <(jq -r '.[].asset' "$RUN_DIR/blend-reserves.json")
    if [ "$count" != 1 ] || [ "$token" = "$USDC_SAC" ]; then
        record blend_multireserve_fixture environment-blocked get_reserve_list "" "" "" "" "" \
            "requires one real Blend USDC reserve distinct from protocol USDC"
        return 1
    fi
    decimals=$(jq -r --arg a "$token" '.[] | select(.asset == $a) | .decimals' "$RUN_DIR/blend-reserves.json") || return 1
    actual_decimals=$(view blend_usdc_decimals "$token" -- decimals | tr -d '"[:space:]') || return 1
    [ "$decimals" = 7 ] && [ "$actual_decimals" = "$decimals" ] \
        || { _assert_fail blend_multireserve_fixture "USDC fixture requires verified seven decimals"; return 1; }
    save_state BLEND_USDC "$token"
    record blend_multireserve_fixture ok assert "" "" "" "" "" "real pool reserve=$token; distinct protocol USDC=$USDC_SAC; decimals=$decimals"

    # SACs need trustlines; contract tokens have no CODE:ISSUER name.
    line=$(view blend_usdc_name "$token" -- name | jq -r .) || return 1
    if [[ "$line" = *:* ]]; then
        code="${line%%:*}"; issuer="${line##*:}"
        [[ "$issuer" =~ ^G[A-Z2-7]{55}$ ]] || return 1
        trustline "$ADMIN" "$code" "$issuer" || return 1
        trustline "$EVE" "$code" "$issuer" || return 1
    fi
    create_market BLEND_USDC "$PRIMARY_HUB_ID" "$token" "$decimals" \
        "$(oracle_cfg_reflector USDC 900000000000000000 1100000000000000000)" \
        "$(asset_config_json 7500 8000 500)" || return 1

    local requests
    requests=$(jq -nc --arg x "$XLM_SAC" --arg u "$token" \
        '[{request_type:2,address:$x,amount:"10000000000"},{request_type:4,address:$u,amount:"100000000"}]')
    if ! inv blend_multireserve_fund "$ADMIN" "$BLEND_POOL" -- submit \
        --from "$ADMIN_ADDR" --spender "$ADMIN_ADDR" --to "$ADMIN_ADDR" --requests "$requests" >/dev/null; then
        record blend_multireserve_liquidity environment-blocked submit "" "" "" "" "" \
            "real Blend USDC borrow unavailable; cross-reserve scenarios remain required"
        return 1
    fi
    inv blend_multireserve_hub_seed "$ADMIN" "$CONTROLLER" -- supply \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$token" 90000000)" >/dev/null || return 1
    # Retain borrowed proceeds plus a real-token interest buffer for teardown.
    sac_transfer "$ADMIN" "$token" "$ADMIN_ADDR" "$EVE_ADDR" 1000000 blend_multireserve_interest_buffer || return 1

    local mode debt_json acct coll_json
    coll_json=$(blend_addr_json "$XLM_SAC")
    for mode in cross_reserve multiple_liabilities; do
        requests=$(jq -nc --arg x "$XLM_SAC" --arg u "$token" --arg mode "$mode" \
            '[{request_type:2,address:$x,amount:"2000000000"},{request_type:4,address:$u,amount:"10000000"}]
             + (if $mode == "multiple_liabilities" then [{request_type:4,address:$x,amount:"100000000"}] else [] end)') || return 1
        inv "blend_seed_$mode" "$EVE" "$BLEND_POOL" -- submit \
            --from "$EVE_ADDR" --spender "$EVE_ADDR" --to "$EVE_ADDR" --requests "$requests" >/dev/null || return 1
        debt_json=$(jq -nc --arg x "$XLM_SAC" --arg u "$token" --arg mode "$mode" \
            '[[$u,"12000000"]] + (if $mode == "multiple_liabilities" then [[$x,"120000000"]] else [] end)') || return 1
        acct=$(blend_migrate "migrate_blend_$mode" "$EVE" "$EVE_ADDR" "$EVE_BLEND_ACCT" \
            "$coll_json" '[]' "$debt_json") || return 1
        assert_hf_at_least "migrate_blend_${mode}_hf" "$acct" "$WAD" || return 1
    done
}
