"""Offline checks for prepared-envelope stress sequencing and composition shape."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
FLOW = ROOT / 'tests/integration/flows/stress.sh'


GROUPED = r'''
ROOT="${FLOW%/flows/stress.sh}"
for f in core assert oracle protocol; do source "$ROOT/lib/$f.sh"; done
RUN_DIR="$WORK"; LOG_DIR="$WORK/logs"; ACTIONS_TSV="$WORK/actions.tsv"; STATE_ENV="$WORK/state.env"; PHASE=init
mkdir -p "$LOG_DIR"; : > "$STATE_ENV"
printf 'seq\tphase\tlabel\tstatus\tfn\thash\tinstructions\tread_bytes\twrite_bytes\tresource_fee\tnote\n' > "$ACTIONS_TSV"
CHANNELS="c1 c2 c3 c4 c5 c6 c7 c8 c9 c10"
STRESS_N=20; STRESS_UNIT=10000000; WAD=1000000000000000000; ADMIN=admin; ADMIN_ADDR=GADMIN; DAVE=dave; DAVE_ADDR=GDAVE
CAROL=carol; CAROL_ADDR=GCAROL; PRIMARY_HUB_ID=1; PRIMARY_SPOKE_ID=1; CONTROLLER=controller; GOVERNANCE=gov; PRICE_AGGREGATOR=pa
FIXTURE_WASM_DIR=fx; NET_ARGS=(--network testnet)
phase() { PHASE="$1"; }
log() { :; }
cid() { printf 'C%055d' "$1" | tr 0-9 A-J; }
written() {
    printf '%s|%s|%s|%s|%s|%s\n' "$PHASE" "$1" "${E2E_JOB:-}" "$2" "$3" "$4" >> "$WORK/writes"
    [ -z "${E2E_JOB:-}" ] || printf '%s|%s\n' "$E2E_JOB" "$(cksum < "$STATE_ENV")" >> "$WORK/cksums"
}
run_deploy() {
    local a prev='' src='' wasm='' k
    for a; do case "$prev" in --source) src="$a";; --wasm) wasm="$a";; esac; prev="$a"; done
    k=$(( ${E2E_JOB##*-} * 2 )); [ "${wasm##*/}" != mock_redstone.wasm ] || k=$((k + 1))
    written "$(basename "$1" .out)" "$src" deploy "$(cid "$k")"
    cid "$k" > "$1"; printf 'Signing transaction: %064d\n' "$k" > "$2"
    RES_INSTR=1 RES_READ=2 RES_WRITE=3 RES_FEE=4
}
inv() { written "$1" "$2" "$5" "$3"; echo '"5"'; }
issue_sac() { written "issue_sac_$2" "${E2E_SRC:-}" asset_deploy ''; save_state "$1" "SAC$2"; }
classic_batch() { written "$1" "$3" batch ''; printf '%s\n' "batch $1 $2 $3 $(($# - 3))" "${@:4}" >> "$WORK/calls"; }
market_listing_exists() { return 1; }; market_wait_listed() { :; }
view() { written "$1" '' view "$2"; echo '{}'; }
eval "real_$(declare -f create_market)"
create_market() { printf '%s' "$5" > "$WORK/oracle_arg.$1"; real_create_market "$@"; }
'''


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
latest_ledger() {
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
        'tx send') [ "$(cat)" = SIGNED ] || return 1
            ! grep -qx "Signing transaction: $(printf '%064d' 1)" "$WORK/stress_delayed_borrow.err" || echo 'signing line before send' >> "$WORK/sequence"
            echo 'Transaction hash is sent' >&2; touch "$WORK/sent"; return "$FAIL_SEND";;
        *) return 1;;
    esac
}
begin_attempt() { echo "begin_attempt $1 $2 $4" >> "$WORK/sequence"; : > "$6"; }
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
                self.assertEqual(sequence.index('begin_attempt stress_delayed_borrow borrow 1'), sequence.index('tx send') - 1)
                self.assertEqual(sequence.index('signing line before send'), sequence.index('tx send') + 1)
                self.assertEqual(Path(directory, 'stress_delayed_borrow.err').read_text(),
                                 f"Signing transaction: {'0'*63}1\nTransaction hash is sent\n")
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

    def test_setup_funds_fixtures_with_three_classic_batches(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_shell(GROUPED + r'''
trustline() { echo trustline >> "$WORK/calls"; }; mint_to() { echo mint_to >> "$WORK/calls"; }
flow_stress_setup
''', directory)
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = Path(directory, 'calls').read_text().splitlines()
            kinds = [w.split('|')[4] for w in Path(directory, 'writes').read_text().splitlines()]
        batches = [c for c in calls if c.startswith('batch ')]
        self.assertEqual(batches, ['batch stress_trust_dave change_trust dave 20',
                                   'batch stress_trust_carol change_trust carol 20',
                                   'batch stress_mint_classic payment admin 40'])
        self.assertNotIn('trustline', calls)
        self.assertNotIn('mint_to', calls)
        self.assertEqual(kinds[:4], ['batch', 'batch', 'batch', 'deploy'])
        self.assertNotIn('batch', kinds[3:])
        trust = [f'trust:ST{i:02d}:GADMIN' for i in range(20)]
        self.assertEqual(calls[1:21], trust)
        self.assertEqual(calls[22:42], trust)
        self.assertEqual(calls[43:83], [f'pay:{who}:ST{i:02d}:GADMIN:10000000000000' for i in range(20) for who in ('GDAVE', 'GCAROL')])

    def test_grouped_fixture_and_crash_writes_use_channels_and_keep_new_keys_serial(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_shell(GROUPED + r'''
cksum < "$STATE_ENV" > "$WORK/cksum0"
flow_stress_setup || exit 1
flow_stress_dualify || exit 2
flow_stress_borrow_frontier() { :; }
flow_stress_composed || exit 3
POSITION_NFT=nft; inv_create() { echo 5; }; sim_probe() { PROBE_STATUS=ok; }; assert_view_eq_at() { :; }
flow_stress_liq_frontier || exit 4
''', directory)
            self.assertEqual(result.returncode, 0, result.stderr)
            work = Path(directory)
            writes = [dict(zip(('phase', 'label', 'job', 'signer', 'fn', 'contract'), w.split('|')))
                      for w in (work / 'writes').read_text().splitlines()]
            state = dict(line.split('=', 1) for line in (work / 'state.env').read_text().splitlines())
            cksums = [line.split('|') for line in (work / 'cksums').read_text().splitlines()]
            initial = (work / 'cksum0').read_text().strip()
            oracles = {i: json.loads((work / f'oracle_arg.ST{i:02d}').read_text()) for i in range(20)}
            fixture_dirs = sorted(int(d.name) for g in work.glob('jobs/stress_fixtures.*') for d in g.iterdir() if d.name.isdigit())

        def cid(k):
            return 'C' + f'{k:055d}'.translate(str.maketrans('0123456789', 'ABCDEFGHIJ'))

        fixture = [w for w in writes if w['phase'] == 'stress_setup' and w['job']]
        jobs = sorted({w['job'] for w in fixture}, key=lambda j: int(j.rsplit('-', 1)[1]))
        self.assertEqual([int(j.rsplit('-', 1)[1]) for j in jobs], list(range(1, 21)))
        self.assertEqual(fixture_dirs, list(range(1, 21)))
        self.assertEqual(len({j.rsplit('-', 1)[0] for j in jobs}), 1)
        for job in jobs:
            n = int(job.rsplit('-', 1)[1])
            code = f'ST{n - 1:02d}'
            rows = [w for w in fixture if w['job'] == job]
            self.assertEqual([w['label'].split('.j')[0] for w in rows],
                             ['deploy_mock', 'deploy_mockrs', f'issue_sac_{code}', f'px_init_{code}', f'rs_px_{code}'], rows)
            self.assertEqual({w['signer'] for w in rows}, {f'c{(n - 1) % 10 + 1}'}, rows)
            self.assertEqual([w['contract'] for w in rows[3:]], [cid(2 * n), cid(2 * n + 1)])
        self.assertEqual({c for job, c in cksums if job in jobs}, {initial})
        for i in range(20):
            self.assertEqual((state[f'STRESS_REF_{i}'], state[f'STRESS_RS_{i}'], state[f'SAC_ST{i:02d}']),
                             (cid(2 * i + 2), cid(2 * i + 3), f'SACST{i:02d}'))
            self.assertEqual(oracles[i]['sources'][0]['Feed']['provider']['Reflector']['contract'], state[f'STRESS_REF_{i}'])
        self.assertFalse([w for w in writes if w['phase'] == 'stress_dualify' and w['fn'] == 'set_price'])
        self.assertEqual(len([w for w in writes if w['phase'] == 'stress_dualify' and w['fn'] == 'set_oracle']), 20)
        new_keys = [w for w in writes if w['label'].startswith(('set_oracle_', 'stress_composed_quote_'))]
        self.assertEqual(len(new_keys), 30)
        self.assertEqual({(w['job'], w['signer'], w['fn']) for w in new_keys}, {('', 'admin', 'set_oracle')})
        crash = [w for w in writes if w['label'].startswith('crash_')]
        self.assertEqual(len(crash), 10)
        for w in crash:
            i = int(w['label'][8:10])
            self.assertEqual((w['job'].rsplit('-', 1)[-1], w['signer']), (str(i + 1), f'c{i + 1}'), w)
            self.assertEqual(w['contract'], state[f'STRESS_REF_{i}' if w['label'].endswith('_p') else f'STRESS_RS_{i}'])

    def test_latest_ledger_rejects_malformed_or_error_responses(self):
        for payload in ('{}', '{"error":{}}', 'garbage', '{"jsonrpc":"2.0","id":1,"result":{"sequence":1.5}}',
                        '{"jsonrpc":"2.0","id":1,"result":{"sequence":42}}'):
            with self.subTest(payload=payload), tempfile.TemporaryDirectory() as directory:
                result = self.run_shell('source "${FLOW%/flows/stress.sh}/lib/invoke.sh"; RPC_URL=rpc; curl() { printf "%s" "$PAYLOAD"; }; latest_ledger',
                                        directory, PAYLOAD=payload)
                if '42' in payload:
                    self.assertEqual((result.returncode, result.stdout), (0, '42\n'), result.stderr)
                else:
                    self.assertNotEqual(result.returncode, 0)


if __name__ == '__main__':
    unittest.main()
