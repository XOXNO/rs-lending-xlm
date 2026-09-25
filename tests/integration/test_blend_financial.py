#!/usr/bin/env python3
"""Offline mutation checks of Blend's actual embedded receipt/reference checker."""
import copy
import json
import re
import subprocess
import tempfile
from fractions import Fraction
from pathlib import Path

HERE = Path(__file__).resolve().parent
CHECK = re.search(r"<<'PYBLEND'\n(.*?)\nPYBLEND", (HERE / 'flows/blend.sh').read_text(), re.S)[1]
RAY, UNIT = 10**27, 10**20
BR, DR, SI, BI = 10**12 + 347, 10**12 + 567, RAY + 42 * UNIT, RAY + 73 * UNIT


def scval(value):
    if isinstance(value, int):
        return {'i128': str(value)}
    if isinstance(value, list):
        return {'vec': [scval(x) for x in value]}
    return {'address': value}


def scenario(coll=True, supply=True, debt=True, existing=False, delegate=False):
    owner = 'owner' if delegate else 'caller'
    source = {'collateral': {'0': '2000000017'}, 'supply': {'0': '500000003', '1': '41'}, 'liabilities': {'0': '300000011'}}
    key = json.dumps({'hub_id': 1, 'asset': 'XLM'})
    untouched = json.dumps({'hub_id': 2, 'asset': 'OTHER'})
    positions = [{}, {}]
    if existing:
        positions = [{key: {'scaled_amount': str(17 * UNIT)}, untouched: {'scaled_amount': '29'}}, {}]
    before = dict(source=source, positions=positions, attrs={'mode': 0, 'spoke_id': 1}, owner=owner,
                  unrelated=[{'stable': {'scaled_amount': '31'}}, {}], owner_source=copy.deepcopy(source),
                  controller_usdc='83', caller_usdc='101', controller_xlm='67', pool_xlm='100000000000', caller_xlm='90000000000',
                  owner_xlm='80000000000',owner_usdc='109',pool_usdc='113')
    after = copy.deepcopy(before)
    credit = sum(int(Fraction(int(source[name]['0']) * BR, 10**12)) for name, active in [('collateral', coll), ('supply', supply)] if active)
    ceil = lambda f: -(-f.numerator // f.denominator)
    paid = ceil(Fraction(int(source['liabilities']['0']) * DR, 10**12)) if debt else 0
    cap = paid + 100000001 if debt else 0
    supply_delta = int(Fraction(credit * UNIT * RAY, SI))
    debt_delta = ceil(Fraction(cap * UNIT * RAY, BI)) - int(Fraction((cap - paid) * UNIT * RAY, BI))
    for name, active in [('collateral', coll), ('supply', supply), ('liabilities', debt)]:
        if active:
            del after['source'][name]['0']
    for side, delta in enumerate([supply_delta, debt_delta]):
        if delta:
            old = int(after['positions'][side].get(key, {}).get('scaled_amount', 0))
            after['positions'][side][key] = {'scaled_amount': str(old + delta)}
    after['pool_xlm'] = str(int(before['pool_xlm']) + credit - paid)
    after['caller_xlm'] = str(int(before['caller_xlm']) - 1234)
    entry = {'updated': {'data': {'contract_data': {'contract': 'BLEND',
             'key': {'vec': [{'symbol': 'ResData'}, {'address': 'XLM'}]},
             'val': {'map': [{'key': {'symbol': 'b_rate'}, 'val': scval(BR)}, {'key': {'symbol': 'd_rate'}, 'val': scval(DR)}]}}}}}
    event = {'contract_id': 'POOL', 'type': 'contract', 'body': {'v0': {
        'topics': [{'symbol': 'market'}, {'symbol': 'batch_state_update'}],
        'data': scval([[1, 'XLM', 123, SI, BI, 0, 0, 0, 0]])}}}
    meta = {'v4': {'operations': [{'changes': [entry], 'events': [event]}]}}
    receipt = {'result': {'resultMetaXdr': json.dumps(meta), 'resultXdr': json.dumps({'fee_charged': '1234'})}}
    args = [json.dumps(['XLM'] if coll else []), json.dumps(['XLM'] if supply else []),
            json.dumps([['XLM', str(cap)]] if debt else []), '77' if existing else '0', '77', '1', 'XLM', 'BLEND', 'POOL', 'caller']
    return before, after, receipt, args, key


def check(before, after, receipt, args, succeeds=True):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        # Isolate only the native XDR codec; execute the production checker as-is.
        (root / 'receipts.py').write_text('import json\ndef decode(kind, value): return json.loads(value)\n')
        for name, value in [('before', before), ('after', after), ('receipt', receipt), ('reserves', [{'asset': 'XLM', 'index': 0, 'decimals': 7}] + ([{'asset': 'BLEND_USDC', 'index': 2, 'decimals': 6}] if 'BLEND_USDC' in before.get('extra_balances', {}) else []))]:
            (root / name).write_text(json.dumps(value))
        result = subprocess.run(['python3', '-', directory, *[str(root / p) for p in ['before', 'after', 'receipt', 'reserves']], *args],
                                input=CHECK, text=True, capture_output=True)
        assert (result.returncode == 0) == succeeds, result.stderr or 'bad migration was accepted'


for flags in [(True, True, True, False, False), (True, False, False, False, False),
              (False, True, False, False, False), (True, False, False, True, False),
              (True, False, False, True, True)]:
    before, after, receipt, args, key = scenario(*flags)
    check(before, after, receipt, args)
    for change in [lambda a: a.update(owner='wrong'), lambda a: a['attrs'].update(spoke_id=2),
                   lambda a: a.update(controller_xlm='68'), lambda a: a.update(caller_usdc='100'),
                   lambda a: a.update(owner_usdc='108'), lambda a: a.update(pool_usdc='112'),
                   lambda a: a.update(pool_xlm=str(int(a['pool_xlm']) - 1)),
                   lambda a: a['positions'][0][key].update(scaled_amount=str(int(a['positions'][0][key]['scaled_amount']) - 1)),
                   lambda a: a.update(unrelated=[{}, {}]), lambda a: a['source']['supply'].pop('1')]:
        broken = copy.deepcopy(after)
        change(broken)
        check(before, broken, receipt, args, False)
    broken = copy.deepcopy(after)
    requested_map = 'collateral' if flags[0] else 'supply'
    broken['source'][requested_map]['0'] = '1'
    check(before, broken, receipt, args, False)
    if flags[2]:
        broken = copy.deepcopy(after)
        broken['positions'][1][key]['scaled_amount'] = str(int(broken['positions'][1][key]['scaled_amount']) + 1)
        check(before, broken, receipt, args, False)
    if flags[3]:
        wrong_id = list(args); wrong_id[4] = '78'
        check(before, after, receipt, wrong_id, False)
    if flags[4]:
        broken = copy.deepcopy(after); broken['owner_source'] = {'collateral': {}, 'supply': {}, 'liabilities': {}}
        check(before, broken, receipt, args, False)
        broken = copy.deepcopy(after); broken['owner_xlm'] = '1'
        check(before, broken, receipt, args, False)

# Distinct collateral/debt, then two simultaneous liabilities. Different reserve
# index, rates and decimals expose accidental XLM constants and cross-netting.
for multiple in (False, True):
    before, after, receipt, args, key = scenario(True, False, multiple, True, False)
    token='BLEND_USDC'; shares=1000003; dr=10**12+1987; bi=RAY+97*UNIT
    paid=-(-(shares*dr)//10**12); cap=paid+200007; unit=10**21
    before['source']['liabilities']['2']=str(shares)
    before['extra_balances']={token:dict(controller='57',pool='100000000',caller='7000000',owner='7000000')}
    after['extra_balances']=copy.deepcopy(before['extra_balances'])
    after['extra_balances'][token]['pool']=str(100000000-paid)
    usd_key=json.dumps({'hub_id':1,'asset':token})
    before['positions'][1][usd_key]={'scaled_amount':'919'}
    after['positions'][1][usd_key]={'scaled_amount':str(919-(-(cap*unit*RAY)//bi)-(cap-paid)*unit*RAY//bi)}
    caps=json.loads(args[2]); caps.append([token,str(cap)]); args[2]=json.dumps(caps)
    meta=json.loads(receipt['result']['resultMetaXdr']); operation=meta['v4']['operations'][0]
    entry=copy.deepcopy(operation['changes'][0]); data=entry['updated']['data']['contract_data']
    data['key']['vec'][1]={'address':token}; data['val']['map'][1]['val']=scval(dr)
    operation['changes'].append(entry)
    operation['events'][0]['body']['v0']['data']['vec'].append(scval([1,token,123,SI,bi,0,0,0,0]))
    receipt['result']['resultMetaXdr']=json.dumps(meta)
    check(before,after,receipt,args)
    for holder in ('controller','caller','owner','pool'):
        broken=copy.deepcopy(after)
        broken['extra_balances'][token][holder]=str(int(broken['extra_balances'][token][holder])+1)
        check(before,broken,receipt,args,False)
    for delta in (-1,1):
        broken=copy.deepcopy(after)
        broken['positions'][1][usd_key]['scaled_amount']=str(int(broken['positions'][1][usd_key]['scaled_amount'])+delta)
        check(before,broken,receipt,args,False)
    broken=copy.deepcopy(after); broken['source']['liabilities']['2']='1'
    check(before,broken,receipt,args,False)
    broken=copy.deepcopy(after); broken['extra_balances']={}
    check(before,broken,receipt,args,False)
    invalid=list(args); invalid_caps=copy.deepcopy(caps); invalid_caps[-1][1]=str(paid-1); invalid[2]=json.dumps(invalid_caps)
    check(before,after,receipt,invalid,False)
    invalid=list(args); invalid[2]=json.dumps(caps+[caps[-1]])
    check(before,after,receipt,invalid,False)
    broken=copy.deepcopy(receipt); missing=copy.deepcopy(meta); missing['v4']['operations'][0]['changes'].pop()
    broken['result']['resultMetaXdr']=json.dumps(missing)
    check(before,after,broken,args,False)

print('Blend single/cross-reserve/multiple-liability accounting and mutation checks passed')

# SDK transport preserves arrays and i128/u64 decimal strings before invoking
# the exact same wrapper. Financial failures must propagate after submission.
transport = r'''set -uo pipefail
source "$1/flows/blend.sh"
source "$1/flows/sdk.sh"
LOG_DIR="$2"; RUN_DIR="$2"; INTEG_DIR="$1"
ALICE=alice; ALICE_ADDR=caller; ADMIN_ACCT=1; PRIMARY_HUB_ID=3; PRIMARY_SPOKE_ID=2
CONTROLLER=CTRL; POSITION_NFT=NFT; XLM_SAC=XLM; BLEND_POOL=BLEND; POOL=POOL
sdk_inv() {
    [ "$2" = buildStellarMigrateFromBlendTx ] || return 1
    printf '%s' "$3" > "$LOG_DIR/args.json"
    printf '%064d' 1 > "$LOG_DIR/$1.hash"
    printf '77\n'
}
blend_snapshot() { printf '{}\n'; }
view() { printf '"caller"\n'; }
_assert_fail() { return 1; }
blend_assert_migration() {
    [ "$1" = sdk_blend_financial ] && [ -f "$2" ] && [ -f "$3" ] || return 1
    [ "$4" = "$LOG_DIR/$(printf '%064d' 1).receipt.json" ] || return 1
    [ "${13}" = BLEND ] && [ "${14}" = POOL ] && [ "${15}" = caller ] || return 1
    [ "$CHECK_FAIL" = 0 ]
}
CHECK_FAIL="$3"
blend_migrate_checked sdk_blend_submit sdk_blend alice caller 0 '["XLM","TOKEN"]' '["TOKEN"]' '[["XLM","1000000000000000001"],["TOKEN","31"]]'
'''
with tempfile.TemporaryDirectory() as directory:
    for fail in ('0','1'):
        result=subprocess.run(['bash','-c',transport,'_',str(HERE),directory,fail],text=True,capture_output=True)
        assert (result.returncode==0)==(fail=='0'), result.stderr
        if fail=='0': assert result.stdout=='77\n'
        args=json.loads((Path(directory)/'args.json').read_text())
        assert args==dict(accountId='0',spokeId=2,hubId=3,blendPool='BLEND',collateralTokens=['XLM','TOKEN'],
                          supplyTokens=['TOKEN'],debtCaps=[dict(token='XLM',cap='1000000000000000001'),dict(token='TOKEN',cap='31')])
print('SDK Blend transport reuses exact financial assertions and propagates failures')

# Execute the real migration preflight and compare its observed view call with
# the manifest demand. Stop at the first mutation; no CLI/network is invoked.
reserve_preflight = r'''set -uo pipefail
source "$1/flows/blend.sh"
RUN_DIR="$2"; XLM_SAC=XLM; BLEND_POOL=BLEND
phase() { :; }
view() {
    printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$4" "$6" >> "$RUN_DIR/reads.tsv"
    printf '%s\n' "$RESERVE"
}
_assert_fail() { return 1; }
blend_restore_min_borrow() { exit 0; }
RESERVE="$3"
flow_blend_migrate
'''
manifest=json.loads((HERE/'cases.json').read_text())
case=next(c for c in manifest if c['id']=='flow_blend_migrate')
demand=next(a for a in case['required_actions'] if a['method']=='get_reserve')
assert demand['status']=='read' and demand['execution']==['simulation'] and demand['count']==1
with tempfile.TemporaryDirectory() as directory:
    root=Path(directory)
    (root/'blend-reserves.json').write_text(json.dumps([dict(asset='XLM',index=0,decimals=7)]))
    for patch in ({},{'asset':'WRONG'},{'config':dict(index=1,decimals=7)},{'config':dict(index=0,decimals=6)}):
        reserve=dict(asset='XLM',config=dict(index=0,decimals=7)); reserve.update(patch)
        (root/'reads.tsv').write_text('')
        result=subprocess.run(['bash','-c',reserve_preflight,'_',str(HERE),directory,json.dumps(reserve)],text=True,capture_output=True)
        assert (result.returncode==0)==(not patch), result.stderr
        assert (root/'reads.tsv').read_text().splitlines()==[f"{demand['label']}\tBLEND\t{demand['method']}\tXLM"]
print('Blend reserve preflight matches required manifest evidence and rejects mapping drift')
