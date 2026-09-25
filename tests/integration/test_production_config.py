#!/usr/bin/env python3
"""Offline policy/decimal assertions; no network or signing keys required."""
import json
import subprocess
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path

from production_config import decimal_cases, fixture_price, lending_case, load, materialize, plan, policy_checks, verify_equal, verify_price


class ProductionPolicyChecks(unittest.TestCase):
    def test_operator_refreshes_expired_fixture_twap_after_setup(self):
        script = Path(__file__).resolve().parent / 'flows/production.sh'
        seeds = [dict(kind='Reflector', contract='R', asset={'Other': 'XLM'}, price='525000000000000000'),
                 dict(kind='Reflector', contract='Q', asset={'Other': 'USDC'}, price='1000000000000000000'),
                 dict(kind='RedStone', contract='S', feed='XLM', price='525000000000000000')]
        body = r'''
set -uo pipefail
source "$1"
RUN_DIR="$2"; LOG_DIR="$2/logs"; NETWORKS_FILE="$2/networks.json"
GOV_CONTROLLER=CTRL; GOVERNANCE=GOV; ADMIN=admin; ADMIN_ADDR=owner
PA_HASH=pa; POOL_HASH=pool; NFT_HASH=nft; OWNED_AGGREGATOR=router
RPC_URL=rpc; NETWORK_PASSPHRASE=network; INTEG_DIR=unused
now=0; fresh_at=0; refreshed=0
phase() { :; }; record() { :; }; _assert_fail() { return 1; }
save_state() { printf -v "$1" '%s' "$2"; }
is_contract_id() { :; }; verify_candidate_contract() { :; }
assert_view_eq_at() { :; }; prod_verify_policy() { :; }
python3() { [ "$2" = verify-price ]; }
price_key_token() { printf '{"Token":"%s"}' "$1"; }
inv() {
    if [ "$5" = set_price ]; then
        [ "$3" = R ] || [ "$3" = Q ] || return 1
        [ "${FAIL_REFRESH:-0}" != 1 ] || return 1
        printf '%s\t%s\t%s\n' "$3" "$7" "$9" >> "$RUN_DIR/refreshed.tsv"
        fresh_at=$now; refreshed=$((refreshed+1))
    else echo '"PA"'; fi
}
prod_ops() {
    case "$1" in
        deployPool) echo '"POOL"';; deployPositionNft) echo '"NFT"';;
        setupAll)
            if [ "${PROD_OP_TAG:-}" = setupAll_replay ]; then now=$((now+360)); else now=$((now+3000)); fi
            jq '.testnet.spoke_ids={"1":1}' "$RUN_DIR/config/networks.json" > "$RUN_DIR/config/networks.tmp"
            mv "$RUN_DIR/config/networks.tmp" "$RUN_DIR/config/networks.json";;
    esac
}
view() {
    # Three mock observations are now, now-300, now-600. A 56-minute
    # operator setup expires the oldest one under the unchanged 3600s cap.
    [ "$refreshed" = 2 ] && [ "$((now-fresh_at+600))" -le 3600 ] || return 1
    echo '{}'
}
[ "$3" != missing ] || prod_refresh_reflectors() { :; }
[ "$3" != failed ] || FAIL_REFRESH=1
flow_production_operator
'''
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'config/testnet').mkdir(parents=True)
            (root / 'logs').mkdir()
            (root / 'networks.json').write_text('{"testnet":{}}')
            fixtures = root / 'config/testnet/fixtures.json'
            fixtures.write_text(json.dumps({'seeds': seeds}))
            (root / 'config/testnet/markets.json').write_text(json.dumps({'markets': [dict(
                name='XLM', asset_address='XLM', hub_id=1, oracle={'asset_decimals': 7})]}))
            before = fixtures.read_bytes()
            for mode in ('valid', 'missing', 'failed'):
                result = subprocess.run(['bash', '-c', body, '_', str(script), directory, mode], capture_output=True, text=True)
                self.assertEqual(result.returncode == 0, mode == 'valid', result.stderr)
                self.assertEqual(fixtures.read_bytes(), before)
            rows = [line.split('\t') for line in (root / 'refreshed.tsv').read_text().splitlines()]
            self.assertEqual(rows, [[s['contract'], json.dumps(s['asset'], separators=(',', ':')), s['price']] for s in seeds[:2]])

    def test_production_policy_readbacks_and_decimal_precision(self):
        source = Path(__file__).resolve().parents[2] / 'configs' / 'mainnet'
        mapping = {address: f'fixture-{i}' for i, address in enumerate(plan(source))}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            materialize(source, root, mapping, 'testnet')
            networks = {'testnet': {'spoke_ids': {key: int(key) + 20 for key in load(root, 'spokes.json')}}}
            checks = policy_checks(root, networks)
            markets, spokes = load(root, 'markets.json'), load(root, 'spokes.json')
            self.assertEqual(len(checks), len(markets['references']) + 2 * len(markets['markets'])
                + sum(1 + len(spoke['assets']) for spoke in spokes.values()))
            self.assertTrue(all(json.loads(c['args'][1]) >= 21 for c in checks if c['method'] == 'get_spoke'))
            for check in checks:
                verify_equal(check['expected'], deepcopy(check['expected']))
                broken = deepcopy(check['expected'])
                broken.pop(next(iter(broken)))
                with self.assertRaises(AssertionError):
                    verify_equal(check['expected'], broken)
            listing = next(c['expected'] for c in checks if c['method'] == 'get_spoke_asset')
            for field in ('supply_cap', 'borrow_cap', 'liquidation_fees'):
                broken = dict(listing, **{field: str(int(listing[field]) + 1)})
                with self.assertRaises(AssertionError):
                    verify_equal(listing, broken)
            lending = lending_case(root, networks)
            self.assertEqual(lending['hub_id'], 3)
            self.assertEqual(lending['spoke_id'], 29)
            self.assertGreater(int(lending['amount']), 0)
            cases = decimal_cases(root, networks)
            self.assertEqual([c['decimals'] for c in cases], [7, 8, 9, 18])
            self.assertEqual(cases[-1]['amount'], '1000000000000000001')
            for case in cases:
                self.assertGreater(int(case['amount']), int(case['partial']))
                self.assertIn(case['asset'], mapping.values())
            fixtures = load(root, 'fixtures.json')
            self.assertEqual(set(fixtures['source_files_sha256']), {'markets.json', 'hubs.json', 'spokes.json'})
            for key in fixtures['prices']:
                token = json.loads(key).get('Token')
                if token:
                    self.assertIn(token, mapping.values())
            for market in load(root, 'markets.json')['markets']:
                key = json.dumps({'Token': market['asset_address']}, sort_keys=True)
                value = {'price_wad': str(fixture_price(root, json.loads(key))), 'asset_decimals': market['oracle']['asset_decimals'], 'timestamp': 1}
                verify_price(root, market['name'], {key: value})
                with self.assertRaises(AssertionError):
                    verify_price(root, market['name'], {key: dict(value, price_wad=str(int(value['price_wad']) // 2))})
                with self.assertRaises(AssertionError):
                    verify_price(root, market['name'], {key: dict(value, price_wad=str(int(value['price_wad']) + 1))})
                with self.assertRaises(AssertionError):
                    verify_price(root, market['name'], {key: dict(value, asset_decimals=value['asset_decimals'] + 1)})

    def test_upgrade_receipts_bind_requested_target_and_hash(self):
        script = Path(__file__).resolve().parent/'flows/production.sh'
        wasm = 'a'*64
        operations = [('Controller','UpgradeController','CTRL','upgrade'), ('Pool','UpgradePool','CTRL','upgrade_pool'),
            ('PositionNft','UpgradePositionNft','CTRL','upgrade_position_nft'),
            ('PriceAggregator','UpgradePriceAggregator','PA','upgrade'), ('Governance','UpgradeGov','GOV','upgrade')]
        for verb, op, target, method in operations:
            proposal = {'contract_address':'GOV','function_name':'propose','args':[
                {'address':'OWNER'},{'vec':[{'symbol':op},{'bytes':wasm}]},{'bytes':'b'*64}]}
            execution = {'contract_address':'GOV','function_name':'execute_self' if target=='GOV' else 'execute',
                'args': ['void', proposal['args'][1], proposal['args'][2]] if target=='GOV' else
                ['void',{'address':target},{'symbol':method},{'vec':[{'bytes':wasm}]},{'bytes':'0'*64},{'bytes':'b'*64}]}
            for invocation in [proposal, execution]:
                def check(value, expected_hash=wasm):
                    return subprocess.run(['bash','-c','source "$1"; shift; prod_upgrade_invocation "$@"','_',
                        str(script),'upgrade'+verb+'Hash',expected_hash,'GOV','CTRL','PA',json.dumps(value)],capture_output=True)
                self.assertEqual(check(invocation).returncode,0)
                self.assertNotEqual(check(invocation,'c'*64).returncode,0)
                self.assertNotEqual(check(dict(invocation,contract_address='ATTACKER')).returncode,0)
                self.assertNotEqual(check(dict(invocation,function_name='unpause')).returncode,0)

    def test_contract_authority_transition_preserves_financial_state(self):
        script = Path(__file__).resolve().parent/'flows/production.sh'
        before = dict(owner='RUNNER', positions=[{'asset': {'scaled_amount':'1000000000000000000000000000'}}, {}],
                      attributes={'spoke_id':1,'mode':0}, usage={'supplied':'1000000000000000000000000000'},
                      book={'state':{'supplied':'1000000000000000000000000000','borrowed':'0','cash':'10000000'}},
                      wallet='1', pool='10000000', controller_cash='0', nft={'total':1}, roles=[], pa_owner='GOV')
        def check(after, owner, ok):
            result = subprocess.run(['bash','-c',
                'source "$1"; record() { :; }; _assert_fail() { return 1; }; prod_caller_state state "$2" "$3" "$4"',
                '_',str(script),json.dumps(before),json.dumps(after),owner],capture_output=True,text=True)
            self.assertEqual(result.returncode==0,ok,result.stderr)
        check(deepcopy(before),'RUNNER',True)
        transferred=deepcopy(before); transferred['owner']='CAROL'
        check(transferred,'CAROL',True)
        check(transferred,'RUNNER',False)
        for field, changed in [('owner','ATTACKER'),('wallet','2'),('pool','9999999'),('controller_cash','1'),
                               ('positions',[{},{}]),('attributes',{'spoke_id':2,'mode':0}),('usage',{}),
                               ('book',{}),('nft',{'total':0}),('roles',['unexpected']),('pa_owner','ATTACKER')]:
            bad=deepcopy(transferred); bad[field]=changed
            check(bad,'CAROL',False)

    def test_asymmetric_stable_reference(self):
        # Cubic invariant bracket: 2998146985239894576 < D < ...4577.
        # At shares=3e18 and min leg price=1e18, both endpoints floor here.
        market = {'name': 'LP', 'asset_address': 'LP', 'oracle': {'asset_decimals': 18,
            'sources': [{'AquariusStableLp': {'pool': 'POOL', 'key_a': {'Token': 'A'},
                'key_b': {'Token': 'B'}, 'reserve_a_decimals': 18, 'reserve_b_decimals': 18}}]}}
        references = [{'key': {'Token': t}, 'oracle': {'sources': [{'Feed': {'provider':
            {'RedStone': {'contract': 'ORACLE', 'feed_id': t}}}}]}} for t in ['A', 'B']]
        fixtures = {'seeds': [{'kind': 'RedStone', 'contract': 'ORACLE', 'feed': t, 'price': p}
            for t,p in [('A', '1000000000000000000'), ('B', '1100000000000000000')]],
            'pools': [{'contract': 'POOL', 'snapshot': {'reserves': ['1000000000000000000',
                '2000000000000000000'], 'shares': '3000000000000000000', 'amplification': '100'}}]}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/'markets.json').write_text(json.dumps({'markets': [market], 'references': references}))
            (root/'fixtures.json').write_text(json.dumps(fixtures))
            self.assertEqual(fixture_price(root, {'Token': 'LP'}), 999382328413298192)
            fixtures['pools'][0]['snapshot']['amplification'] = '200'
            (root/'fixtures.json').write_text(json.dumps(fixtures))
            self.assertNotEqual(fixture_price(root, {'Token': 'LP'}), 999382328413298192)

    def test_integer_normalization_does_not_hide_money_or_boolean_errors(self):
        verify_equal({'amount': '1000000000000000001'}, {'amount': 1000000000000000001})
        for bad in (1000000000000000000, 1e18, True, 'unreadable'):
            with self.assertRaises(AssertionError):
                verify_equal({'amount': '1000000000000000001'}, {'amount': bad})
        with self.assertRaises(AssertionError):
            verify_equal({'enabled': True}, {'enabled': 1})


if __name__ == '__main__':
    unittest.main()
