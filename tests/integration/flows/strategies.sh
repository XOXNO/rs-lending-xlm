flash_data_hex() {
    local mode="$1"
    jq -nc --argjson m "$mode" '{map:[{key:{symbol:"mode"},val:{u32:$m}}]}' \
        | stellar xdr encode --type ScVal | base64 -d | xxd -p | tr -d '\n'
}

flash_loan_fee_state() {
    python3 - "$@" <<'PYFEE'
import json,sys
p,q=map(json.loads,sys.argv[1:3]);fee=int(sys.argv[3]);unit=10**(27-int(sys.argv[4]));R=10**27
assert int(p['borrowed'])==int(q['borrowed'])==0, 'fee fixture must have no outstanding debt'
assert int(p['supply_index'])==int(q['supply_index']), 'unexpected index drift without borrowing'
shares=fee*unit*R//int(q['supply_index'])
assert int(q['revenue'])-int(p['revenue'])==shares, 'wrong fee destination'
assert int(q['supplied'])-int(p['supplied'])==shares, 'wrong protocol shares'
assert int(q['cash'])-int(p['cash'])==fee, 'wrong fee backing'
PYFEE
}

flash_loan_checked() {
    local label="$1" mode="$2" key params fee before after recv cash ctrl caller
    key=$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")
    params=$(view "${label}_params" "$POOL" -- get_sync_data --hub_asset "$key" | jq -ce 'select(.params | type == "object") | .params') || return 1
    fee=$(python3 -c 'import json,sys;p=json.loads(sys.argv[1]);b=int(p["flashloan_fee"]);assert 0<b<=10000;print(max(1,(100000000*b+5000)//10000))' "$params") || return 1
    before=$(view "${label}_state_before" "$POOL" -- get_sync_data --hub_asset "$key" | jq -c '.state') || return 1
    recv=$(balance "$USDC_SAC" "$FLASH_RECEIVER") || return 1
    cash=$(balance "$USDC_SAC" "$POOL") || return 1
    ctrl=$(balance "$USDC_SAC" "$CONTROLLER") || return 1
    caller=$(balance "$USDC_SAC" "$ALICE_ADDR") || return 1
    inv "$label" "$ALICE" "$CONTROLLER" -- flash_loan --caller "$ALICE_ADDR" --asset "$key" --amount 100000000 \
        --receiver "$FLASH_RECEIVER" --data "$(flash_data_hex "$mode")" >/dev/null || return 1
    after=$(view "${label}_state_after" "$POOL" -- get_sync_data --hub_asset "$key" | jq -c '.state') || return 1
    flash_loan_fee_state "$before" "$after" "$fee" "$(jq -r '.asset_decimals' <<<"$params")" || { _assert_fail "${label}_financial" 'protocol fee share accounting differs'; return 1; }
    assert_delta "${label}_receiver_fee" "$recv" "$(balance "$USDC_SAC" "$FLASH_RECEIVER")" "-$fee" || return 1
    assert_delta "${label}_pool_fee" "$cash" "$(balance "$USDC_SAC" "$POOL")" "$fee" || return 1
    assert_delta "${label}_controller" "$ctrl" "$(balance "$USDC_SAC" "$CONTROLLER")" 0 || return 1
    assert_delta "${label}_caller" "$caller" "$(balance "$USDC_SAC" "$ALICE_ADDR")" 0 || return 1
    record "${label}_financial" ok assert "" "" "" "" "" 'exact fee shares and receiver/pool/controller/caller conservation'
}

flow_flash_loans() {
    phase flash_loans

    sac_transfer "$ALICE" "$USDC_SAC" "$ALICE_ADDR" "$FLASH_RECEIVER" 50000000 fund_flash_receiver

    inv flash_loan_set_plan "$ALICE" "$FLASH_RECEIVER" -- set_plan \
        --controller "$CONTROLLER" --hub_id "$PRIMARY_HUB_ID" \
        --spoke_id "$PRIMARY_SPOKE_ID" --account_id 0 >/dev/null \
        || die flash_loan_set_plan "set_plan must succeed so reentry hits this controller"

    flash_loan_checked flash_loan_success 0 || return 1
    record flash_loan_fee_booked ok flash_loan "" "" "" "" "" 'exact protocol fee shares verified'
    flash_loan_checked flash_loan_over_repay 6 || return 1

    local mode name pattern
    # The Soroban host rejects re-entry into a contract already on the call
    # stack with Context/InvalidAction, before the controller's #400 guard runs.
    # A live callback sees only that host error; the harness test
    # meta/reentrancy_matrix.rs pins #400.
    local re_pattern='Error\(Context, InvalidAction\)'
    for mode in 1 2 3 4 5 7 8 9 10 11 12 13 14 15 16 17 18; do
        case $mode in
            1) name=no_repay; pattern='Error\(Contract, #402\)' ;;
            2) name=under_repay; pattern='Error\(Contract, #402\)' ;;
            3) name=reenter_pool; pattern="$re_pattern" ;;
            4) name=panic; pattern='Error\(Contract, #3\)' ;;
            5) name=reenter_supply; pattern="$re_pattern" ;;
            7) name=push_to_pool; pattern='Error\(Contract, #402\)' ;;
            8) name=reenter_borrow; pattern="$re_pattern" ;;
            9) name=reenter_withdraw; pattern="$re_pattern" ;;
            10) name=reenter_repay; pattern="$re_pattern" ;;
            11) name=reenter_flash_loan; pattern="$re_pattern" ;;
            12) name=reenter_flash_position; pattern="$re_pattern" ;;
            13) name=reenter_multiply; pattern="$re_pattern" ;;
            14) name=reenter_swap_debt; pattern="$re_pattern" ;;
            15) name=reenter_swap_collateral; pattern="$re_pattern" ;;
            16) name=reenter_rdwc; pattern="$re_pattern" ;;
            17) name=reenter_liquidate; pattern="$re_pattern" ;;
            18) name=reenter_migrate; pattern="$re_pattern" ;;
        esac
        xfail "flash_loan_$name" "$pattern" "$ALICE" "$CONTROLLER" -- flash_loan \
            --caller "$ALICE_ADDR" --asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" --amount 100000000 \
            --receiver "$FLASH_RECEIVER" --data "$(flash_data_hex $mode)"
    done
}

# Snapshot the two routed fixture assets; use fee-adjusted XLM wallet balances.
strategy_snapshot() {
    local label="$1" caller="$2" acct="$3" positions='[{},{}]' tokens='{}' token wallet cash ctrl sync
    local exists=false owner=null attrs=null
    if [ "$acct" != 0 ]; then
        exists=$(view "${label}_exists" "$CONTROLLER" -- account_exists --account_id "$acct" | tr -d '[:space:]') || return 1
        [[ "$exists" = true || "$exists" = false ]] || return 1
        positions=$(view "${label}_positions" "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
        # A closed account has no NFT or attributes: do not call reverting views.
        if [ "$exists" = true ]; then
            owner=$(view "${label}_owner" "$POSITION_NFT" -- owner_of --token_id "$acct") || return 1
            attrs=$(view "${label}_attributes" "$CONTROLLER" -- get_account_attributes --account_id "$acct") || return 1
        fi
    fi
    for token in "$XLM_SAC" "$USDC_SAC"; do
        wallet=$(financial_balance "$token" "$caller") || return 1
        cash=$(balance "$token" "$POOL") || return 1
        ctrl=$(balance "$token" "$CONTROLLER") || return 1
        sync=$(view "${label}_${token:0:8}_sync" "$POOL" -- get_sync_data --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$token")") || return 1
        tokens=$(jq -cn --argjson old "$tokens" --arg token "$token" --arg wallet "$wallet" --arg pool "$cash" --arg controller "$ctrl" --argjson sync "$sync" '$old+{($token):{wallet:$wallet,pool:$pool,controller:$controller,sync:$sync}}') || return 1
    done
    jq -cn --argjson positions "$positions" --argjson tokens "$tokens" --argjson exists "$exists" --argjson owner "$owner" --argjson attrs "$attrs" \
        '{positions:$positions,tokens:$tokens,exists:$exists,owner:$owner,attrs:$attrs}'
}

strategy_assert_financial() {
    local label="$1"; shift
    if ! python3 - "$INTEG_DIR" "$@" <<'PYSTRATEGY'
import base64,copy,json,subprocess,sys
from pathlib import Path
sys.path.insert(0,sys.argv[1])
from receipts import decode
before_file,after_file,receipt_file,request_file,account_id,controller,router=sys.argv[2:]
b,a,receipt,request=[json.loads(Path(p).read_text()) for p in (before_file,after_file,receipt_file,request_file)]
method=request.pop('method'); caller=request['--caller']; key=lambda name:json.loads(request[name])
assert method in ('multiply','swap_debt','swap_collateral','repay_debt_with_collateral')
if method=='multiply': source,dest=key('--debt'),key('--collateral')
elif method=='swap_debt': source,dest=key('--new_debt'),key('--existing_debt')
elif method=='swap_collateral': source,dest=key('--current'),key('--new')
else: source,dest=key('--collateral'),key('--debt')
ti,to=source['asset'],dest['asset']; assert ti!=to and set(b['tokens'])==set(a['tokens'])=={ti,to}
def sc(v):
    kind,value=next(iter(v.items()))
    if kind=='map': return {sc(p['key']):sc(p['val']) for p in value}
    if kind=='vec': return [sc(x) for x in value]
    if kind in ('i128','u128','i64','u64','u32','i32'): return int(value)
    if kind in ('symbol','address','bool','string','bytes'): return value
    raise AssertionError(f'unsupported strategy evidence: {kind}')
meta=decode('TransactionMeta',receipt['result']['resultMetaXdr'])
events=[e for op in meta['v4']['operations'] for e in op['events']] if 'v4' in meta else meta['v3']['soroban_meta']['events']
transfers=[]; batches=[]
for event in events:
    if event['type']!='contract': continue
    body=event['body']['v0']; topics=[sc(t) for t in body['topics']]
    if event['contract_id'] in (ti,to) and topics and topics[0]=='transfer':
        amount=sc(body['data']); amount=amount['amount'] if isinstance(amount,dict) else amount
        assert isinstance(amount,int) and amount>=0
        transfers.append((event['contract_id'],topics[1],topics[2],amount))
    if event['contract_id']==controller and topics==['position','batch_update']: batches.append(sc(body['data']))
assert len(batches)==1 and int(batches[0][0])==int(account_id), 'missing or wrong strategy account event'
close=request.get('--close_position')=='true'
if request['--account_id']=='0':
    assert method=='multiply' and b['exists'] is False and b['owner'] is None and b['attrs'] is None
    owner=caller; attrs={'spoke_id':int(request['--spoke_id']),'mode':int(request['--mode'])}
else:
    assert int(request['--account_id'])==int(account_id) and b['exists'] is True
    owner=b['owner']; attrs=b['attrs']
    assert isinstance(owner,str) and owner and isinstance(attrs,dict)
assert batches[0][1]==[owner,int(attrs['spoke_id']),int(attrs['mode'])], 'strategy event changed owner, spoke or mode'
if close:
    assert a['exists'] is False and a['owner'] is None and a['attrs'] is None, 'closed account still exists'
else:
    assert a['exists'] is True and a['owner']==owner and a['attrs']==attrs, 'strategy changed persisted owner, spoke or mode'
move=lambda token,fr,to_:sum(n for t,f,d,n in transfers if (t,f,d)==(token,fr,to_))
spent=move(ti,controller,router)-move(ti,router,controller)
output=move(to,router,controller)
assert spent>0 and output>0, 'missing routed input or wrong output destination'
route=sc(decode('ScVal',base64.b64encode(bytes.fromhex(request['--swap'])).decode()))
ops=bytes.fromhex(route['ops']); assert len(ops)>=10 and ops[0]==1
assert (route['assets'][ops[1]],route['assets'][ops[2]])==(ti,to), 'route assets differ'
assert output>=int(route['amounts'][ops[3]])>0, 'output below signed route minimum'
expected=copy.deepcopy(b['positions']); cash={t:0 for t in (ti,to)}; wallet=cash.copy()
pool_state={}
for token in (ti,to):
    accrued=subprocess.run(['bash','-c','source "$1"; pool_accrual_at_committed_index "$2" "$3"','_',str(Path(sys.argv[1])/'lib/assert.sh'),json.dumps(b['tokens'][token]['sync']),json.dumps(a['tokens'][token]['sync'])],capture_output=True,text=True)
    assert accrued.returncode==0, accrued.stderr
    pool_state[token]={k:int(v) for k,v in json.loads(accrued.stdout)['state'].items()}
legs={}; initial=0
if method=='multiply':
    payment,initial=json.loads(request['--initial_payment']); initial=int(initial)
    assert payment==dest and request['--convert_swap']=='null'
    wallet[to]-=initial
    legs={(1,6,ti):('borrow',int(request['--debt_to_flash_loan'])),(0,0,to):('supply',initial+output)}
elif method=='swap_debt': legs={(1,8,ti):('borrow',int(request['--amount'])),(1,8,to):('repay',output)}
elif method=='swap_collateral': legs={(0,9,ti):('withdraw',int(request['--amount'])),(0,0,to):('supply',output)}
else:
    legs={(0,10,ti):('withdraw',int(request['--collateral_amount'])),(1,11,to):('repay',output)}
seen=set(); available=None
for side,rows in enumerate(batches[0][2:4]):
    for row in rows:
        action,hub,token,shares,index,event_amount=row[:6]
        k={'hub_id':hub,'asset':token}; encoded=json.dumps(k)
        old=next((v for q,v in expected[side].items() if json.loads(q)==k),{'scaled_amount':'0'})
        if action==7:
            assert shares==int(old['scaled_amount']), 'risk refresh changed principal'
            continue
        tag=(side,action,token)
        assert tag in legs and tag not in seen, 'unexpected or duplicate strategy leg'
        seen.add(tag); operation,offered=legs[tag]
        assert k==(source if token==ti else dest), 'wrong hub in strategy leg'
        updated=copy.deepcopy(expected)
        updated[side]={q:v for q,v in updated[side].items() if json.loads(q)!=k}
        if shares: updated[side][encoded]={'scaled_amount':str(shares)}
        sync=a['tokens'][token]['sync']; field='supply_index' if side==0 else 'borrow_index'
        assert int(sync['state'][field])==index, 'event index differs from committed market'
        args=[json.dumps(v) for v in (operation,k,expected,updated,sync)]+[str(offered),str(sync['params']['asset_decimals'])]
        check=subprocess.run(['bash','-c','source "$1"; shift; lifecycle_amount "$@"','_',str(Path(sys.argv[1])/'flows/lifecycle.sh'),*args],capture_output=True,text=True)
        assert check.returncode==0, check.stderr
        paid=int(check.stdout); assert event_amount==paid, 'event cash differs from principal reference'
        expected=updated
        pool_state[token]['supplied' if side==0 else 'borrowed']+=shares-int(old['scaled_amount'])
        if close and action==10 and shares: legs[(0,12,ti)]=('withdraw',0)
        if operation=='borrow':
            bps=int(b['tokens'][token]['sync']['params']['flashloan_fee'])
            fee=max(1,(paid*bps+5000)//10000) if bps else 0
            available=paid-fee; cash[token]-=available
            unit=10**(27-int(sync['params']['asset_decimals']))
            fee_shares=min(fee*unit*10**27//pool_state[token]['supply_index'],2**127-1-pool_state[token]['supplied'])
            pool_state[token]['revenue']+=fee_shares; pool_state[token]['supplied']+=fee_shares
        elif operation=='supply': cash[token]+=paid
        elif operation=='repay':
            cash[token]+=paid; wallet[token]+=offered-paid
        else:
            cash[token]-=paid
            if action==12: wallet[token]+=paid
            else: available=paid
assert seen==set(legs), 'missing strategy principal leg'
assert available is not None and 0<spent<=available, 'incorrect strategy funding'
wallet[ti]+=available-spent
normalize=lambda maps:[{(json.loads(k)['hub_id'],json.loads(k)['asset']):int(v['scaled_amount']) for k,v in p.items() if int(v['scaled_amount'])} for p in maps]
assert normalize(expected)==normalize(a['positions']), 'position state differs or unrelated principal changed'
if close: assert normalize(a['positions'])==[{},{}], 'close left principal'
for token in (ti,to):
    pool_state[token]['cash']+=cash[token]
    assert {k:int(v) for k,v in a['tokens'][token]['sync']['state'].items()}==pool_state[token], 'wrong pool principal, cash or protocol fee destination'
    for holder,want in [('pool',cash[token]),('wallet',wallet[token]),('controller',0)]:
        assert int(a['tokens'][token][holder])-int(b['tokens'][token][holder])==want, f'wrong {token}/{holder} cash or refund'
PYSTRATEGY
    then
        _assert_fail "$label" 'routed strategy differs from committed principal, cash, destination or refund reference'
        return 1
    fi
    record "$label" ok assert "" "" "" "" "" 'exact routed identity, principal, accrued pool books, fee destination, cash and refunds'
}

strategy_inv() {
    local label="$1" signer="$2" contract="$3"; shift 3
    local args=("$@") method="$2" caller='' acct=0 result hash
    local before="$LOG_DIR/$label.before.json" after="$LOG_DIR/$label.after.json" request="$LOG_DIR/$label.request.json"
    shift 2
    python3 -c 'import json,sys;print(json.dumps(dict(zip(sys.argv[2::2],sys.argv[3::2]),method=sys.argv[1])))' "$method" "$@" >"$request" || return 1
    while [ "$#" -gt 0 ]; do
        case "$1" in --caller) caller="$2";; --account_id) acct="$2";; esac
        shift 2
    done
    strategy_snapshot "${label}_before" "$caller" "$acct" >"$before" || return 1
    result=$(inv "$label" "$signer" "$contract" "${args[@]}") || return 1
    if [ "$acct" = 0 ]; then
        acct=$(tr -d '\"[:space:]' <<<"$result")
        [[ "$acct" =~ ^[1-9][0-9]*$ ]] || { _assert_fail "$label" 'invalid strategy account'; return 1; }
    fi
    strategy_snapshot "${label}_after" "$caller" "$acct" >"$after" || return 1
    hash=$(extract_signing_hash "$LOG_DIR/$label.err") || return 1
    strategy_assert_financial "${label}_financial" "$before" "$after" "$LOG_DIR/$hash.receipt.json" "$request" "$acct" "$CONTROLLER" "$AGGREGATOR" || return 1
    printf '%s\n' "$result"
}

flow_strategies() {
    phase strategies
    local flash_usdc=300000000
    local swap_hex
    swap_hex=$(agg_route_hex "$USDC_SAC" "$XLM_SAC" "$flash_usdc") || return 1
    local macct
    macct=$(strategy_inv multiply_long "$ALICE" "$CONTROLLER" -- multiply \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --collateral "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" --debt_to_flash_loan "$flash_usdc" \
        --debt "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" --mode 2 --swap "$swap_hex" \
        --initial_payment "[$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC"),\"5000000000\"]" --convert_swap null | tr -d '"') || return 1
    save_state ALICE_MACCT "$macct"
    log "multiply account = $macct"
    assert_hf_at_least hf_multiply "$macct" "$WAD"

    local new_xlm_debt=1000000000
    swap_hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" "$new_xlm_debt") || return 1
    strategy_inv swap_debt "$ALICE" "$CONTROLLER" -- swap_debt \
        --caller "$ALICE_ADDR" --account_id "$macct" \
        --existing_debt "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" --amount "$new_xlm_debt" \
        --new_debt "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" --swap "$swap_hex" >/dev/null || return 1
    assert_borrow_at_least xlm_debt_post_swap "$macct" "$XLM_SAC" 500000000

    leg_swap_collateral() {
        local hex
        hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" 2000000000) || return 1
        strategy_inv swap_collateral "$ALICE" "$CONTROLLER" -- swap_collateral \
            --caller "$ALICE_ADDR" --account_id "$macct" \
            --current "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" --amount 2000000000 \
            --new "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" --swap "$hex" >/dev/null || return 1
    }
    retry_leg leg_swap_collateral || return 1

    lifecycle_inv supply_for_rdwc "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id "$macct" --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 10000000000)" >/dev/null || return 1
    lifecycle_inv borrow_for_rdwc "$ALICE" "$CONTROLLER" -- borrow \
        --caller "$ALICE_ADDR" --account_id "$macct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 550000000)" --to null >/dev/null || return 1
    leg_repay_debt_with_coll() {
        local hex
        hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" 5000000000) || return 1
        strategy_inv repay_debt_with_coll "$ALICE" "$CONTROLLER" -- repay_debt_with_collateral \
            --caller "$ALICE_ADDR" --account_id "$macct" \
            --collateral "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" --collateral_amount 5000000000 \
            --debt "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" --swap "$hex" --close_position false >/dev/null || return 1
    }
    retry_leg leg_repay_debt_with_coll || return 1

    assert_hf_at_least hf_post_strategies "$macct" "$WAD"

    local rdwc_acct
    rdwc_acct=$(lifecycle_inv rdwc_close_supply "$CAROL" "$CONTROLLER" -- supply \
        --caller "$CAROL_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 4000000000)" | tr -d '"') || return 1
    lifecycle_inv rdwc_close_borrow "$CAROL" "$CONTROLLER" -- borrow \
        --caller "$CAROL_ADDR" --account_id "$rdwc_acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 100000000)" --to null >/dev/null || return 1
    leg_rdwc_close() {
        local hex

        hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" 4000000000 0.10) || return 1
        strategy_inv rdwc_close "$CAROL" "$CONTROLLER" -- repay_debt_with_collateral \
            --caller "$CAROL_ADDR" --account_id "$rdwc_acct" \
            --collateral "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" --collateral_amount 4000000000 \
            --debt "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" --swap "$hex" --close_position true >/dev/null || return 1
    }
    retry_leg leg_rdwc_close || return 1

    assert_bool_view rdwc_closed false account_exists --account_id "$rdwc_acct"

    local flash_xlm=5000000000 sacct=""
    leg_multiply_short() {
        local hex
        hex=$(agg_route_hex "$XLM_SAC" "$USDC_SAC" "$flash_xlm" 0.10) || return 1
        sacct=$(strategy_inv multiply_short "$ALICE" "$CONTROLLER" -- multiply \
            --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --collateral "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" --debt_to_flash_loan "$flash_xlm" \
            --debt "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" --mode 3 --swap "$hex" \
            --initial_payment "[$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC"),\"1000000000\"]" --convert_swap null | tr -d '"') || return 1
        [ -n "$sacct" ]
    }
    retry_leg leg_multiply_short || return 1
    save_state ALICE_SACCT "$sacct"
    assert_hf_at_least hf_short "$sacct" "$WAD"
}

flow_same_market() {
    phase same_market
    local acct key pool_pre ctrl_pre debt_pre positions_pre positions_post synced payer_pre payer_post pool_post
    # CLI Bytes file arguments contain raw bytes; an empty argv value is rejected.
    : > "$RUN_DIR/empty-route.bin"
    key=$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")
    acct=$(inv_create net_supply "$ALICE" "$CONTROLLER" -- supply --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" --assets "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 1000000000)") || return 1
    inv net_borrow "$ALICE" "$CONTROLLER" -- borrow --caller "$ALICE_ADDR" --account_id "$acct" --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 200000000)" --to null >/dev/null || return 1
    pool_pre=$(balance "$USDC_SAC" "$POOL") || return 1
    ctrl_pre=$(balance "$USDC_SAC" "$CONTROLLER") || return 1
    debt_pre=$(_view_int net_debt_pre get_borrow_amount --account_id "$acct" --hub_asset "$key") || return 1
    xfail net_nonempty_route 'Error\(Contract, #16\)' "$ALICE" "$CONTROLLER" -- repay_debt_with_collateral \
        --caller "$ALICE_ADDR" --account_id "$acct" --collateral "$key" --collateral_amount 100000000 --debt "$key" --swap 00 --close_position false || return 1
    positions_pre=$(view net_positions_pre "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    inv net_partial "$ALICE" "$CONTROLLER" -- repay_debt_with_collateral \
        --caller "$ALICE_ADDR" --account_id "$acct" --collateral "$key" --collateral_amount 100000000 --debt "$key" --swap-file-path "$RUN_DIR/empty-route.bin" --close_position false >/dev/null || return 1
    positions_post=$(view net_positions_post "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    synced=$(view net_committed_indexes "$POOL" -- get_sync_data --hub_asset "$key") || return 1
    assert_net_settle net_exact_accounting "$positions_pre" "$positions_post" "$synced" 100000000 7 || return 1
    assert_borrow_decreased net_debt_reduced "$acct" "$USDC_SAC" "$debt_pre" || return 1
    assert_delta net_pool_cash "$pool_pre" "$(balance "$USDC_SAC" "$POOL")" 0 || return 1
    assert_delta net_controller_cash "$ctrl_pre" "$(balance "$USDC_SAC" "$CONTROLLER")" 0 || return 1
    payer_pre=$(balance "$USDC_SAC" "$ALICE_ADDR") || return 1
    pool_pre=$(balance "$USDC_SAC" "$POOL") || return 1
    inv net_full "$ALICE" "$CONTROLLER" -- repay_debt_with_collateral \
        --caller "$ALICE_ADDR" --account_id "$acct" --collateral "$key" --collateral_amount 1000000000 --debt "$key" --swap-file-path "$RUN_DIR/empty-route.bin" --close_position true >/dev/null || return 1
    assert_bool_view net_closed false account_exists --account_id "$acct" || return 1
    payer_post=$(balance "$USDC_SAC" "$ALICE_ADDR") || return 1
    pool_post=$(balance "$USDC_SAC" "$POOL") || return 1
    synced=$(view net_close_committed_indexes "$POOL" -- get_sync_data --hub_asset "$key") || return 1
    local refund
    refund=$(net_close_refund "$positions_post" "$synced" 7) || { _assert_fail net_close_reference "invalid committed state"; return 1; }
    assert_delta net_close_refund "$payer_pre" "$payer_post" "$refund" || return 1
    assert_delta net_close_pool "$pool_pre" "$pool_post" "-$refund" || return 1
    assert_delta net_close_controller "$ctrl_pre" "$(balance "$USDC_SAC" "$CONTROLLER")" 0
}
