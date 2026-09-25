"""Integer principal/refund reference and suppressed-refund regressions."""
import json
import subprocess
import unittest
from pathlib import Path

HERE=Path(__file__).resolve().parent
KEY={'asset':'CTOKEN','hub_id':1}
R=10**27

def position(side,shares):
    p=[{},{}]
    if shares:p[side][json.dumps(KEY)]={'scaled_amount':str(shares)}
    return p

def amount(method,old,new,index,offered,decimals):
    side=0 if method in ['supply','withdraw'] else 1
    args=[json.dumps(x) for x in [method,KEY,position(side,old),position(side,new),{'state':{'supply_index':str(index),'borrow_index':str(index)}}]]+[str(offered),str(decimals)]
    return subprocess.run(['bash','-c','source "$1"; shift; lifecycle_amount "$@"','_',str(HERE/'flows/lifecycle.sh'),*args],capture_output=True,text=True)

class LifecycleMath(unittest.TestCase):
    def test_rounding_and_full_refund(self):
        for d in [7,8,9,18]:
            u=10**(27-d); i=R+123456789; n=10**d+1
            floor=n*u*R//i; ceil=(n*u*R+i-1)//i
            for method,old,new,offer,want in [('supply',0,floor,n,n),('borrow',0,ceil,n,n),('repay',ceil,0,n+100,n+1),('withdraw',floor,0,0,n-1)]:
                got=amount(method,old,new,i,offer,d)
                self.assertEqual(got.returncode,0,got.stderr)
                self.assertEqual(int(got.stdout),want)
                self.assertNotEqual(amount(method,old,new+1,i,offer,d).returncode,0)
    def test_partial_burn(self):
        for method in ['repay','withdraw']:
            i=R+999; u=10**20; n=17
            burn=n*u*R//i if method=='repay' else (n*u*R+i-1)//i
            self.assertEqual(amount(method,100*u,100*u-burn,i,n,7).stdout,'17\n')
    def test_sdk_transport_uses_pinned_builder_arguments(self):
        script = r'''source "$1/flows/sdk.sh"
ALICE=alice; ALICE_ADDR=CALLER; CONTROLLER=CTRL; PRIMARY_SPOKE_ID=2
sdk_inv() { printf '%s\n%s\n' "$2" "$3"; }
sdk_lifecycle_submit check alice CTRL -- "$2" --caller CALLER --account_id "$3" --spoke_id 2 --assets '[[{"asset":"TOKEN","hub_id":1},"1000000000000000001"]]'
'''
        for method,builder in [('supply','Supply'),('borrow','Borrow'),('repay','Repay'),('withdraw','Withdraw')]:
            result=subprocess.run(['bash','-c',script,'_',str(HERE),method,'0'],text=True,capture_output=True)
            self.assertEqual(result.returncode,0,result.stderr)
            name,args=result.stdout.splitlines()
            self.assertEqual(name,'buildStellar'+builder+'Tx')
            self.assertEqual(json.loads(args),dict(asset='TOKEN',hubId=1,amount='1000000000000000001',accountNonce='0',spokeId=2))
        self.assertNotEqual(subprocess.run(['bash','-c',script,'_',str(HERE),'upgrade','0'],capture_output=True).returncode,0)

    def test_suppressed_refund_fails_cash_predicate(self):
        # Offered 110, current debt100: payer loses100, not110.
        paid=amount('repay',100*10**20,0,R,110,7)
        self.assertEqual(paid.stdout,'100\n')
        for final,ok in [('900',True),('890',False)]:
            r=subprocess.run(['bash','-c','source "$1"; record() { :; }; log() { :; }; assert_delta refund 1000 "$2" -100','_',str(HERE/'lib/assert.sh'),final],capture_output=True)
            self.assertEqual(r.returncode==0,ok)

if __name__=='__main__': unittest.main()
