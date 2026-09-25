#!/usr/bin/env python3
"""Exercise the routed checker against missing debt and stolen/suppressed refunds."""
import copy
import json
import re
import subprocess
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
CHECK = re.search(r"<<'PYSTRATEGY'\n(.*?)\nPYSTRATEGY", (HERE / 'flows/strategies.sh').read_text(), re.S)[1]
R, U = 10**27, 10**20
KEY = lambda token: {'hub_id': 1, 'asset': token}


def sc(value):
    if isinstance(value, int): return {'i128': str(value)}
    if isinstance(value, list): return {'vec': [sc(x) for x in value]}
    return {'address': value}


def event(contract, topics, data):
    return {'type': 'contract', 'contract_id': contract,
            'body': {'v0': {'topics': topics, 'data': data}}}


def scenario(method, close=False, withdraw_all=False):
    before = {'exists': True, 'owner': 'CALLER', 'attrs': {'spoke_id': 1, 'mode': 2}, 'positions': [{}, {}], 'tokens': {t: {
        'wallet': '1000', 'pool': '1000', 'controller': '7',
        'sync': {'params': {'asset_decimals': 7, 'flashloan_fee': 100, 'reserve_factor': 1000},
                 'state': {'supply_index': str(R), 'borrow_index': str(R), 'supplied': str(2000*U), 'borrowed': str(1000*U), 'revenue': '0', 'cash': '1000', 'last_timestamp': '10'}}} for t in ['A', 'B']}}
    def pos(side, token, amount):
        if amount: before['positions'][side][json.dumps(KEY(token))] = {'scaled_amount': str(amount*U)}
    if method == 'swap_debt': pos(1, 'A', 20); pos(1, 'B', 30)
    if method in ['swap_collateral', 'repay_debt_with_collateral']: pos(0, 'A', 100)
    if method == 'swap_collateral': pos(0, 'B', 10)
    if method == 'repay_debt_with_collateral': pos(1, 'B', 30)
    after = copy.deepcopy(before)
    request = {'method': method, '--caller': 'CALLER', '--swap': '', '--account_id': '1'}
    deposits, borrows = [], []
    def leg(side, action, token, amount, remaining):
        row = [action, 1, token, remaining*U, R, amount]
        (deposits if side == 0 else borrows).append(row)
        after['positions'][side].pop(json.dumps(KEY(token)), None)
        if remaining: after['positions'][side][json.dumps(KEY(token))] = {'scaled_amount': str(remaining*U)}
    def delta(token, holder, amount): after['tokens'][token][holder] = str(int(after['tokens'][token][holder])+amount)
    spent, output = 50, 40
    if method == 'multiply':
        before.update(exists=False, owner=None, attrs=None)
        request.update({'--account_id': '0', '--spoke_id': '1', '--mode': '2'})
        request.update({'--debt': json.dumps(KEY('A')), '--collateral': json.dumps(KEY('B')),
                        '--debt_to_flash_loan': '50', '--initial_payment': json.dumps([KEY('B'), '100']), '--convert_swap': 'null'})
        leg(1, 6, 'A', 50, 50); leg(0, 0, 'B', 140, 140)
        spent = 49
        delta('A', 'pool', -49); delta('B', 'pool', 140); delta('B', 'wallet', -100)
    elif method == 'swap_debt':
        request.update({'--new_debt': json.dumps(KEY('A')), '--existing_debt': json.dumps(KEY('B')), '--amount': '50'})
        leg(1, 8, 'A', 50, 70); leg(1, 8, 'B', 30, 0)
        spent = 49
        delta('A', 'pool', -49); delta('B', 'pool', 30); delta('B', 'wallet', 10)
    elif method == 'swap_collateral':
        request.update({'--current': json.dumps(KEY('A')), '--new': json.dumps(KEY('B')), '--amount': '50'})
        leg(0, 9, 'A', 50, 50); leg(0, 0, 'B', 40, 50)
        delta('A', 'pool', -50); delta('B', 'pool', 40)
    else:
        amount = 100 if withdraw_all else 50
        spent = amount
        request.update({'--collateral': json.dumps(KEY('A')), '--debt': json.dumps(KEY('B')),
                        '--collateral_amount': str(amount), '--close_position': str(close).lower()})
        leg(0, 10, 'A', amount, 100-amount); leg(1, 11, 'B', 30, 0)
        delta('A', 'pool', -amount); delta('B', 'pool', 30); delta('B', 'wallet', 10)
        if close and not withdraw_all:
            leg(0, 12, 'A', 50, 0); delta('A', 'pool', -50); delta('A', 'wallet', 50)
    if close: after.update(exists=False, owner=None, attrs=None)
    for token in ['A', 'B']:
        state=after['tokens'][token]['sync']['state']
        for side, field in [(0,'supplied'),(1,'borrowed')]:
            old=int(before['positions'][side].get(json.dumps(KEY(token)),{}).get('scaled_amount',0))
            new=int(after['positions'][side].get(json.dumps(KEY(token)),{}).get('scaled_amount',0))
            state[field]=str(int(state[field])+new-old)
        state['cash']=after['tokens'][token]['pool']
        if token=='A' and method in ['multiply','swap_debt']:
            state['supplied']=str(int(state['supplied'])+U); state['revenue']=str(U)
    route = {'map': [{'key': {'symbol': k}, 'val': v} for k, v in [
        ('assets', sc(['A', 'B'])), ('amounts', sc([1])), ('ops', {'bytes': '010001000000000001000000000000'})]]}
    request['--swap'] = json.dumps(route).encode().hex()
    transfers = [event('A', [{'symbol': 'transfer'}, sc('CTRL'), sc('ROUTER'), {'string': 'A'}], sc(spent)),
                 event('B', [{'symbol': 'transfer'}, sc('ROUTER'), sc('CTRL'), {'string': 'B'}], sc(output))]
    batch = event('CTRL', [{'symbol': 'position'}, {'symbol': 'batch_update'}], sc([1, ['CALLER', 1, 2], deposits, borrows]))
    receipt = {'result': {'resultMetaXdr': json.dumps({'v4': {'operations': [{'events': transfers+[batch]}]}})}}
    return before, after, receipt, request


def check(before, after, receipt, request, succeeds=True):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / 'receipts.py').write_text('import json,base64\ndef decode(kind,value): return json.loads(base64.b64decode(value) if kind=="ScVal" else value)\n')
        (root / 'flows').mkdir()
        (root / 'lib').mkdir()
        (root / 'lib/assert.sh').symlink_to(HERE / 'lib/assert.sh')
        (root / 'flows/lifecycle.sh').symlink_to(HERE / 'flows/lifecycle.sh')
        for name, value in [('before', before), ('after', after), ('receipt', receipt), ('request', request)]:
            (root / name).write_text(json.dumps(value))
        result = subprocess.run(['python3', '-', tmp, *[str(root / name) for name in ['before', 'after', 'receipt', 'request']],
                                 '1', 'CTRL', 'ROUTER'], input=CHECK, text=True, capture_output=True)
        assert (result.returncode == 0) == succeeds, result.stderr or 'invalid routed accounting accepted'


for method, close, all_ in [('multiply', False, False), ('swap_debt', False, False), ('swap_collateral', False, False),
                            ('repay_debt_with_collateral', False, False), ('repay_debt_with_collateral', True, False),
                            ('repay_debt_with_collateral', True, True)]:
    b, a, receipt, req = scenario(method, close, all_)
    check(b, a, receipt, req)
    for field, value, event_index in [('owner', 'ATTACKER', 0), ('spoke_id', 99, 1), ('mode', 3, 2)]:
        bad_after, bad_receipt = copy.deepcopy(a), copy.deepcopy(receipt)
        if not close:
            if field=='owner': bad_after['owner']=value
            else: bad_after['attrs'][field]=value
        meta=json.loads(bad_receipt['result']['resultMetaXdr'])
        meta['v4']['operations'][0]['events'][-1]['body']['v0']['data']['vec'][1]['vec'][event_index]=sc(value)
        bad_receipt['result']['resultMetaXdr']=json.dumps(meta)
        check(b, bad_after, bad_receipt, req, False)
        # A dishonest event must not hide wrong persisted identity, either.
        if not close: check(b, bad_after, receipt, req, False)
    broken=copy.deepcopy(a); broken['exists']=not a['exists']
    check(b, broken, receipt, req, False)
    for token, holder in [('A', 'pool'), ('B', 'pool'), ('A', 'controller'), ('B', 'wallet')]:
        broken = copy.deepcopy(a)
        broken['tokens'][token][holder] = str(int(broken['tokens'][token][holder])-1)
        check(b, broken, receipt, req, False)
    # A transfer to an attacker must not count as the router's controller output.
    broken = copy.deepcopy(receipt)
    meta = json.loads(broken['result']['resultMetaXdr'])
    meta['v4']['operations'][0]['events'][1]['body']['v0']['topics'][2] = sc('ATTACKER')
    broken['result']['resultMetaXdr'] = json.dumps(meta)
    check(b, a, broken, req, False)
    if method in ['multiply', 'swap_debt']:
        for field in ['revenue','supplied','borrowed']:
            broken=copy.deepcopy(a)
            state=broken['tokens']['A']['sync']['state']; state[field]=str(int(state[field])-U)
            check(b, broken, receipt, req, False)
        # Retaining the fee in cash without protocol ownership must fail.
        broken=copy.deepcopy(a); state=broken['tokens']['A']['sync']['state']
        state['revenue']='0'; state['supplied']=str(int(state['supplied'])-U)
        check(b, broken, receipt, req, False)
        broken = copy.deepcopy(a); broken['positions'][1].pop(json.dumps(KEY('A')))
        check(b, broken, receipt, req, False)
        broken = copy.deepcopy(receipt); meta = json.loads(broken['result']['resultMetaXdr'])
        meta['v4']['operations'][0]['events'][-1]['body']['v0']['data']['vec'][3]['vec'].pop(0)
        broken['result']['resultMetaXdr'] = json.dumps(meta)
        check(b, a, broken, req, False)
    if method in ['swap_debt', 'repay_debt_with_collateral']:
        # Successful repay/account close cannot hide a retained 10-unit excess.
        broken = copy.deepcopy(a); broken['tokens']['B']['wallet'] = b['tokens']['B']['wallet']
        broken['tokens']['B']['controller'] = str(int(b['tokens']['B']['controller'])+10)
        check(b, broken, receipt, req, False)

# Partial debt repayment and router input refunds use separate cash predicates.
b, a, receipt, req = scenario('swap_debt')
b['positions'][1][json.dumps(KEY('B'))]['scaled_amount'] = str(100*U)
a['positions'][1][json.dumps(KEY('B'))] = {'scaled_amount': str(60*U)}
a['tokens']['B']['pool'] = '1040'; a['tokens']['B']['wallet'] = '1000'
a['tokens']['B']['sync']['state']['cash']='1040'
a['tokens']['B']['sync']['state']['borrowed']=str(960*U)
meta = json.loads(receipt['result']['resultMetaXdr'])
row = meta['v4']['operations'][0]['events'][-1]['body']['v0']['data']['vec'][3]['vec'][1]['vec']
row[3] = sc(60*U); row[5] = sc(40)
receipt['result']['resultMetaXdr'] = json.dumps(meta)
check(b, a, receipt, req)
b, a, receipt, req = scenario('multiply')
meta = json.loads(receipt['result']['resultMetaXdr'])
meta['v4']['operations'][0]['events'].insert(1, event('A', [{'symbol': 'transfer'}, sc('ROUTER'), sc('CTRL')], sc(3)))
receipt['result']['resultMetaXdr'] = json.dumps(meta); a['tokens']['A']['wallet'] = '1003'
check(b, a, receipt, req)
a['tokens']['A']['wallet'] = '1000'
check(b, a, receipt, req, False)
# Closed snapshots avoid owner_of/get_account_attributes, which revert after burn.
script = r'''source "$1"
XLM_SAC=A; USDC_SAC=B; CONTROLLER=CTRL; POOL=POOL; POSITION_NFT=NFT; PRIMARY_HUB_ID=1
financial_balance() { echo 1; }; balance() { echo 1; }; hub_key() { echo '{}'; }
view() {
    case "$4" in
        account_exists) echo false;;
        get_account_positions) echo '[{},{}]';;
        get_sync_data) echo '{}';;
        *) echo 'closed snapshot queried absent identity' >&2; return 1;;
    esac
}
strategy_snapshot closed CALLER 1
'''
closed=subprocess.run(['bash','-c',script,'_',str(HERE/'flows/strategies.sh')],capture_output=True,text=True)
assert closed.returncode==0, closed.stderr
snapshot=json.loads(closed.stdout)
assert snapshot['exists'] is False and snapshot['owner'] is None and snapshot['attrs'] is None
print('Routed principal, identity, destination, missing debt and suppressed-refund regressions passed')

# The published SDK route must use the same financial checker, bound to its
# builder inputs, returned account and committed wire receipt. A failed check
# must fail the case even when the independently read health factor is healthy.
with tempfile.TemporaryDirectory() as tmp:
    script = r'''set -uo pipefail
source "$1"
LOG_DIR="$2"; CHECK_RESULT="$3"; INTEG_DIR="$4"
XLM_SAC=B; USDC_SAC=A; ALICE_ADDR=CALLER; PRIMARY_HUB_ID=1; PRIMARY_SPOKE_ID=2
CONTROLLER=CTRL; AGGREGATOR=ROUTER; WAD=1000000000000000000
phase() { :; }; _assert_fail() { echo "$*" >&2; return 1; }
agg_route_hex() { [ "$*" = 'A B 100000000' ] || return 1; echo 01020304; }
strategy_snapshot() { printf '%s\n' "$*"; }
sdk_inv() {
    [ "$1" = sdk_multiply ] && [ "$2" = buildStellarMultiplyTx ] || return 1
    printf '%s' "$3" >"$LOG_DIR/builder.json"
    printf '{"hash":"%064d"}' 1 >"$LOG_DIR/sdk_multiply.sdk.json"
    printf '{"committed":true}' >"$LOG_DIR/$(printf '%064d' 1).receipt.json"
    echo 42
}
strategy_assert_financial() {
    python3 - "$LOG_DIR" "$@" <<'PYSDK'
import base64,json,sys
from pathlib import Path
root=Path(sys.argv[1])
label,before,after,receipt,request,account,controller,router=sys.argv[2:]
assert (label,account,controller,router)==('sdk_multiply_financial','42','CTRL','ROUTER')
assert Path(before).read_text().strip()=='sdk_multiply_before CALLER 0'
assert Path(after).read_text().strip()=='sdk_multiply_after CALLER 42'
assert Path(receipt).name=='0'*63+'1.receipt.json'
assert json.loads(Path(receipt).read_text())=={'committed':True}
a=json.loads((root/'builder.json').read_text()); r=json.loads(Path(request).read_text())
assert r['method']=='multiply' and r['--caller']=='CALLER'
assert r['--account_id']==str(a['accountNonce'])=='0'
assert r['--spoke_id']==str(a['spokeId'])=='2'
assert r['--mode']==str(a['mode'])=='1'
for name in ('collateral','debt'):
    assert json.loads(r['--'+name])=={'hub_id':a[name]['hubId'],'asset':a[name]['asset']}
assert r['--debt_to_flash_loan']==a['debtToFlashLoan']=='100000000'
assert bytes.fromhex(r['--swap'])==base64.b64decode(a['steps']['routeXdr'])==bytes([1,2,3,4])
p=a['initialPayment']
assert json.loads(r['--initial_payment'])==[{'hub_id':p['hubId'],'asset':p['asset']},p['amount']]
assert p['amount']=='10000000000' and r['--convert_swap']=='null'
(root/'checked').touch()
PYSDK
    [ "$?" = 0 ] || return 1
    return "$CHECK_RESULT"
}
assert_hf_at_least() { touch "$LOG_DIR/healthy"; }
flow_sdk_strategy
'''
    root=Path(tmp)
    for check_result in (0,1):
        for marker in ('checked','healthy'):
            (root/marker).unlink(missing_ok=True)
        result=subprocess.run(['bash','-c',script,'_',str(HERE/'flows/sdk.sh'),tmp,str(check_result),str(HERE)],capture_output=True,text=True)
        assert result.returncode==check_result, result.stdout+result.stderr
        assert (root/'checked').exists(), 'SDK flow bypassed financial check'
        assert (root/'healthy').exists()==(check_result==0), 'failed accounting reached HF-only success'
manifest=json.loads((HERE/'cases.json').read_text())
required=next(c for c in manifest if c['id']=='flow_sdk_strategy')['required_actions']
assert any(a['label']=='sdk_multiply_financial' and a['method']=='assert' and a['status']=='ok' and a['execution']==['assertion'] for a in required)
print('Published SDK strategy financial binding and failure propagation passed')

# Load the scenario's real flow imports before any mocks: missing shared helper
# imports must fail offline instead of reaching Testnet as command-not-found.
scenario_source=(HERE/'scenarios/sdk.sh').read_text()
flow_import=re.search(r'^for f in .*source "\$INTEG_DIR/flows/\$f.sh"; done$',scenario_source,re.M)[0]
loaded=subprocess.run(['bash','-c','set -eu; INTEG_DIR="$1"; '+flow_import+'; declare -F strategy_snapshot strategy_assert_financial >/dev/null','_',str(HERE)],capture_output=True,text=True)
assert loaded.returncode==0, loaded.stdout+loaded.stderr
