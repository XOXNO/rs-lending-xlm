"""Fault injection for risk snapshots, NFT enumeration and route headers."""
import csv
import json
import subprocess
import unittest
import tempfile
from pathlib import Path
HERE=Path(__file__).resolve().parent

def shell(file,body,*args):
    return subprocess.run(['bash','-c','source "$1"; source "$2"; shift 2; record() { :; }; log() { :; }; _assert_fail() { return 1; }; '+body,'_',str(HERE/'lib/assert.sh'),str(HERE/file),*args],capture_output=True,text=True)

class Predicates(unittest.TestCase):
    def test_sanity_owner_guard_belongs_to_executing_case(self):
        result=shell('flows/admin.sh', '''
set -e
phase() { CASE_ID="flow_$1"; }
for f in inv assert_market_field assert_view_eq_at; do eval "$f() { :; }"; done
for f in pay_vec hub_key market_params_json spoke_args price_key_token; do eval "$f() { echo '{}'; }"; done
view() {
    case "$1" in
        pa_price_spread) echo '[1,1]';;
        pa_oracle) echo '{"min_sanity_price_wad":8000,"max_sanity_price_wad":12000}';;
        *) echo '[{"price_wad":"10000"}]';;
    esac
}
xfail() {
    if [ "$1" = pa_set_sanity_band_owner_guard ]; then
        echo "$CASE_ID"; exit 0
    fi
}
flow_admin
exit 1
''')
        self.assertEqual(result.returncode,0,result.stderr)
        definitions=json.loads((HERE/'cases.json').read_text())
        owners=[case['id'] for case in definitions if any(
            action['label']=='pa_set_sanity_band_owner_guard'
            for action in case['required_actions'])]
        self.assertEqual(owners,[result.stdout.strip()])

    def test_risk_stamps(self):
        key=json.dumps({'asset':'A','hub_id':1}); base=[{key:dict(scaled_amount='123',loan_to_value=5000,liquidation_threshold=7000,liquidation_bonus=800,liquidation_fees=100)},{}]
        for change,ok in [('none',True),('threshold',False),('principal',False),('missing',False)]:
            got=json.loads(json.dumps(base))
            if change=='threshold':got[0][key]['liquidation_threshold']=6999
            if change=='principal':got[0][key]['scaled_amount']='122'
            if change=='missing':got[0]={}
            result=shell('flows/admin.sh','CONTROLLER=C PRIMARY_HUB_ID=1; SNAP="$1"; BEFORE="$2"; view() { echo "$SNAP"; }; risk_assert_stamps test 1 A "[5000,7000,800,100]" "$BEFORE"',json.dumps(got),json.dumps(base))
            self.assertEqual(result.returncode==0,ok,result.stderr)
    def test_nft_enumeration(self):
        body='''POSITION_NFT=N; ITEMS="$1"; view() { case "$1" in *_count) echo 2;; *_item_0) echo 1;; *_item_1) echo "$ITEMS";; esac; }; nft_assert_enumeration enum OWNER 2 true'''
        self.assertEqual(shell('flows/nft.sh',body,'2').returncode,0)
        self.assertNotEqual(shell('flows/nft.sh',body,'1').returncode,0)
        self.assertNotEqual(shell('flows/nft.sh',body,'3').returncode,0)
    def test_recap_shortfall_and_excess(self):
        R=10**27;U=10**20
        state={'state':dict(supplied=str(1000*U),borrowed=str(100*U),supply_index=str(R),borrow_index=str(R),cash='850')}
        for offered,want in [(0,0),(20,20),(100,50)]:
            result=shell('flows/liquidation.sh','recapitalization_reference "$1" "$2" 7',json.dumps(state),str(offered))
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(int(result.stdout),want)
        # Returning all100 instead of expected50 cannot satisfy exact predicate.
        r=shell('lib/assert.sh','assert_raw_within recap 100 50 0')
        self.assertNotEqual(r.returncode,0)

    def test_bad_debt_exact_index_and_books(self):
        R=10**27
        positions=[{'collateral':{'scaled_amount':str(30*R)}},{'debt':{'scaled_amount':str(10*R)}}]
        before={'params':{'reserve_factor':1000},'state':dict(supplied=str(1000*R),borrowed=str(100*R),revenue='0',supply_index=str(R),borrow_index=str(R),cash='9000000000',last_timestamp=0)}
        # Committed borrow index 1.1: $10 interest, $9 supplier reward, $1
        # protocol revenue. Burn $11 debt and apply the two native floor steps.
        shares=991080277502477700693756194
        after=json.loads(json.dumps(before))
        after['state'].update(supplied=str(1000*R+shares),borrowed=str(90*R),revenue=str(shares),borrow_index=str(11*R//10),supply_index='998010891089108910891089108',last_timestamp=1000)
        coll={'params':{'reserve_factor':1000},'state':dict(supplied=str(30*R),borrowed='0',revenue='0',supply_index=str(R),borrow_index=str(R),cash='300000000',last_timestamp=0)}
        seized=json.loads(json.dumps(coll));seized['state'].update(revenue=str(30*R),last_timestamp=1000)
        body='bad_debt_accounting "$1" "$2" "$3" "$4" "$5"'
        def check(post,ok):
            result=shell('flows/liquidation.sh',body,*map(json.dumps,[positions,before,post,coll,seized]))
            self.assertEqual(result.returncode==0,ok,result.stderr)
        check(after,True)
        for field,value in [('supply_index',str(R//2)),('supply_index',str(int(after['state']['supply_index'])+1)),('borrowed',str(90*R+1)),('cash','9000000001')]:
            bad=json.loads(json.dumps(after));bad['state'][field]=value;check(bad,False)
        # Invented protocol shares preserve user supply shares but must fail.
        bad=json.loads(json.dumps(after))
        for field in ['supplied','revenue']:bad['state'][field]=str(int(bad['state'][field])+1)
        check(bad,False)

    def test_recap_preserves_accrued_books(self):
        R=10**27
        before={'params':{'reserve_factor':1000},'state':dict(supplied=str(1000*R),borrowed=str(100*R),revenue='0',supply_index=str(R),borrow_index=str(R),cash='8500000000',last_timestamp=0)}
        shares=991080277502477700693756194
        after=json.loads(json.dumps(before))
        after['state'].update(supplied=str(1000*R+shares),revenue=str(shares),borrow_index=str(11*R//10),supply_index=str(1009*R//1000),last_timestamp=1000,cash='8999999999')
        body='recapitalization_reference "$1" 1000000000 7 "$2"'
        result=shell('flows/liquidation.sh',body,json.dumps(before),json.dumps(after))
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertEqual(int(result.stdout),499999999)
        healthy=json.loads(json.dumps(before));healthy['state']['cash']='9000000000'
        result=shell('flows/liquidation.sh',body,json.dumps(healthy),json.dumps(healthy))
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertEqual(int(result.stdout),0)
        for field,value in [('supplied','0'),('borrowed','0'),('revenue','0'),('supply_index',str(R)),('cash','9000000000')]:
            bad=json.loads(json.dumps(after));bad['state'][field]=value
            self.assertNotEqual(shell('flows/liquidation.sh',body,json.dumps(before),json.dumps(bad)).returncode,0)
        # More than one accrual chunk cannot be reconstructed from one final index.
        bad=json.loads(json.dumps(after));bad['state']['last_timestamp']=31556926001
        self.assertNotEqual(shell('flows/liquidation.sh',body,json.dumps(before),json.dumps(bad)).returncode,0)

    def test_operator_mutation_needs_receipt(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);(root/'configs').mkdir();(root/'logs').mkdir()
            (root/'configs/script.sh').write_text('echo noop\n')
            for verb,tag,ok in [('upgradeControllerHash','upgradeControllerHash',False),('setupAll','setupAll',False),('validateConfigs','validateConfigs',True),('setupAll','setupAll_replay',True)]:
                result=shell('flows/production.sh','RUN_DIR="$1"; LOG_DIR="$1/logs"; REPO_ROOT="$1"; ADMIN=admin; PROD_OP_TAG="$3"; prod_ops "$2"',d,verb,tag)
                self.assertEqual(result.returncode==0,ok,result.stderr)
    def test_operator_receipts_fetch_in_parallel_and_fail_closed(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);(root/'configs').mkdir();(root/'logs').mkdir()
            (root/'configs/script.sh').write_text("for n in $(seq 1 12); do printf 'Signing transaction: %064d\\n' $n >&2; done\n")
            body='''RUN_DIR="$1"; LOG_DIR="$1/logs"; REPO_ROOT="$1"; ADMIN=admin; GOVERNANCE=GOV
record() { echo "$1 $2 $4" >> "$RUN_DIR/records"; }
tx_status() { sleep 1; echo '{"result":{"envelopeXdr":"x"}}' > "$LOG_DIR/$1.receipt.json"; [ "$1" = "${BAD:-}" ] && echo FAILED || echo SUCCESS; }
fetch_resources() { RES_INSTR=1 RES_READ=2 RES_WRITE=3 RES_FEE=4; }
stellar() { echo '{"tx":{"tx":{"operations":[{"body":{"invoke_host_function":{"host_function":{"invoke_contract":{"function_name":"propose","contract_address":"GOV"}}}}}]}}}'; }
start=$(date +%s); prod_ops setupAll >/dev/null; rc=$?; echo "$(( $(date +%s) - start ))" > "$RUN_DIR/elapsed"; exit $rc'''
            result=shell('flows/production.sh',body,d)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertLess(int((root/'elapsed').read_text()),6)
            rows=(root/'records').read_text().split('\n')
            self.assertEqual([r.split()[2] for r in rows if r.startswith('operator_setupAll_tx_')],[f'{n:064d}' for n in range(1,13)])
            (root/'records').unlink()
            result=shell('flows/production.sh','BAD=$(printf "%064d" 7); '+body,d)
            self.assertNotEqual(result.returncode,0)
            self.assertNotIn('operator_setupAll ok',(root/'records').read_text())
    def test_setup_binds_proposals_to_executions_and_replay_sends_nothing(self):
        def call(fn,salt):
            return {'function_name':fn,'contract_address':'GOV','args':[{'address':'ADMIN'},{'bytes':salt}]}
        for tag,calls,ok in [('setupAll',[call('propose','aa'),call('propose','bb'),call('execute','aa'),call('execute','bb')],True),
                             ('setupAll',[call('propose','aa'),call('propose','bb'),call('execute','aa')],False),
                             ('setupAll',[call('propose','aa'),call('propose','bb'),call('execute','aa'),call('execute','aa')],False),
                             ('setupAll_replay',[call('propose','aa')],False),
                             ('setupAll_replay',[],True)]:
            with tempfile.TemporaryDirectory() as d:
                root=Path(d);(root/'configs').mkdir();(root/'logs').mkdir()
                for n,c in enumerate(calls,1): (root/f'inv.{n}').write_text(json.dumps(c))
                (root/'configs/script.sh').write_text(''.join(f"echo 'Signing transaction: {n:064d}' >&2\n" for n in range(1,len(calls)+1)) or 'true\n')
                body='''RUN_DIR="$1"; LOG_DIR="$1/logs"; REPO_ROOT="$1"; ADMIN=admin; GOVERNANCE=GOV
record() { :; }
tx_status() { echo "{\\"result\\":{\\"envelopeXdr\\":\\"$(( 10#$1 ))\\"}}" > "$LOG_DIR/$1.receipt.json"; echo SUCCESS; }
fetch_resources() { RES_INSTR=1 RES_READ=2 RES_WRITE=3 RES_FEE=4; }
stellar() { local n; n=$(cat); echo "{\\"tx\\":{\\"tx\\":{\\"operations\\":[{\\"body\\":{\\"invoke_host_function\\":{\\"host_function\\":{\\"invoke_contract\\":$(cat "$RUN_DIR/inv.$n")}}}}]}}}"; }
PROD_OP_TAG="$2" prod_ops setupAll >/dev/null'''
                result=shell('flows/production.sh',body,d,tag)
                self.assertEqual(result.returncode==0,ok,(tag,len(calls),result.stderr[-300:]))
    def test_split_governance_op_binds_propose_execute_and_record(self):
        salt='ab'*32; op='cd'*32
        propose={'function_name':'propose','contract_address':'GOV','args':[{'address':'ADMIN'},{'vec':[]},{'bytes':salt}]}
        def execute(s_,target='CTRL',fn='set_price_aggregator'):
            return {'function_name':'execute','contract_address':'GOV','args':[{'address':'ADMIN'},{'address':target},{'symbol':fn},{'vec':[]},{'bytes':'00'*32},{'bytes':s_}]}
        for case,exec_call,record_salt,ok in [('good',execute(salt),salt,True),('salt',execute('ee'*32),salt,False),
                                              ('target',execute(salt,target='OTHER'),salt,False),('record',execute(salt),'ff'*32,False)]:
            with tempfile.TemporaryDirectory() as d:
                root=Path(d);(root/'configs').mkdir();(root/'logs').mkdir();(root/'ops/testnet').mkdir(parents=True)
                (root/'inv.1').write_text(json.dumps(propose));(root/'inv.2').write_text(json.dumps(exec_call))
                (root/'configs/script.sh').write_text(f"""case "$1" in
  awaitOp) exit 0;;
  executeOp) printf 'Signing transaction: %064d\\n' 2 >&2;;
  *) printf 'Signing transaction: %064d\\n' 1 >&2
     echo '{{"kind":"controller","target":"CTRL","function":"set_price_aggregator","salt":"{record_salt}"}}' > "$OPS_ROOT/testnet/{op}.json"
     echo "Scheduled op {op} (AUTO_EXECUTE=0; run 'executeOp {op}' after the delay)." >&2;;
esac
""")
                body='''RUN_DIR="$1"; LOG_DIR="$1/logs"; REPO_ROOT="$1"; ADMIN=admin; GOVERNANCE=GOV; STATE_ENV="$1/state.env"
record() { echo "$1 $2 $3" >> "$RUN_DIR/records"; }
tx_status() { echo "{\\"result\\":{\\"envelopeXdr\\":\\"$(( 10#$1 ))\\"}}" > "$LOG_DIR/$1.receipt.json"; echo SUCCESS; }
fetch_resources() { RES_INSTR=1 RES_READ=2 RES_WRITE=3 RES_FEE=4; }
stellar() { local n; n=$(cat); echo "{\\"tx\\":{\\"tx\\":{\\"operations\\":[{\\"body\\":{\\"invoke_host_function\\":{\\"host_function\\":{\\"invoke_contract\\":$(cat "$RUN_DIR/inv.$n")}}}}]}}}"; }
source "$1/lib/core.sh" 2>/dev/null || true
save_state() { eval "$1=\\$2"; }
prod_propose setPriceAggregator setPriceAggregator || exit 3
grep -q '^operator_setPriceAggregator ok' "$RUN_DIR/records" && exit 4
prod_execute_split setPriceAggregator setPriceAggregator >/dev/null'''
                result=shell('flows/production.sh',body,d)
                rows=(root/'records').read_text()
                self.assertEqual('operator_setPriceAggregator_propose ok propose' in rows,record_salt==salt,(case,rows))
                self.assertEqual(result.returncode==0,ok,(case,result.returncode,result.stderr[-500:]))
                self.assertEqual('operator_setPriceAggregator ok setPriceAggregator' in rows,ok,(case,rows))
        with tempfile.TemporaryDirectory() as d:
            (Path(d)/'logs').mkdir()
            result=shell('flows/production.sh','RUN_DIR="$1"; LOG_DIR="$1/logs"; prod_execute_split setAggregator setAggregator',d)
            self.assertNotEqual(result.returncode,0)
    def test_production_fixture_groups_keep_plan_order_and_serial_xoxno(self):
        plan={'CZ1':{'kind':'Reflector'},'CA2':{'kind':'RedStone'},'CM3':{'kind':'Xoxno'},'CB4':{'kind':'token','decimals':7,'name':'T'},'CC5':{'kind':'pool','decimals':7,'name':'LPPOOL'}}
        seeds=[dict(kind='Reflector',contract='R1',asset={'Other':'A'},price='1'),dict(kind='RedStone',contract='S',feed='F',price='2'),
               dict(kind='Xoxno',contract='X',feed='XF',price='30000000000'),dict(kind='RedStone',contract='S',feed='F',price='2'),
               dict(kind='Reflector',contract='R1',asset={'Other':'B'},price='3')]
        body=r'''source "$1/lib/core.sh"; source "$1/lib/assert.sh"; source "$1/flows/production.sh"
RUN_DIR="$2"; LOG_DIR="$2/logs"; ACTIONS_TSV="$2/actions.tsv"; STATE_ENV="$2/state.env"; PHASE=init
mkdir -p "$LOG_DIR"; printf 'seq\tphase\tlabel\tstatus\tfn\thash\tinstructions\tread_bytes\twrite_bytes\tresource_fee\tnote\n' > "$ACTIONS_TSV"
CHANNELS="c1 c2 c3"; ADMIN=admin; ADMIN_ADDR=GADMIN; INTEG_DIR="$2/fake"; REPO_ROOT=unused; FIXTURE_WASM_DIR=fx; WASM_DIR=w; NET_ARGS=(--network testnet)
phase() { PHASE="$1"; }; log() { :; }
cid() { printf 'C%055d' "$1" | tr 0-9 A-J; }
run_deploy() {
    local a prev='' src='' n
    for a; do [ "$prev" != --source ] || src="$a"; prev="$a"; done
    n=$(basename "$1" .out); n=${n#fixture_}; n=${n%%.*}
    command sleep "$(awk -v n="$n" 'BEGIN { print (6 - n) * 0.3 }')"
    echo "$n|$src" >> "$RUN_DIR/deploys"
    cid "$n" > "$1"; printf 'Signing transaction: %064d\n' "$n" > "$2"
    RES_INSTR=1 RES_READ=2 RES_WRITE=3 RES_FEE=4
}
inv() {
    printf '%s|%s|%s|%s|%s\n' "$1" "${E2E_JOB:-}" "$2" "$3" "$5" >> "$RUN_DIR/writes"
    record "$1" ok "$5" "$(python3 -c 'import hashlib, sys; print(hashlib.sha256(sys.argv[1].encode()).hexdigest())' "${E2E_JOB:-p} $*")" 1 2 3 4 '' transaction "$3"
}
flow_production_fixtures'''
        def run(d,fixture_seeds):
            fake=Path(d)/'fake';fake.mkdir()
            (fake/'fixtures.json').write_text(json.dumps(dict(bases={'CZ1x':{'Other':'USD'}},seeds=fixture_seeds,pools=[dict(contract='P',snapshot={})])))
            (fake/'production_config.py').write_text(f'''import json, shutil, sys
from pathlib import Path
if sys.argv[1] == 'plan':
    print({json.dumps(json.dumps(plan))})
else:
    shutil.copy(Path(__file__).with_name('fixtures.json'), Path(sys.argv[3])/'fixtures.json')
''')
            result=subprocess.run(['bash','-c',body,'_',str(HERE),d],capture_output=True,text=True)
            root=Path(d)
            writes=[dict(zip(('label','job','signer','contract','fn'),w.split('|'))) for w in (root/'writes').read_text().splitlines()] if (root/'writes').exists() else []
            actions=list(csv.DictReader((root/'actions.tsv').read_text().splitlines(),delimiter='\t'))
            evidence=list(csv.DictReader((root/'evidence.tsv').read_text().splitlines(),delimiter='\t'))
            return result,root,writes,actions,evidence
        with tempfile.TemporaryDirectory() as d:
            result,root,writes,actions,evidence=run(d,seeds)
            mapping=json.loads((root/'address-map.json').read_text())
            seed_jobs=[j for g in root.glob('jobs/prod_seeds.*') for j in g.iterdir() if j.name.isdigit()]
            deploys=sorted((root/'deploys').read_text().split())
        self.assertEqual(result.returncode,0,result.stderr)
        cid=lambda n:'C'+f'{n:055d}'.translate(str.maketrans('0123456789','ABCDEFGHIJ'))
        self.assertEqual(list(mapping.items()),[(k,cid(n)) for n,k in enumerate(plan,1)])
        fixtures=[(a,e) for a,e in zip(actions,evidence) if a['label'].startswith('production_fixture_')]
        self.assertEqual([(a['label'],a['status'],a['fn'],e['execution'],e['contract']) for a,e in fixtures],
                         [(f'production_fixture_{n}','ok','deploy','deployment',cid(n)) for n in range(1,6)])
        self.assertTrue(all(len(a['hash'])==64 for a,_ in fixtures))
        self.assertEqual(deploys,[f'{n}|c{(n-1)%3+1}' for n in range(1,6)])
        required=next(c for c in json.loads((HERE/'cases.json').read_text()) if c['id']=='flow_production_fixtures')['required_actions']
        for r in required:
            self.assertGreaterEqual(len([a for a in actions if (a['label'],a['fn'],a['status'])==(r['label'],r['method'],r['status'])]),r['count'],r)
        for w in writes:
            serial=w['label'] in ('prod_xoxno_register','prod_xoxno_seed','prod_lp_fixture')
            self.assertEqual((w['job']=='',w['signer']=='admin'),(serial,serial),w)
        self.assertEqual(len([w for w in writes if w['label']=='prod_redstone_seed']),1)
        self.assertEqual(len(seed_jobs),3)
        seeds[3]=dict(seeds[3],price='9')
        with tempfile.TemporaryDirectory() as d:
            result,root,writes,actions,_=run(d,seeds)
            spawned=list(root.glob('jobs/prod_seeds.*'))+list(root.glob('jobs/prod_bases.*'))
        self.assertNotEqual(result.returncode,0)
        self.assertEqual(spawned,[])
        self.assertEqual([w for w in writes if w['label'] in ('prod_reflector_seed','prod_redstone_seed','prod_reflector_base')],[])
        self.assertEqual([(a['label'],a['status']) for a in actions if a['status']=='FAIL'],[('prod_seed_dedupe','FAIL')])
        self.assertIn('conflicting prices',next(a['note'] for a in actions if a['label']=='prod_seed_dedupe'))
    GROUPED_READS = r"""source "$1/lib/core.sh"; source "$1/lib/invoke.sh"; source "$1/lib/assert.sh"; source "$1/lib/assets.sh"; source "$1/lib/protocol.sh"
source "$1/flows/lifecycle.sh"; source "$1/flows/teardown.sh"; source "$1/flows/production.sh"
set -uo pipefail
RUN_DIR="$2"; LOG_DIR="$2/logs"; ACTIONS_TSV="$2/actions.tsv"; STATE_ENV="$2/state.env"; PHASE=init; INTEG_DIR="$2/integ"
mkdir -p "$LOG_DIR"; printf 'seq\tphase\tlabel\tstatus\tfn\thash\tinstructions\tread_bytes\twrite_bytes\tresource_fee\tnote\n' > "$ACTIONS_TSV"
ADMIN=admin ADMIN_ADDR=GADMIN ALICE_ADDR=GALICE BOB_ADDR=GISSUER RPC_URL=rpc NET_ARGS=(--network testnet)
CONTROLLER=CCTRL POOL=CPOOL POSITION_NFT=CNFT GOVERNANCE=CGOV PRICE_AGGREGATOR=CPA PRIMARY_SPOKE_ID=4
log() { :; }; latest_ledger() { echo 1; }
cat > "$RUN_DIR/node" <<'NODE'
#!/bin/bash
out=$6; shift 6
jq -n '[$ARGS.positional[] | {(.): {balance: "3"}}] | add // {}' --args "$@" > "$out"
NODE
chmod +x "$RUN_DIR/node"; NODE_BIN="$RUN_DIR/node"
stellar() {
    local a prev='' id='' fn='' args='' after=0 n v
    for a; do
        if [ "$after" = 1 ]; then if [ -z "$fn" ]; then fn="$a"; else args="$args $a"; fi; continue; fi
        [ "$a" != -- ] || after=1
        [ "$prev" != --id ] || id="$a"; prev="$a"
    done
    [ -z "${E2E_JOB:-}" ] || command sleep "${DELAY_UNIT:-0.}$(( (20 - ${E2E_JOB##*-}) % 10 ))"
    v=$(printf '%s' "$id$args" | cksum | cut -d' ' -f1)
    case "$fn" in
        name) if [ "$id" = SAC2 ]; then echo '"USDC:GISSUER"'; else echo "\"name-$id\""; fi;;
        total_supply) n=$(( $(cat "$RUN_DIR/totals" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$RUN_DIR/totals"; echo "${TOTAL_SEQ:-3 3}" | cut -d' ' -f"$n";;
        get_token_id) echo "\"$(( ${args##* } + 10 ))\"";;
        account_exists) echo false;;
        get_revenue) echo '"0"';;
        get_reserves) echo "\"$(( v % 10 ))\"";;
        get_borrowed_amount|get_supplied_amount) echo "\"$(( v % 20 ))\"";;
        balance) case "$args" in *CPOOL*) echo "\"$(( 10 + v % 20000 ))\"";; *) echo "\"$(( v % 2000 ))\"";; esac;;
        *) jq -nc --arg fn "$fn" --arg id "$id" --arg args "$args" '{fn:$fn,id:$id,args:$args}';;
    esac
}
"""

    SERIAL_SNAPSHOT = r"""
serial_snapshot() {
    local label="$1" acct="$2" asset="$3" runner="$4" positions attributes usage owner wallet pool
    positions=$(view "${label}_positions" "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    attributes=$(view "${label}_attributes" "$CONTROLLER" -- get_account_attributes --account_id "$acct") || return 1
    usage=$(view "${label}_usage" "$CONTROLLER" -- get_spoke_usage --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key 1 "$asset")") || return 1
    owner=$(view "${label}_owner" "$POSITION_NFT" -- owner_of --token_id "$acct") || return 1
    local controller_cash book nft_state roles='[]' role held pa_owner
    book=$(view "${label}_pool_book" "$POOL" -- get_sync_data --hub_asset "$(hub_key 1 "$asset")") || return 1
    nft_state=$(jq -nc --argjson name "$(view "${label}_name" "$POSITION_NFT" -- name)" \
        --argjson symbol "$(view "${label}_symbol" "$POSITION_NFT" -- symbol)" \
        --argjson uri "$(view "${label}_uri" "$POSITION_NFT" -- token_uri --token_id "$acct")" \
        --argjson total "$(view "${label}_total" "$POSITION_NFT" -- total_supply)" \
        '{name:$name,symbol:$symbol,uri:$uri,total:$total}') || return 1
    for role in PROPOSER EXECUTOR CANCELLER GUARDIAN ORACLE; do
        held=$(view "${label}_role_$role" "$GOVERNANCE" -- has_role --account "$ADMIN_ADDR" --role "$role") || return 1
        roles=$(jq -nc --argjson roles "$roles" --arg role "$role" --argjson held "$held" '$roles+[{role:$role,held:$held}]') || return 1
    done
    pa_owner=$(view "${label}_pa_owner" "$PRICE_AGGREGATOR" -- get_owner) || return 1
    controller_cash=$(balance "$asset" "$CONTROLLER") || return 1
    wallet=$(balance "$asset" "$runner") || return 1
    pool=$(balance "$asset" "$POOL") || return 1
    jq -ncS --argjson positions "$positions" --argjson attributes "$attributes" --argjson usage "$usage" \
        --argjson owner "$owner" --arg wallet "$wallet" --arg pool "$pool" --arg controller_cash "$controller_cash" \
        --argjson book "$book" --argjson nft "$nft_state" --argjson roles "$roles" --argjson pa_owner "$pa_owner" \
        '{positions:$positions,attributes:$attributes,usage:$usage,owner:$owner,wallet:$wallet,pool:$pool,controller_cash:$controller_cash,book:$book,nft:$nft,roles:$roles,pa_owner:$pa_owner}'
}
serial=$(serial_snapshot prod_snap 7 CUSDC CRUNNER) || exit 1
grouped=$(prod_position_snapshot prod_snap 7 CUSDC CRUNNER) || exit 2
printf '%s\n' "$serial" > "$RUN_DIR/serial.json"; printf '%s\n' "$grouped" > "$RUN_DIR/grouped.json"
"""

    SERIAL_TEARDOWN = r"""
for m in $MARKETS; do E2E_JOB_DIR="$RUN_DIR" td_snapshot_job "${m%%:*}" "${m##*:}" || exit 1; done
mv "$RUN_DIR/snapshot.jsonl" "$RUN_DIR/before-cleanup.jsonl"
for m in $MARKETS; do td_residue_job "${m%%:*}" "${m##*:}"; done
"""

    @staticmethod
    def rows(root):
        return [line.split('\t') for line in (root/'actions.tsv').read_text().splitlines()[1:]]

    def test_position_snapshot_group_equals_serial(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            result = subprocess.run(['bash', '-c', self.GROUPED_READS + self.SERIAL_SNAPSHOT, '_', str(HERE), d], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr[-2000:])
            serial, grouped = (root/'serial.json').read_text(), (root/'grouped.json').read_text()
            rows = self.rows(root)
        self.assertEqual(grouped, serial)
        snapshot = json.loads(grouped)
        self.assertEqual([r['role'] for r in snapshot['roles']], ['PROPOSER', 'EXECUTOR', 'CANCELLER', 'GUARDIAN', 'ORACLE'])
        self.assertEqual(snapshot['nft']['name'], 'name-CNFT')
        self.assertEqual(len({snapshot['wallet'], snapshot['pool'], snapshot['controller_cash']}), 3)
        floor = next(i for i, r in enumerate(rows) if r[2] == 'prod_snap_ledger_floor')
        self.assertEqual(floor, 18)
        self.assertEqual([r[2:] for r in rows[floor+1:]], [r[2:] for r in rows[:floor]])

    def test_teardown_trustline_reads_share_the_simulation_slots(self):
        body = self.GROUPED_READS + r'''
E2E_SIM_SLOTS=1 DELAY_UNIT=0.0
cat > "$RUN_DIR/node" <<'NODE'
#!/bin/bash
span="$(dirname "$0")/span"; echo "start node" >> "$span"; sleep 0.3; echo "end node" >> "$span"
out=$6; shift 6
jq -n '[$ARGS.positional[] | {(.): {balance: "3"}}] | add // {}' --args "$@" > "$out"
NODE
eval "$(declare -f stellar | sed '1s/^stellar/stellar_mock/')"
stellar() { echo "start view" >> "$RUN_DIR/span"; stellar_mock "$@"; local rc=$?; echo "end view" >> "$RUN_DIR/span"; return "$rc"; }
group_begin before_cleanup 8 reads || exit 1
group_spawn td_snapshot_job 1 SAC2; group_spawn td_snapshot_job 2 SAC2
group_end || exit 2
'''
        with tempfile.TemporaryDirectory() as d:
            result = subprocess.run(['bash', '-c', body, '_', str(HERE), d], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr[-2000:])
            spans = (Path(d)/'span').read_text().split('\n')[:-1]
        self.assertEqual(spans.count('start node'), 2)
        self.assertEqual([line.split()[0] for line in spans], ['start', 'end'] * (len(spans) // 2), spans)

    def test_teardown_groups_keep_serial_rows_and_snapshot_order(self):
        markets = ' '.join([f'1:SAC{n}' for n in range(1, 9)] + ['2:SAC1', '1:SACA'])
        grouped = self.GROUPED_READS + f'MARKETS="{markets}"; TOTAL_SEQ="3 0"; DELAY_UNIT=0.0\nflow_teardown; echo "$?" > "$RUN_DIR/rc"\n'
        serial = self.GROUPED_READS + f'MARKETS="{markets}"\n' + self.SERIAL_TEARDOWN
        with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
            ran, ref = [subprocess.Popen(['bash', '-c', body, '_', str(HERE), d], stderr=subprocess.PIPE, text=True) for body, d in ((grouped, a), (serial, b))]
            ran_err, ref_err = ran.communicate()[1], ref.communicate()[1]
            self.assertEqual(ref.returncode, 0, ref_err[-2000:])
            self.assertEqual((Path(a)/'rc').read_text().strip(), '1', ran_err[-2000:])
            got, want = (Path(a)/'before-cleanup.jsonl').read_text(), (Path(b)/'before-cleanup.jsonl').read_text()
            rows, reference = self.rows(Path(a)), self.rows(Path(b))
        self.assertEqual(got, want)
        lines = [json.loads(line) for line in got.splitlines()]
        self.assertEqual([f"{r['hub']}:{r['asset']}" for r in lines if 'hub' in r], markets.split())
        labels = [r[2] for r in rows]
        start, end = labels.index('before_cleanup_ledger_floor'), labels.index('pre_cleanup_conservation')
        snapshot_rows = [r[2:] for r in rows[start+1:end]]
        self.assertEqual(snapshot_rows, [r[2:] for r in reference[:len(snapshot_rows)]])
        residue_rows = [r[2:] for r in rows[labels.index('td_residue_ledger_floor')+1:]]
        self.assertEqual(residue_rows, [r[2:] for r in reference[len(snapshot_rows):]])
        self.assertTrue(any(r[1] == 'FAIL' for r in residue_rows) and any(r[0].startswith('td_residue_') for r in residue_rows))
        self.assertEqual([l for l in labels if l.startswith('td_exists_')], ['td_exists_10', 'td_exists_11', 'td_exists_12'])
        self.assertEqual([l for l in labels if l.startswith('td_token_')], ['td_token_0', 'td_token_1', 'td_token_2'])

    def test_governance_wait_is_deadline_based(self):
        body='''count=$(mktemp); gov_state() { local n=$(( $(cat "$count") + 1 )); echo "$n" > "$count"; [ "$n" -ge "$READY_AT" ] && echo Ready || echo Waiting; }
echo 0 > "$count"; READY_AT="$1"; start=$(date +%s); out=$(gov_await_ready op "$2"); rc=$?; echo "$rc $out $(( $(date +%s) - start ))"; rm -f "$count"'''
        rc,state,elapsed=shell('flows/governance.sh',body,'3','180').stdout.split()
        self.assertEqual((rc,state),('0','Ready')); self.assertLessEqual(int(elapsed),4)
        rc,state,elapsed=shell('flows/governance.sh',body,'999','2').stdout.split()
        self.assertEqual((rc,state),('1','Waiting')); self.assertLessEqual(int(elapsed),5)
    def test_wallet_funding_is_parallel_and_new_wallet_calls_friendbot_only_when_unfunded(self):
        with tempfile.TemporaryDirectory() as d:
            body='''source "$2/lib/core.sh"; record() { :; }; log() { :; }; backoff_sleep() { :; }
RUN_DIR="$1"; LOG_DIR="$1"; INTEG_DIR="$1"; RUN_TS=t; NET_ARGS=(--rpc-url x); save_state() { :; }; die() { exit 9; }
stellar() { case "$1 $2" in 'keys address') [ -f "$RUN_DIR/key.$3" ] && echo "G$3" || return 1;; 'keys generate') sleep 1; touch "$RUN_DIR/key.$3";; esac; }
curl() { local url; for url; do :; done; echo "$url" >> "$RUN_DIR/curl"
  case "$url" in *friendbot*) touch "$RUN_DIR/funded"; printf 200;; *horizon*) [ -f "$RUN_DIR/funded" ] || [ -z "${UNFUNDED:-}" ] || return 22
  echo '{"balances":[{"asset_type":"native","balance":"10000.0"}]}';; esac; }
start=$(date +%s); prefund_wallets admin alice bob carol dave; echo "$(( $(date +%s) - start ))" > "$RUN_DIR/prefund"
new_wallet ADMIN admin'''
            result=shell('lib/wallet.sh',body,d,str(HERE))
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertLessEqual(int((Path(d)/'prefund').read_text()),3)
            calls=(Path(d)/'curl').read_text().split()
            self.assertEqual(sum('friendbot' in c for c in calls),5)
            self.assertEqual(calls[-1],'https://horizon-testnet.stellar.org/accounts/Ge2e_admin_t')
            (Path(d)/'curl').unlink();(Path(d)/'funded').unlink()
            result=shell('lib/wallet.sh','UNFUNDED=1; '+body.replace('prefund_wallets admin alice bob carol dave','true'),d,str(HERE))
            self.assertEqual(result.returncode,0,result.stderr)
            calls=(Path(d)/'curl').read_text().split()
            self.assertEqual([('friendbot' in c) for c in calls],[False,True,False])
    def test_grant_guardian_is_proposed_before_governance_work(self):
        body='''ADMIN=admin ALICE=alice DAVE_ADDR=GDAVE GOVERNANCE=GOV GOV_CONTROLLER=CTRL
phase() { :; }; xfail() { :; }; gov_assert_state() { :; }; pay_vec() { echo '[]'; }; view() { echo 5; }
gov_scval_args() { echo '[]'; }
inv() { echo "inv $1" >> "$1.log"; echo "op_$1"; }
gov_await_ready() { echo "await $1" >> "$1.log"; echo Ready; }'''
        with tempfile.TemporaryDirectory() as d:
            result=shell('flows/governance.sh',body.replace('$1.log',d+'/calls')+'\nflow_governance')
            self.assertEqual(result.returncode,0,result.stderr)
            calls=(Path(d)/'calls').read_text().splitlines()
        self.assertEqual(calls[0],'inv gov_propose_grant_guardian')
        self.assertLess(calls.index('inv gov_propose_grant_guardian'),calls.index('inv gov_create_hub'))
        self.assertEqual(calls.count('inv gov_propose_grant_guardian'),1)
        self.assertLess(calls.index('await op_gov_propose_grant_guardian'),calls.index('inv gov_execute_grant_guardian'))
    def test_liq_setup_lists_only_the_lane_markets(self):
        body='''ADMIN=admin ALICE=alice BOB=bob CAROL=carol ADMIN_ADDR=GADMIN BOB_ADDR=GBOB CAROL_ADDR=GCAROL PRIMARY_HUB_ID=1 PRIMARY_SPOKE_ID=1 CONTROLLER=CTRL WAD=1
phase() { :; }; deploy_mock_reflector() { :; }; deploy_mock_redstone() { :; }; dual_px() { :; }; save_state() { :; }
issue_sac() { eval "$1=SAC$2"; }; oracle_cfg_mock_dual() { echo '{}'; }; asset_config_json() { echo "$3"; }; pay_vec() { shift; echo "$*"; }
trustline() { echo "trustline $*" >> "$LOG"; }; mint_to() { echo "mint_to $*" >> "$LOG"; }
classic_batch() { echo "batch $*" >> "$LOG"; }
create_market() { echo "market $1 $3 $6" >> "$LOG"; }
inv() { echo "inv $1 ${13}" >> "$LOG"; }
E2E_LANE=liq-c flow_liq_setup'''
        with tempfile.TemporaryDirectory() as d:
            result=shell('flows/liquidation.sh',f'LOG={d}/calls; '+body)
            self.assertEqual(result.returncode,0,result.stderr)
            calls=(Path(d)/'calls').read_text().splitlines()
        self.assertEqual([c for c in calls if c.startswith('market')],['market LIQE SACLIQE 200','market LIQF SACLIQF 200'])
        self.assertEqual([c for c in calls if c.startswith('inv')],['inv liq_seed_liquidity SACLIQF 500000000000'])
        self.assertEqual([c for c in calls if c.startswith(('batch','trustline','mint_to'))],
                         ['batch liq_trust_bob change_trust bob trust:LIQE:GADMIN trust:LIQF:GADMIN',
                          'batch liq_trust_carol change_trust carol trust:LIQE:GADMIN trust:LIQF:GADMIN',
                          'batch liq_mint_classic payment admin pay:GBOB:LIQE:GADMIN:1000000000000 pay:GCAROL:LIQE:GADMIN:1000000000000'
                          ' pay:GBOB:LIQF:GADMIN:1000000000000 pay:GCAROL:LIQF:GADMIN:1000000000000'])
    def test_flash_fee_destination(self):
        p=dict(borrowed='0',supply_index=str(10**27),revenue='0',supplied=str(100*10**27),cash='1000000000')
        q={**p,'revenue':str(50000*10**20),'supplied':str(100*10**27+50000*10**20),'cash':'1000050000'}
        body='flash_loan_fee_state "$1" "$2" 50000 7'
        self.assertEqual(shell('flows/strategies.sh',body,json.dumps(p),json.dumps(q)).returncode,0)
        for field in ['revenue','supplied','cash']:
            bad={**q,field:str(int(q[field])-1)}
            self.assertNotEqual(shell('flows/strategies.sh',body,json.dumps(p),json.dumps(bad)).returncode,0)
    def test_route_header_roundtrip(self):
        value={'map':[{'key':{'symbol':k},'val':v} for k,v in [('amounts',{'vec':[{'i128':'10000000'}]}),('assets',{'vec':[]}),('ops',{'bytes':'01020304000000000506070809'})]]}
        raw=subprocess.check_output(['stellar','xdr','encode','--type','ScVal'],input=json.dumps(value).encode()).strip()
        import base64
        result=shell('lib/aggregator.sh','agg_referral_route_hex "$1" 16909060',base64.b64decode(raw).hex())
        self.assertEqual(result.returncode,0,result.stderr)
        decoded=json.loads(subprocess.check_output(['stellar','xdr','decode','--type','ScVal','--output','json'],input=base64.b64encode(bytes.fromhex(result.stdout))))
        expected=json.loads(json.dumps(value));expected['map'][2]['val']['bytes']='01020304010203040506070809'
        self.assertEqual(decoded,expected)

if __name__=='__main__':unittest.main()
