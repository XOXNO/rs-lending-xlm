#!/usr/bin/env python3
"""Funding swaps serialize quote-through-receipt and never mask prerequisites."""
import os
import shutil
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
source "$1/lib/core.sh"; source "$1/lib/invoke.sh"; source "$1/lib/assets.sh"
INTEG_DIR="$2"; LOG_DIR="$2"; RPC_URL=unused; XLM_SAC=X; AGGREGATOR=A
label="$3"; mode="$4"; RUN_TS="$5"
_assert_fail() { echo "$*" >&2; return 1; }
record() { echo "$*" >> "$LOG_DIR/failures"; }
extract_signing_hash() { cat "$1"; }
curl() {
    case "$mode" in
        rpc_transport) return 7;;
        rpc_empty) ;;
        rpc_error) echo '{"jsonrpc":"2.0","id":1,"error":{"code":-1}}';;
        rpc_string) echo '{"jsonrpc":"2.0","id":1,"result":{"sequence":"100"}}';;
        *) printf '{"jsonrpc":"2.0","id":1,"result":{"sequence":%s}}' "$(cat "$LOG_DIR/ledger")";;
    esac
    printf '\n200'
}
agg_route_hex() {
    echo "$label quote $AGGREGATOR_MIN_LEDGER" >> "$LOG_DIR/order"
    touch "$LOG_DIR/$label.quoted"
    echo 00
}
inv() {
    echo "$label submit" >> "$LOG_DIR/order"
    touch "$LOG_DIR/$label.submitted"
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
    def launch(label, mode='ok', lane='lane-a'):
        return subprocess.Popen(['bash', '-c', script, '_', str(HERE), directory, label, mode, lane], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
    def reached(label, stage='quoted'):
        for _ in range(200):
            if (root / f'{label}.{stage}').exists():
                return
            time.sleep(.01)
        raise AssertionError(f'{stage} never reached')
    first = launch('first', 'fail')
    reached('first')
    second = launch('second')
    for proc, code in [(first, 1), (second, 0)]:
        out, err = proc.communicate(timeout=5)
        assert proc.returncode == code, out + err
    assert (root / 'order').read_text().splitlines() == [
        'first quote 100', 'first submit', 'first receipt',
        'second quote 101', 'second submit', 'second receipt'], 'concurrent quote or resubmission'
    cancelled = launch('cancelled', 'cancel')
    reached('cancelled', 'submitted')
    os.killpg(cancelled.pid, signal.SIGTERM)
    cancelled.communicate(timeout=5)
    after = launch('after-cancel')
    out, err = after.communicate(timeout=5)
    assert after.returncode == 1 and 'unresolved' in err, out + err
    assert not (root / 'after-cancel.quoted').exists(), 'quote raced potentially pending submission'
    assert (root / 'runs/.external-funding.lane-a.pending.json').exists()
    other = launch('other-lane', lane='lane-b')
    out, err = other.communicate(timeout=5)
    assert other.returncode == 0 and (root / 'other-lane.quoted').exists(), out + err
    assert not (root / 'runs/.external-funding.lane-b.pending.json').exists()
    again = launch('again')
    out, err = again.communicate(timeout=5)
    assert again.returncode == 1 and 'earlier funding submission unresolved' in err, out + err
    for mode in ('rpc_transport', 'rpc_empty', 'rpc_error', 'rpc_string'):
        failed_rpc = root / mode
        (failed_rpc / 'runs').mkdir(parents=True)
        result = subprocess.run(['bash', '-c', script, '_', str(HERE), str(failed_rpc), 'rpc', mode, 'lane-a'], capture_output=True, text=True, timeout=5)
        assert result.returncode == 1 and 'funding ledger' in result.stderr, result.stderr
        assert not (failed_rpc / 'rpc.quoted').exists()
        assert not (failed_rpc / 'runs/.external-funding.lane-a.pending.json').exists()

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

FRIENDBOT = r"""set -uo pipefail
source "$1/lib/core.sh"; source "$1/lib/wallet.sh"
INTEG_DIR="$2"; RUN_DIR="$2"; LOG_DIR="$2/logs"; ACTIONS_TSV="$2/actions.tsv"; PHASE=test; RUN_TS=t; NET_ARGS=(--rpc-url x)
mkdir -p "$LOG_DIR"; printf 'header\n' > "$ACTIONS_TSV"
backoff_sleep() { :; }
date() {
    [ "$1" = +%s ] || { command date "$@"; return; }
    local now; now=$(( $(cat "$RUN_DIR/clock" 2>/dev/null || echo 1000) + 10 )); echo "$now" > "$RUN_DIR/clock"; echo "$now"
}
stellar() {
    case "$1 $2" in
        'keys address') [ -f "$RUN_DIR/key.$3" ] && echo "G$3" || return 1;;
        'keys generate') touch "$RUN_DIR/key.$3";;
    esac
}
curl() {
    local url n code; for url; do :; done
    case "$url" in
        *friendbot*)
            echo "start ${url##*=}" >> "$RUN_DIR/span"
            n=$(( $(cat "$RUN_DIR/${url##*=}.fb" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$RUN_DIR/${url##*=}.fb"
            code=$(sed -n "${n}p" "$RUN_DIR/codes"); code="${code:-429}"
            [ -z "${SPAN:-}" ] || sleep "$SPAN"
            echo "end ${url##*=}" >> "$RUN_DIR/span"
            printf '%s' "$code";;
        *horizon*)
            n=$(( $(cat "$RUN_DIR/${url##*/}.hz" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$RUN_DIR/${url##*/}.hz"
            [ "$n" -gt "${HZ429:-0}" ] || { printf 429; return 22; }
            [ "$n" -ge "$FUNDED_AT" ] || { printf 404; return 22; }
            echo '{"balances":[{"asset_type":"native","balance":"10000.0"}]}' > "$6"; printf 200;;
    esac
}
"""

def friendbot(directory, body, codes, funded_at, **env):
    (Path(directory) / 'codes').write_text(''.join(c + '\n' for c in codes))
    return subprocess.run(['bash', '-c', FRIENDBOT + body, '_', str(HERE), directory], capture_output=True, text=True, timeout=30,
                          env=dict(os.environ, FUNDED_AT=str(funded_at), **env))

for codes, funded_at, succeeds in [(['429', '429', '200'], 3, True), (['403'], 99, False)]:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        result = friendbot(directory, 'friendbot_fund alice', codes, funded_at)
        assert (result.returncode == 0) == succeeds, result.stderr
        assert (root / 'logs/friendbot_alice.codes').read_text().split() == codes
        assert (root / 'Galice.hz').read_text().strip() == str(len(codes))
for code in ['200', '400']:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        result = friendbot(directory, 'friendbot_fund alice', [code], 3)
        assert result.returncode == 0, (code, result.stderr)
        assert (root / 'logs/friendbot_alice.codes').read_text().split() == [code]
        assert (root / 'Galice.hz').read_text().strip() == '3'
with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    result = friendbot(directory, 'friendbot_fund alice', ['200'], 999)
    assert result.returncode == 1 and (root / 'logs/friendbot_alice.codes').read_text().split() == ['200'], result.stderr
    assert int((root / 'Galice.hz').read_text()) > 1
with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    result = friendbot(directory, 'new_wallet ALICE alice', [], 999)
    recorded = (root / 'logs/friendbot_e2e_alice_t.codes').read_text().split()
    assert result.returncode == 1 and len(recorded) >= 5 and set(recorded) == {'429'}, result.stderr
    assert 'wallet_alice\tFAIL' in (root / 'actions.tsv').read_text()
with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    result = friendbot(directory, 'friendbot_fund alice & a=$!; friendbot_fund bob & b=$!; wait "$a" && wait "$b"', ['200'], 1,
                       E2E_FRIENDBOT_SLOTS='1', SPAN='0.3')
    span = [line.split() for line in (root / 'span').read_text().splitlines()]
    assert result.returncode == 0 and len(span) == 4, (result.stderr, span)
    assert [s[0] for s in span] == ['start', 'end', 'start', 'end'] and span[0][1] == span[1][1] != span[2][1] == span[3][1], span
with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    result = friendbot(directory, 'exec 30>>"$2/held"; new_wallet ALICE alice', ['200'], 1)
    assert result.returncode == 1 and not (root / 'span').exists(), result.stderr
    assert (root / 'logs/friendbot_e2e_alice_t.codes').read_text().split() == ['slot-unavailable']
    assert 'wallet_alice\tFAIL\tfatal' in (root / 'actions.tsv').read_text() and 'no friendbot slot' in (root / 'actions.tsv').read_text()
with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    result = friendbot(directory, '''_assert_fail() { printf '%s\\t%s\\n' "$1" "$2" >> "$RUN_DIR/fails"; }
exec 30>>"$2/held"; lane_channels 1''', ['200'], 999)
    assert result.returncode == 1 and not (root / 'span').exists(), result.stderr
    assert (root / 'fails').read_text() == 'lane_channel_1\tno friendbot slot free within 300 s\n', result.stderr
for throttled, funded, checks in [('2', True, 3), ('99', False, 7)]:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        result = friendbot(directory, 'sleep() { echo "$1" >> "$RUN_DIR/slept"; }\nwallet_funded Galice "$RUN_DIR/alice.json"', [], 1, HZ429=throttled)
        slept = [int(s) for s in (root / 'slept').read_text().split()]
        assert (result.returncode == 0) == funded and (root / 'Galice.hz').read_text().strip() == str(checks), result.stderr
        assert len(slept) == checks - 1 and all(min(8 << i, 60) <= s <= 3 * min(8 << i, 60) // 2 for i, s in enumerate(slept)), slept
with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    result = friendbot(directory, 'sleep() { echo "$1" >> "$RUN_DIR/slept"; echo $(( $(cat "$RUN_DIR/clock" 2>/dev/null || echo 1000) + $1 )) > "$RUN_DIR/clock"; }\nfriendbot_fund alice', ['200'], 1, HZ429='99')
    slept = [int(s) for s in (root / 'slept').read_text().split()]
    assert result.returncode == 1 and slept and sum(slept) <= 90, (result.stderr, slept)
    assert (root / 'logs/friendbot_alice.codes').read_text().split() == ['200'] and int((root / 'Galice.hz').read_text()) > len(slept), result.stderr
for slots, valid in [('0', False), ('abc', False), ('6', True)]:
    result = subprocess.run(['bash', '-c', 'source "$1/env.sh"', '_', str(HERE)], capture_output=True, text=True, timeout=30,
                            env=dict(os.environ, RUN_TS='t', NETWORK='testnet', E2E_FRIENDBOT_SLOTS=slots))
    assert (result.returncode == 0) == valid and (valid or 'invalid E2E_FRIENDBOT_SLOTS' in result.stderr), (slots, result.stderr)
for slots, valid in [('0', False), ('100', False), ('12', True)]:
    result = subprocess.run(['bash', '-c', 'source "$1/env.sh" && source "$1/env.sh" && printf "%s\\n%s\\n%s\\n" "$PATH" "$E2E_STELLAR" "$E2E_RPC_SLOTS"', '_', str(HERE)],
                            capture_output=True, text=True, timeout=30, env=dict(os.environ, RUN_TS='t', NETWORK='testnet', E2E_RPC_SLOTS=slots))
    assert (result.returncode == 0) == valid and (valid or 'invalid E2E_RPC_SLOTS' in result.stderr), (slots, result.stderr)
    if valid:
        path, stellar, exported = result.stdout.splitlines()
        assert path.split(':').count(str(HERE / 'bin')) == 1 and path.split(':')[0] == str(HERE / 'bin'), path
        assert stellar == (shutil.which('stellar') or '') and exported == '12', (stellar, exported)
for name, value, valid in [('E2E_RPC_READ_SLOTS', '0', False), ('E2E_RPC_READ_SLOTS', '21', False), ('E2E_RPC_READ_SLOTS', '20', True),
                           ('THROTTLE_RETRIES', 'x', False), ('THROTTLE_RETRIES', '21', False), ('THROTTLE_RETRIES', '0', True),
                           ('E2E_SLOT_DIR', 'relative/slots', False), ('E2E_SLOT_DIR', '/tmp/shared-slots', True)]:
    result = subprocess.run(['bash', '-c', 'source "$1/env.sh" && printf "%s|%s|%s|%s\\n" "$E2E_RPC_READ_SLOTS" "$THROTTLE_RETRIES" "$E2E_SLOT_DIR" "$E2E_RPC_WAIT_LOG"', '_', str(HERE)],
                            capture_output=True, text=True, timeout=30, env={**os.environ, 'RUN_TS': 't', 'NETWORK': 'testnet', name: value})
    assert (result.returncode == 0) == valid and (valid or f'invalid {name}' in result.stderr), (name, value, result.stderr)
    if valid:
        exported = dict(zip(['E2E_RPC_READ_SLOTS', 'THROTTLE_RETRIES', 'E2E_SLOT_DIR'], result.stdout.strip().split('|')))
        assert exported[name] == value and result.stdout.strip().endswith(f'/runs/t/rpc-wait.tsv'), result.stdout
result = subprocess.run(['bash', '-c', 'source "$1/env.sh" && printf "%s|%s|%s" "$E2E_RPC_READ_SLOTS" "$THROTTLE_RETRIES" "$E2E_SLOT_DIR"', '_', str(HERE)],
                        capture_output=True, text=True, timeout=30,
                        env={k: v for k, v in {**os.environ, 'RUN_TS': 't', 'NETWORK': 'testnet'}.items() if k not in ('E2E_RPC_READ_SLOTS', 'THROTTLE_RETRIES', 'E2E_SLOT_DIR')})
assert result.returncode == 0 and result.stdout == f'6|6|/tmp/rs-lending-e2e-slots-{os.getuid()}', result.stdout
print('Friendbot retries throttling, logs every code, fails closed at the deadline, caps concurrency and refuses an inherited slot fd')
print('Horizon funding reads back off on 429 within a bound and inside the friendbot deadline; env.sh validates and exports the RPC knobs and routes stellar through one shim')
