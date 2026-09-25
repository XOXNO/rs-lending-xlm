"""Fault injection for risk snapshots, NFT enumeration and route headers."""
import json
import subprocess
import unittest
import tempfile
from pathlib import Path
HERE=Path(__file__).resolve().parent

def shell(file,body,*args):
    return subprocess.run(['bash','-c','source "$1"; source "$2"; shift 2; record() { :; }; log() { :; }; _assert_fail() { return 1; }; '+body,'_',str(HERE/'lib/assert.sh'),str(HERE/file),*args],capture_output=True,text=True)

class Predicates(unittest.TestCase):
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
