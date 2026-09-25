#!/usr/bin/env python3
"""Funding swaps serialize quote-through-receipt and never mask prerequisites."""
import os
import signal
import subprocess
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent


def run(script, *args):
    result = subprocess.run(['bash', '-c', 'set -uo pipefail\n' + script, '_', *map(str, args)], capture_output=True, text=True, timeout=10)
    assert result.returncode == 0, result.stdout + result.stderr


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    (root / 'runs').mkdir()
    # Real file lock, separate processes; a second quote cannot race a submitted
    # operation. The mock receipt advances the ledger seen by the next holder.
    script = r'''set -uo pipefail
source "$1/lib/assets.sh"
INTEG_DIR="$2"; LOG_DIR="$2"; RPC_URL=unused; XLM_SAC=X; AGGREGATOR=A
label="$3"; mode="$4"
_assert_fail() { echo "$*" >&2; return 1; }
record() { echo "$*" >> "$LOG_DIR/failures"; }
extract_signing_hash() { cat "$1"; }
curl() {
    case "$mode" in
        rpc_transport) return 7;;
        rpc_empty) return 0;;
        rpc_error) echo '{"jsonrpc":"2.0","id":1,"error":{"code":-1}}'; return;;
        rpc_string) echo '{"jsonrpc":"2.0","id":1,"result":{"sequence":"100"}}'; return;;
    esac
    printf '{"jsonrpc":"2.0","id":1,"result":{"sequence":%s}}' "$(cat "$LOG_DIR/ledger")"
}
agg_route_hex() {
    echo "$label quote $AGGREGATOR_MIN_LEDGER" >> "$LOG_DIR/order"
    touch "$LOG_DIR/$label.quoted"
    echo 00
}
inv() {
    echo "$label submit" >> "$LOG_DIR/order"
    if [ "$mode" = cancel ]; then sleep 60; fi
    sleep .2
    echo 101 > "$LOG_DIR/ledger"
    echo "$label receipt" >> "$LOG_DIR/order"
    if [ "$mode" = fail ]; then
        printf '%064d' 1 > "$LOG_DIR/$label.err"
        printf '{"jsonrpc":"2.0","id":1,"result":{"txHash":"%064d","status":"FAILED"}}' 1 > "$LOG_DIR/$(printf '%064d' 1).receipt.json"
    fi
    [ "$mode" != fail ]
}
swap_xlm_to wallet addr token 1 "$label"
'''
    (root / 'ledger').write_text('100')
    def launch(label, mode='ok'):
        return subprocess.Popen(['bash', '-c', script, '_', str(HERE), directory, label, mode], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
    def quoted(label):
        for _ in range(200):
            if (root / f'{label}.quoted').exists():
                return
            time.sleep(.01)
        raise AssertionError('quote never reached')
    first = launch('first', 'fail')
    quoted('first')
    second = launch('second')
    for proc, code in [(first, 1), (second, 0)]:
        out, err = proc.communicate(timeout=5)
        assert proc.returncode == code, out + err
    assert (root / 'order').read_text().splitlines() == [
        'first quote 100', 'first submit', 'first receipt',
        'second quote 101', 'second submit', 'second receipt'], 'concurrent quote or resubmission'
    cancelled = launch('cancelled', 'cancel')
    quoted('cancelled')
    os.killpg(cancelled.pid, signal.SIGTERM)
    cancelled.communicate(timeout=5)
    after = launch('after-cancel')
    out, err = after.communicate(timeout=5)
    assert after.returncode == 1 and 'unresolved' in err, out + err
    assert not (root / 'after-cancel.quoted').exists(), 'quote raced potentially pending submission'
    assert (root / 'runs/.external-funding.pending.json').exists()
    for mode in ('rpc_transport', 'rpc_empty', 'rpc_error', 'rpc_string'):
        failed_rpc = root / mode
        (failed_rpc / 'runs').mkdir(parents=True)
        result = subprocess.run(['bash', '-c', script, '_', str(HERE), str(failed_rpc), 'rpc', mode], capture_output=True, text=True, timeout=5)
        assert result.returncode == 1 and 'funding ledger' in result.stderr, result.stderr
        assert not (failed_rpc / 'rpc.quoted').exists()
        assert not (failed_rpc / 'runs/.external-funding.pending.json').exists()

    # Actual quote helper: save stale attempts, then accept fresh; never accept
    # malformed or permanently stale snapshots, and keep route encoding intact.
    for mode in ('fresh', 'stale', 'malformed', 'multihop'):
        logs = root / mode
        logs.mkdir()
        run(r'''
source "$1/lib/aggregator.sh"
LOG_DIR="$2"; mode="$3"; AGGREGATOR_API=unused; AGGREGATOR_MIN_LEDGER=101
_assert_fail() { echo "$*" >> "$LOG_DIR/failures"; return 1; }
sleep() { :; }
curl() {
    local n=0 ledger=100 hops='[{}]'
    [ ! -f "$LOG_DIR/count" ] || n=$(cat "$LOG_DIR/count")
    n=$((n+1)); echo "$n" > "$LOG_DIR/count"
    [ "$mode" != fresh ] || [ "$n" -lt 2 ] || ledger=101
    [ "$mode" != malformed ] || ledger='"invalid"'
    if [ "$mode" = multihop ]; then ledger=101; hops='[{},{}]'; fi
    printf '{"snapshot":{"ledger":%s},"hops":%s,"routeXdr":"AQID"}' "$ledger" "$hops"
}
if value=$(agg_route_hex X Y 1); then
    [ "$mode" = fresh ] || [ "$mode" = multihop ] || exit 1
    [ "$value" = 010203 ] || exit 1
else
    [ "$mode" = stale ] || [ "$mode" = malformed ] || exit 1
    [ -s "$LOG_DIR/failures" ] || exit 1
fi
''', HERE, logs, mode)
        expected = {'fresh': 2, 'stale': 12, 'malformed': 1, 'multihop': 4}[mode]
        assert len(list(logs.glob('quote_*'))) == expected

    # Every prerequisite failure stops immediately, without marking wallets funded.
    steps = ['line_USDC', 'trust_admin', 'trust_alice', 'trust_bob', 'trust_carol',
             'fund_swap_usdc', 'balance', 'fund_alice_usdc', 'fund_bob_usdc',
             'fund_carol_usdc', 'line_EURC', 'trust_eurc', 'fund_alice_eurc']
    for index, fail in enumerate(steps):
        (root / 'steps').write_text('')
        run(r'''
source "$1/flows/lifecycle.sh"
ROOT="$2"; FAIL="$3"; ADMIN=admin; ALICE=alice; BOB=bob; CAROL=carol
ADMIN_ADDR=G1; ALICE_ADDR=G2; BOB_ADDR=G3; CAROL_ADDR=G4; USDC_SAC=USDC; EURC_SAC=EURC
phase() { :; }; log() { :; }; _uint_ge() { return 0; }
step() { echo "$1" >> "$ROOT/steps"; [ "$1" != "$FAIL" ]; }
classic_line() { step "line_$1" || return 1; echo "$1:issuer"; }
trustline() { if [ "$2" = EURC ]; then step trust_eurc; else step "trust_$1"; fi; }
swap_xlm_to() { step "$5"; }
balance() { step balance || return 1; echo 400; }
sac_transfer() { step "$6"; }
save_state() { touch "$ROOT/incorrectly-funded"; }
if flow_fund_usdc; then exit 1; fi
[ ! -e "$ROOT/incorrectly-funded" ]
''', HERE, root, fail)
        assert (root / 'steps').read_text().splitlines() == steps[:index + 1]
print('Funding lock, cancellation, quote freshness, no-resubmit and prerequisite regressions passed')
