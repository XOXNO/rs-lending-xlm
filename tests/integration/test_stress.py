"""Offline checks for prepared-envelope stress sequencing and composition shape."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
FLOW = ROOT / 'tests/integration/flows/stress.sh'


class StressChecks(unittest.TestCase):
    def run_shell(self, code, directory, **env):
        return subprocess.run(['bash', '-c', 'set -uo pipefail\nsource "$FLOW"\n' + code],
                              env={**os.environ, 'FLOW': str(FLOW), 'WORK': str(directory), **env},
                              capture_output=True, text=True)

    def test_composed_roots_use_distinct_reference_keys_then_restore(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_shell(r'''
source "${FLOW%/flows/stress.sh}/lib/protocol.sh"
ADMIN=admin; PRICE_AGGREGATOR=pa
phase() { :; }
stress_sac() { echo "token$1"; }
stress_select_oracles() { MOCK="reflector-$1"; MOCKRS="redstone-$1"; }
inv() {
    local label="$1"; shift 5
    printf '%s\t%s\t%s\n' "$label" "$2" "$4" >> "$WORK/configs"
}
flow_stress_borrow_frontier() { echo "$1" > "$WORK/mode"; }
flow_stress_composed
''', directory)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(Path(directory, 'mode').read_text().strip(), 'composed')
            configs = [line.split('\t') for line in Path(directory, 'configs').read_text().splitlines()]
            self.assertEqual(len(configs), 30)
            for index in range(10):
                quote, token = configs[index * 2:index * 2 + 2]
                qkey, qcfg = json.loads(quote[1]), json.loads(quote[2])
                tcfg = json.loads(token[2])
                scaled = tcfg['sources'][0]['Scaled']
                self.assertEqual(qcfg['asset_decimals'], 0)
                self.assertEqual(scaled['quote'], qkey)
                self.assertIn('RedStone', scaled['factor']['provider'])
                self.assertIn('Reflector', qcfg['sources'][0]['Feed']['provider'])
                for config in (qcfg, tcfg):
                    lo, hi = int(config['min_sanity_price_wad']), int(config['max_sanity_price_wad'])
                    self.assertGreaterEqual((hi - lo) * 10000 // (hi + lo), 50)
                    self.assertLessEqual(((hi - lo) * 10000 + hi + lo - 1) // (hi + lo), 1000)
                    self.assertLess(lo, 10**18)
                    self.assertGreater(hi, 10**18)
            self.assertEqual(len({json.loads(x[1])['Ref'] for x in configs[:20:2]}), 10)
            for restore in configs[20:]:
                self.assertEqual(len(json.loads(restore[2])['sources']), 2)

    def test_delayed_borrow_never_rebuilds_after_shared_write_or_retries_send(self):
        for fail, status, budget in (('0', 'SUCCESS', '0'), ('1', 'SUCCESS', '0'),
                                     ('0', 'UNKNOWN', '0'), ('0', 'SUCCESS', '1')):
            failed = fail != '0' or status != 'SUCCESS' or budget != '0'
            with self.subTest(fail_send=fail, status=status, budget=budget), tempfile.TemporaryDirectory() as directory:
                result = self.run_shell(r'''
LOG_DIR="$WORK"; ACTIONS_TSV="$WORK/actions.tsv"; touch "$ACTIONS_TSV"; DAVE_DUAL_ACCT=7; PRIMARY_HUB_ID=1; PRIMARY_SPOKE_ID=1
DAVE=dave; DAVE_ADDR=dave_address; CAROL=carol; CAROL_ADDR=carol_address
CONTROLLER=controller; NETWORK_PASSPHRASE=test; STRESS_UNIT=10000000; NET_ARGS=(--network testnet)
phase() { :; }
stress_sac() { echo "token$1"; }
hub_key() { echo key; }
pay_vec() { echo '[]'; }
_view_int() { case "$1" in *before*) echo 100;; *) echo 10000100;; esac; }
balance() { if [ -f "$WORK/sent" ]; then echo 10000000000; else echo 0; fi; }
stress_latest_ledger() {
    local n=10
    [ ! -f "$WORK/ledger" ] || n=$(cat "$WORK/ledger")
    echo $((n+1)) > "$WORK/ledger"
    echo "$n"
}
sleep() { :; }
is_wasm_hash() { [[ "$1" =~ ^[a-f0-9]{64}$ ]]; }
inv() { echo "$1" >> "$WORK/sequence"; }
stellar() {
    echo "$1 $2" >> "$WORK/sequence"
    case "$1 $2" in
        'contract invoke') echo BUILT;;
        'tx simulate') [ "$(cat)" = BUILT ] || return 1; echo PREPARED;;
        'tx sign') [ "$(cat)" = PREPARED ] || return 1; echo SIGNED;;
        'tx hash') [ "$(cat)" = SIGNED ] || return 1; printf '%064d\n' 1;;
        'tx send') [ "$(cat)" = SIGNED ] || return 1; touch "$WORK/sent"; return "$FAIL_SEND";;
        *) return 1;;
    esac
}
record_attempt() { echo "$1 $4 $5 $6" >> "$WORK/attempts"; }
tx_status() { echo "$TX_STATUS"; }
fetch_resources() { RES_INSTR=1; RES_READ=2; RES_WRITE=3; RES_FEE=4; return "$BUDGET_FAIL"; }
record() { echo "$1 $2 $3" >> "$WORK/records"; }
_assert_fail() { echo "$*" >&2; return 1; }
assert_delta() { [ "$(( $3 - $2 ))" -eq "$4" ]; }
view() { echo '[{"a":1,"b":1,"c":1,"d":1,"e":1},{"a":1,"b":1,"c":1,"d":1,"e":1}]'; }
flow_stress_delayed
''', directory, FAIL_SEND=fail, TX_STATUS=status, BUDGET_FAIL=budget)
                self.assertEqual(result.returncode, int(failed), result.stderr)
                sequence = Path(directory, 'sequence').read_text().splitlines()
                self.assertEqual(sequence.count('tx simulate'), 1)
                self.assertEqual(sequence.count('tx send'), 1)
                self.assertLess(sequence.index('tx sign'), sequence.index('stress_shared_topup'))
                self.assertLess(sequence.index('stress_shared_topup'), sequence.index('tx send'))
                delay = json.loads(Path(directory, 'stress_delayed_borrow.delay.json').read_text())
                self.assertGreaterEqual(delay['submitted_after_ledger'] - delay['prepared_after_ledger'], 3)
                attempts = Path(directory, 'attempts').read_text().splitlines()
                self.assertEqual(len(attempts), 1)
                self.assertEqual(attempts[0].split()[1:3], ['1', fail])
                records = Path(directory, 'records').read_text()
                if failed:
                    self.assertIn('stress_delayed_borrow FAIL borrow', records)
                    self.assertNotIn('stress_delayed_repay', sequence)
                else:
                    self.assertIn('stress_delayed_borrow ok borrow', records)
                    self.assertIn('stress_delayed_dimensions ok assert', records)
                    self.assertIn('stress_delayed_repay', sequence)

    def test_latest_ledger_rejects_malformed_or_error_responses(self):
        for payload in ('{}', '{"error":{}}', 'garbage', '{"jsonrpc":"2.0","id":1,"result":{"sequence":1.5}}'):
            with self.subTest(payload=payload), tempfile.TemporaryDirectory() as directory:
                result = self.run_shell('curl() { printf "%s" "$PAYLOAD"; }; stress_latest_ledger', directory, PAYLOAD=payload)
                self.assertNotEqual(result.returncode, 0)


if __name__ == '__main__':
    unittest.main()
