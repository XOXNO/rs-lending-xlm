#!/usr/bin/env python3
"""Offline regressions exercise the real shell helpers with mocked CLI/RPC."""
import csv
import importlib.util
import json
import subprocess
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('gate', HERE / 'gate.py')
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


def shell(body):
    with tempfile.TemporaryDirectory() as directory:
        setup = f'''
set -uo pipefail
source "{HERE}/lib/core.sh"
source "{HERE}/lib/invoke.sh"
source "{HERE}/lib/assert.sh"
source "{HERE}/lib/assets.sh"
LOG_DIR={directory}; RUN_DIR={directory}; ACTIONS_TSV={directory}/actions.tsv
PHASE=test; ADMIN=admin; RPC_URL=unused; NET_ARGS=(--network testnet)
printf 'seq\\tphase\\tlabel\\tstatus\\tfn\\thash\\tinstructions\\tread_bytes\\twrite_bytes\\tresource_fee\\tnote\\n' > "$ACTIONS_TSV"
backoff_sleep() {{ :; }}
sleep() {{ :; }}
'''
        result = subprocess.run(['bash', '-c', setup + body], capture_output=True, text=True)
        assert result.returncode == 0, result.stdout + result.stderr


def check_gate():
    original_verify = gate.verify_receipt
    gate.verify_receipt = lambda *args: resources
    manifest = json.loads((HERE / 'cases.json').read_text())
    definitions = [c for c in manifest if 'agg' in c['lanes']]
    required = [c['id'] for c in definitions]
    with tempfile.TemporaryDirectory() as directory:
        run = Path(directory)
        (run / 'logs').mkdir()
        (run / 'metadata.json').write_text(json.dumps(dict(lane='agg', selected_cases=required, source_sha='a'*40, configuration_sha256='b'*64, case_manifest_sha256=gate.digest(HERE/'cases.json'),sdk_lock_sha256=gate.digest(HERE/'sdk/package-lock.json'), instruction_leeway=20000000,network='testnet',network_passphrase='Test SDF Network ; September 2015',rpc_url='https://example.invalid',cli_version='stellar 28.0.0',sdk_version='1.0.221',stellar_sdk_version='16.0.1')))
        (run / 'candidate.json').write_text(json.dumps({'source_sha': 'a'*40, 'artifacts': {f'{c}.wasm': 'a'*64 for c in gate.CONTRACTS}}))
        limits = dict(txMaxInstructions=400000000, txMaxDiskReadBytes=200000,
                      txMaxWriteBytes=132096, txMaxDiskReadEntries=200, txMaxWriteLedgerEntries=200,
                      txMaxSizeBytes=132096, txMaxContractEventsSizeBytes=16384, txMemoryLimit=41943040)
        (run / 'network-limits.json').write_text(json.dumps({n:dict(limits=limits,ledger=1,network=dict(passphrase=('Test SDF Network ; September 2015' if n=='testnet' else 'Public Global Stellar Network ; September 2015'),protocolVersion=28),version=dict(protocolVersion=28,version='28.0.0')) for n in ['testnet','mainnet']}))
        addresses={c:'C'+'A'*54+chr(66+i) for i,c in enumerate(gate.CONTRACTS)}
        (run/'deployed-artifacts.jsonl').write_text(''.join(json.dumps(dict(address=a,artifact=c+'.wasm',sha256='a'*64))+'\n' for c,a in addresses.items()))
        actions, cases, proofs = [], [], []
        for case in definitions:
            first = len(actions)+1
            demands = case.get('required_actions') or [dict(label='prerequisite',method='assert',status='ok',count=1,execution=['assertion'])]
            for demand in demands:
                for _ in range(demand['count']):
                    seq = len(actions)+1
                    kind = demand['execution'][0]
                    hash_ = f'{seq:064x}' if kind in {'transaction','deployment'} else ''
                    actions.append([str(seq),'test',demand['label'],demand['status'],demand['method'],hash_,'','','','',''])
                    proofs.append([str(seq),kind,addresses[demand.get('contract','controller')]])
                    if hash_:
                        receipt = dict(status='SUCCESS',txHash=hash_,ledger=1,envelopeXdr='AA==',resultMetaXdr='AA==',resultXdr='AA==',events={'contractEventsXdr':[]})
                        resources = dict(resources=dict(instructions=1000000,disk_read_bytes=0,write_bytes=0,footprint={'read_only':[],'read_write':[]}))
                        (run/'logs'/f'{hash_}.receipt.json').write_text(json.dumps({'result':receipt}))
                        (run/'logs'/f'{hash_}.resources.json').write_text(json.dumps(resources))
            cases.append([case['id'],'pass',first,len(actions)])
        def write(name, fields, records):
            with (run/name).open('w') as f:
                w=csv.writer(f,delimiter='\t',lineterminator='\n'); w.writerow(fields); w.writerows(records)
        def baseline():
            write('actions.tsv',gate.ACTION_FIELDS,actions)
            write('cases.tsv',['id','status','first_action','last_action'],cases)
            write('evidence.tsv',['seq','execution','contract'],proofs)
        def rejected():
            try: gate.validate(run)
            except (ValueError,OSError,KeyError,AssertionError): return
            raise AssertionError('false green')
        baseline()
        assert gate.validate(run, expected_lane='agg') == len(required)
        wrong=[r.copy() for r in proofs]
        for row in wrong: row[2]='C'+'Z'*55
        write('evidence.tsv',['seq','execution','contract'],wrong); rejected(); baseline()
        meta=run/'metadata.json'; saved_meta=meta.read_text()
        meta.write_text(json.dumps({'lane':'agg','selected_cases':required,'source_sha':'a'*40})); rejected(); meta.write_text(saved_meta)
        for leeway in [1000000, 2000000, 0, '20000000', 20000000.0]:
            wrong_policy=json.loads(saved_meta); wrong_policy['instruction_leeway']=leeway
            meta.write_text(json.dumps(wrong_policy)); rejected(); meta.write_text(saved_meta)
        try: gate.validate(run, expected_lane='sdk')
        except ValueError: pass
        else: raise AssertionError('copied lane accepted')
        for status in ['FAIL','UNEXPECTED-OK','research','environment-blocked','sim-exceeded','unknown']:
            broken=[r.copy() for r in actions]; broken[0][3]=status
            write('actions.tsv',gate.ACTION_FIELDS,broken); rejected()
        baseline()
        # A failed action followed by the same successful label remains sticky.
        broken=[r.copy() for r in actions]; broken[0][3]='FAIL'; broken[1][2]=broken[0][2]
        write('actions.tsv',gate.ACTION_FIELDS,broken); rejected()
        baseline()
        for records in [[],cases[:-1],cases+cases[:1]]:
            write('cases.tsv',['id','status','first_action','last_action'],records); rejected()
        baseline()
        index=next(i for i,r in enumerate(proofs) if r[1]=='transaction')
        missing=[r.copy() for r in actions]; missing[index][5]=''
        write('actions.tsv',gate.ACTION_FIELDS,missing); rejected()
        baseline()
        # Successful assertions cannot replace required submitted evidence.
        weakened=[r.copy() for r in proofs]; weakened[index][1]='assertion'
        write('evidence.tsv',['seq','execution','contract'],weakened); rejected()
        baseline()
        receipt=run/'logs'/f'{actions[index][5]}.receipt.json'
        saved=receipt.read_text(); receipt.unlink(); rejected(); receipt.write_text(saved)
        (run/'active-case').write_text('interrupted'); rejected(); (run/'active-case').unlink()
        baseline()
        write('cases.tsv',['id','status','first_action','last_action'],cases[:-1])
        gate.mark_incomplete(run,'timeout')
        assert 'incomplete' in (run/'cases.tsv').read_text(); rejected()
        baseline()
        with (run/'actions.tsv').open('a') as f: f.write('malformed\n')
        rejected()
    gate.verify_receipt = original_verify


check_gate()
# A signed timeout or budget failure never executes a second mutation.
for error in ['timeout', 'Trapped', 'ResourceLimitExceeded']:
    shell('''
n=0
stellar() { n=$((n+1)); echo "Signing transaction: $(printf '%064d' 1)" >&2; echo ERROR >&2; return 1; }
tx_status() { echo UNKNOWN; }
if inv mutation admin contract -- supply; then exit 1; fi
[ "$n" = 1 ] && grep -q FAIL "$ACTIONS_TSV"
'''.replace('echo ERROR', f'echo {error}'))
# Success without submission proof is a failure.
shell('stellar() { echo 1; }; if inv missing admin contract -- supply; then exit 1; fi')
# Bad transport, JSON-RPC errors, absent transactionData and invalid XDR fail.
for payload in ['', '{}', '{', '{"error":{"code":-1}}', '{"jsonrpc":"2.0","id":1,"result":{}}']:
    shell('''
stellar() { echo invalid; }
curl() { printf '%s' PAYLOAD; }
if sim_probe bad admin contract -- supply; then exit 1; fi
[ "$PROBE_STATUS" = error ] && grep -q FAIL "$ACTIONS_TSV"
'''.replace('PAYLOAD', "'" + payload + "'"))
shell('stellar() { echo tx; }; curl() { return 7; }; ! sim_probe bad admin c -- supply')
# Failed or nonnumeric balance cannot become zero.
for body in ['return 1', 'echo null', 'echo malformed']:
    shell(f'view() {{ {body}; }}; ! balance token owner')
# A failed mutation leg is never replayed by its outer wrapper.
shell('n=0; leg() { n=$((n+1)); return 1; }; ! retry_leg leg; [ "$n" = 1 ]')
# A phase cannot report pass after ignoring a failed action, even if a later
# action with the same label succeeds. Earlier cases stay outside this range.
for status in ['FAIL', 'UNEXPECTED-OK']:
    shell('''
printf 'id\\tstatus\\tfirst_action\\tlast_action\\n' > "$RUN_DIR/cases.tsv"
record previous FAIL assert
good() { record good ok assert; }
run_case good good || exit 1
ignored_failure() { record repeated STATUS assert; record repeated ok assert; }
if run_case ignored ignored_failure; then exit 1; fi
awk -F'\\t' '$1=="good" && $2=="pass" {good=1} $1=="ignored" && $2=="fail" {bad=1} END {exit !(good && bad)}' "$RUN_DIR/cases.tsv"
[ ! -e "$RUN_DIR/active-case" ]
'''.replace('STATUS', status))
print('E2E offline harness regressions passed')
# Teardown batches full withdrawals by account, retaining mixed hub keys and
# the burn proof. A submitted failure must never fall back to per-asset calls.
for count, failure in [(1, False), (5, False), (5, True), (6, False)]:
    keys = [dict(hub_id=i % 2 + 1, asset=f'TOKEN{i // 2}') for i in range(count)]
    positions = [{json.dumps(key): dict(scaled_amount='123') for key in keys}, {}]
    shell(f'''
source "{HERE}/flows/teardown.sh"
CONTROLLER=CTRL; POSITION_NFT=NFT; ALICE=alice; ALICE_ADDR=OWNER
positions='{json.dumps(positions)}'
n=0; burned=0
view() {{
    case "$1" in
        td_exists2_7) echo true;;
        td_owner2_7) echo OWNER;;
        td_positions2_7) echo "$positions";;
        *) return 1;;
    esac
}}
inv() {{
    n=$((n+1))
    [ "$2" = alice ] && [ "$3" = CTRL ] && [ "$5" = withdraw ] || return 1
    [ "$6" = --caller ] && [ "$7" = OWNER ] && [ "$8" = --account_id ] && [ "$9" = 7 ] || return 1
    [ "${{10}}" = --withdrawals ] && [ "${{12}}" = --to ] && [ "${{13}}" = null ] || return 1
    jq -e --argjson positions "$positions" 'length == {count} and all(.[]; .[1] == "0") and (map(.[0]|tojson)|sort) == ($positions[0]|keys|map(fromjson|tojson)|sort)' <<<"${{11}}" || return 1
    record "$1" {'FAIL' if failure else 'ok'} withdraw
    return {int(failure)}
}}
assert_bool_view() {{
    [ "$1" = td_burned_7 ] && [ "$2" = false ] && [ "$3" = account_exists ] && [ "$5" = 7 ] || return 1
    burned=1
}}
rc=0
_td_withdraw_account 7 || rc=$?
[ "$n" = {0 if count > 5 else 1} ] && [ "$burned" = {int(count <= 5 and not failure)} ] || exit 1
{('[ "$rc" -ne 0 ] && grep -q FAIL "$ACTIONS_TSV"') if failure or count > 5 else '[ "$rc" = 0 ]'}
''')
# Suppressed refund, wrong fee destination and recap refund must all fail.
for before, after, expected in [('100', '100', '10000000'), ('100', '100', '25'), ('1000', '800', '-20')]:
    shell(f'! assert_delta broken {before} {after} {expected}; grep -q FAIL "$ACTIONS_TSV"')
shell('assert_delta large 100000000000000000000000 100000000000000000000001 1')
shell(f'source "{HERE}/flows/blend.sh"; ! blend_maps_empty \'{{"collateral":{{}},"liabilities":{{}},"supply":{{"0":"50"}}}}\'; ! blend_maps_empty \'{{}}\'')
# An interrupted case cannot replay a mutation when local resume is requested.
shell('''
printf 'id\\tstatus\\tfirst_action\\tlast_action\\n' > "$RUN_DIR/cases.tsv"
mutate_then_exit() { echo committed >> "$RUN_DIR/commits"; exit 7; }
(run_case partial mutate_then_exit) && exit 1
[ -f "$RUN_DIR/active-case" ]
E2E_RESUME=1
if run_case partial mutate_then_exit; then exit 1; fi
[ "$(wc -l < "$RUN_DIR/commits" | tr -d ' ')" = 1 ]
''')
# Quote transport/decoding failures are recorded even if a phase ignores return.
shell(f'source "{HERE}/lib/aggregator.sh"; AGGREGATOR_API=https://unused; curl() {{ return 7; }}; ! agg_route_hex a b 1; grep -q FAIL "$ACTIONS_TSV"')
# Wrong deployed bytecode cannot leave a successful output for ignored callers.
shell('''
verify_deployed_wasm() { return 1; }
cli_upload() { printf '%064d' 1; }
! run_deploy "$LOG_DIR/deploy.out" "$LOG_DIR/deploy.err" -- cli_upload
[ ! -s "$LOG_DIR/deploy.out" ] && grep -q FAIL "$ACTIONS_TSV"
''')
# Mainnet-derived disposable policy preserves every enabled parameter exactly.
from production_config import plan, materialize
from copy import deepcopy
with tempfile.TemporaryDirectory() as directory:
    source=HERE.parents[1]/'configs/mainnet'; output=Path(directory)/'testnet'
    definitions=plan(source)
    mapping={key:'fixture-'+str(i) for i,key in enumerate(definitions)}
    materialize(source,output,mapping,'testnet')
    actual=json.loads((output/'markets.json').read_text())
    expected=json.loads((source/'markets.json').read_text())
    expected['markets']=[m for m in expected['markets'] if m.get('enabled',True)]
    expected['network']='testnet'
    def substitute(value):
        if isinstance(value,str): return mapping.get(value,value)
        if isinstance(value,list): return [substitute(v) for v in value]
        if isinstance(value,dict): return {k:substitute(v) for k,v in value.items()}
        return value
    assert actual==substitute(expected), 'production policy changed while replacing addresses'
    assert {7,8,9,18}<={m['oracle']['asset_decimals'] for m in actual['markets']}
    for name in ['hubs.json','spokes.json']:
        config=json.loads((source/name).read_text())
        if name=='spokes.json':
            config={k:v for k,v in config.items() if v.get('enabled',True)}
            for spoke in config.values():
                spoke['assets']={k:v for k,v in spoke['assets'].items() if v.get('enabled',True)}
        assert json.loads((output/name).read_text())==substitute(config)
    broken=deepcopy(mapping); broken[next(iter(broken))]=list(broken.values())[1]
    try: materialize(source,output,broken,'testnet')
    except AssertionError: pass
    else: raise AssertionError('aliased provider/token fixtures allowed')
# Independent same-market arithmetic rejects a skipped burn and a lost refund.
shell('''
pre='[{"market":{"scaled_amount":"100000000000000000000000000000"}},{"market":{"scaled_amount":"20000000000000000000000000000"}}]'
post='[{"market":{"scaled_amount":"90000000000000000000000000000"}},{"market":{"scaled_amount":"10000000000000000000000000000"}}]'
state='{"state":{"supply_index":"1000000000000000000000000000","borrow_index":"1000000000000000000000000000"}}'
assert_net_settle exact "$pre" "$post" "$state" 100000000 7
! assert_net_settle suppressed "$pre" "$pre" "$state" 100000000 7
[ "$(net_close_refund "$post" "$state" 7)" = 800000000 ]
''')

# Independent liquidation reference includes staged fee rounding on full close.
shell('''
state='{"state":{"supply_index":"1000000000000000000000000000","borrow_index":"1000000000000000000000000000"}}'
pre='[{"market":{"scaled_amount":"1000000000000000000000000000000"}},{"market":{"scaled_amount":"600000000000000000000000000000"}}]'
[ "$(liquidation_reference "$pre" "$state" "$state" 1000000000)" = '1000000000 1666571428 2380000' ] || exit 1
pre='[{"market":{"scaled_amount":"833342857200000000000000000000"}},{"market":{"scaled_amount":"500000000000000000000000000000"}}]'
[ "$(liquidation_reference "$pre" "$state" "$state" 6000000000)" = '5000000000 8332857142 11899999' ] || exit 1
''')
# Wrong-share debt retirement cannot be hidden by correct token transfers.
shell('''
pre='[{},{"market":{"scaled_amount":"600000000000000000000000000000"}}]'
post='[{},{"market":{"scaled_amount":"500000000000000000000000000000"}}]'
state='{"state":{"borrow_index":"1000000000000000000000000000"}}'
assert_liquidation_debt_burn correct "$pre" "$post" "$state" 1000000000 || exit 1
! assert_liquidation_debt_burn suppressed "$pre" "$pre" "$state" 1000000000
''')
# Negative probes cannot borrow another lane wallet's signing authority.
shell('''
export XDG_CONFIG_HOME="$RUN_DIR/private"
mkdir -p "$XDG_CONFIG_HOME/stellar/identity"
printf 'test-only source identity' > "$XDG_CONFIG_HOME/stellar/identity/bob.toml"
printf 'test-only victim identity' > "$XDG_CONFIG_HOME/stellar/identity/victim.toml"
stellar() {
  [ "$3" = --config-dir ] || return 8
  [ -f "$4/identity/bob.toml" ] && [ ! -e "$4/identity/victim.toml" ] || return 9
  echo 'Missing signing key for account victim' >&2
  return 1
}
xfail unauthorized 'Missing signing key for account victim' bob contract -- withdraw
''')

# Credit uses exact fixture seizure and ceil(bonus shares * 1%), including
# fractional native units retained by the first leg and spent by the next.
R=10**27
seized=166657142857142857143000000000
fee=238000000000000000000204012
stamp=dict(liquidation_threshold=7500,liquidation_bonus=800,liquidation_fees=100)
credit_before=[{'market':dict(stamp,scaled_amount=str(1000*R))},{'debt':{'scaled_amount':str(600*R)}}]
credit_after=deepcopy(credit_before); credit_after[0]['market']['scaled_amount']=str(1000*R-seized)
credit_after[1]['debt']['scaled_amount']=str(500*R)
receiver_before=[{},{}]
receiver_after=[{'market':dict(stamp,scaled_amount=str(seized-fee))},{}]
pool_before={'state':dict(supply_index=str(R),borrowed='0',supplied=str(1000*R),cash='10000000000',revenue='0')}
pool_after=deepcopy(pool_before); pool_after['state']['revenue']=str(fee)
attrs={'spoke_id':1,'mode':0}
debt_state={'state':{'borrow_index':str(R)}}
def credit_check(values, succeeds, paid=1000000000):
    import shlex
    args=' '.join(shlex.quote(json.dumps(v)) for v in values)+' 1 '+shlex.quote(json.dumps(debt_state))+' '+str(paid)
    shell(f"{'assert_liquidation_credit correct' if succeeds else '! assert_liquidation_credit broken'} {args}")
credit_values=[credit_before,credit_after,receiver_before,receiver_after,pool_before,pool_after,attrs]
credit_check(credit_values,True)
existing=deepcopy(credit_values)
existing[2]=[{'market':dict(stamp,scaled_amount='7')},{}]
existing[3][0]['market']['scaled_amount']=str(seized-fee+7)
credit_check(existing,True)
second=deepcopy(credit_values)
second[0]=deepcopy(credit_after); second[1]=deepcopy(credit_after)
second_seized=83328571428571428571000000000
second_fee=118999999999999999999387966
second[1][0]['market']['scaled_amount']=str(1000*R-seized-second_seized)
second[1][1]['debt']['scaled_amount']=str(450*R)
second[2]=deepcopy(receiver_after)
second[3][0]['market']['scaled_amount']=str(seized-fee+second_seized-second_fee)
second[4]=deepcopy(pool_after); second[5]['state']['revenue']=str(fee+second_fee)
credit_check(second,True,500000000)
for index,path,value in [
    (3,('market',),str(seized-fee-1)),             # lost receiver share
    (5,('revenue',),'0'),                          # suppressed fee
    (5,('supplied',),str(1000*R+1)),               # extra pool share
    (5,('cash',),'9999999999'),                    # moved collateral cash
    (6,('spoke_id',),2),                          # wrong receiver spoke
    (6,('mode',),1),                              # wrong receiver mode
]:
    broken=deepcopy(credit_values)
    if index==3: broken[3][0]['market']['scaled_amount']=value
    elif index==5: broken[5]['state'][path[0]]=value
    else: broken[6][path[0]]=value
    credit_check(broken,False)
# Both preserve conservation; each must fail the independent amount checks.
broken=deepcopy(credit_values)
broken[3][0]['market']['scaled_amount']=str(seized-fee-1)
broken[5]['state']['revenue']=str(fee+1)
credit_check(broken,False)
broken=deepcopy(credit_values)
broken[1][0]['market']['scaled_amount']=str(1000*R-seized-1)
broken[3][0]['market']['scaled_amount']=str(seized-fee+1)
credit_check(broken,False)


# Pool ABI exposes params through get_sync_data, never get_market_params. Drive
# the real successful flash-loan wrapper so a nonexistent method cannot hide
# behind arithmetic-only tests; retain the protocol-share rejection check.
for flash_mode in (0, 6):
    shell(f'source "{HERE}/flows/strategies.sh"\nMODE={flash_mode}\n'+r'''
PRIMARY_HUB_ID=1; USDC_SAC=USDC; POOL=POOL; CONTROLLER=CTRL
ALICE=alice; ALICE_ADDR=CALLER; FLASH_RECEIVER=RECEIVER
committed=0; suppress_fee=0
hub_key() { echo '{"hub_id":1,"asset":"USDC"}'; }
flash_data_hex() { echo "$1"; }
view() {
    [ "$2" = POOL ] && [ "$4" = get_sync_data ] && [ "$5" = --hub_asset ] || return 9
    [ "$6" = '{"hub_id":1,"asset":"USDC"}' ] || return 9
    python3 - "$committed" "$suppress_fee" <<'PYFLASH'
import json,sys
n=int(sys.argv[1]); fee=100000; shares=fee*10**20
print(json.dumps({'params':{'flashloan_fee':10,'asset_decimals':7},'state':{
 'borrowed':'0','supply_index':str(10**27),'supplied':str(10**30+n*shares),
 'revenue':str(n*shares if sys.argv[2]=='0' else 0),'cash':str(10**10+n*fee)}}))
PYFLASH
}
balance() {
    case "$2" in
        RECEIVER) echo "$((50000000-committed*100000))";;
        POOL) echo "$((10000000000+committed*100000))";;
        CTRL|CALLER) echo 700;;
        *) return 8;;
    esac
}
inv() {
    [ "$2" = alice ] && [ "$3" = CTRL ] && [ "$5" = flash_loan ] || return 8
    [ "${15}" = "$MODE" ] || return 8
    committed=1
}
flash_loan_checked checked "$MODE" || exit 1
committed=0; suppress_fee=1
if flash_loan_checked suppressed "$MODE"; then exit 1; fi
''')
