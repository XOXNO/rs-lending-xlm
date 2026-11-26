#!/usr/bin/env python3
"""Offline oracle assertions reject changed prices and timestamps and missing receipts."""
import json
import subprocess
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
with tempfile.TemporaryDirectory() as directory:
    logs = Path(directory)
    (logs / 'confirmed.receipt.json').write_text(json.dumps({'result': {'createdAt': '1700000010'}}))
    aggregate = {'price': '101000000', 'package_timestamp': '1700000000000', 'write_timestamp': '1700000010000'}
    script = '''
source "$1/flows/xoxno_oracle.sh"
LOG_DIR="$2"
extract_signing_hash() { printf '%s' confirmed; }
record() { :; }
_assert_fail() { return 1; }
_xo_assert_aggregate check "$3" 101000000 1700000000000 mutation
'''

    def check(value):
        return subprocess.run(['bash', '-c', script, 'test', str(HERE), directory, json.dumps(value)],
                              capture_output=True, text=True).returncode == 0

    assert check(aggregate)
    assert check({key: int(value) for key, value in aggregate.items()})
    for field in aggregate:
        changed = dict(aggregate)
        changed[field] = str(int(changed[field]) + 1)
        assert not check(changed), f'accepted incorrect {field}'
        changed = dict(aggregate)
        del changed[field]
        assert not check(changed), f'accepted missing {field}'
    assert not check(None)
    (logs / 'confirmed.receipt.json').write_text('{"result":{}}')
    assert not check(aggregate), 'accepted missing ledger close time'

SETTLE = '''
source "$1/flows/xoxno_oracle.sh"
HEADS="$2" COUNTER="$2.n"
echo 0 > "$COUNTER"
sleep() { :; }
_assert_fail() { return 1; }
rpc_post() { local n; n=$(( $(cat "$COUNTER") + 1 )); echo "$n" > "$COUNTER"; sed -n "${n}p" "$HEADS"; }
_xo_history_settle label "$3"
rc=$?
echo "$(cat "$COUNTER")"
exit "$rc"
'''


def head(sequence, close):
    result = {'sequence': sequence}
    if close is not None:
        result['closeTime'] = str(close)
    return json.dumps({'jsonrpc': '2.0', 'id': 1, 'result': result})


def settle(resolution, heads):
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / 'heads'
        path.write_text(''.join(line + '\n' for line in heads))
        done = subprocess.run(['bash', '-c', SETTLE, 'test', str(HERE), str(path), str(resolution)],
                              capture_output=True, text=True)
        return done.returncode, int(done.stdout.split()[-1])


BUCKET = 1700000040
assert BUCKET % 60 == 0
assert settle(1, [head(10, BUCKET), head(10, BUCKET), head(11, BUCKET + 5)]) == (0, 3), 'did not wait for a new ledger'
assert settle(60, [head(10, BUCKET), head(11, BUCKET + 45), head(12, BUCKET + 31), head(13, BUCKET + 30)]) == (0, 4), \
    'accepted a ledger with less than half of the bucket left'
assert settle(60, [head(10, BUCKET + 50), head(11, BUCKET + 65)]) == (0, 2), 'rejected a ledger early in a new bucket'
assert settle(1, [head(10, BUCKET), head(11, None)])[0] != 0, 'accepted a ledger without close time'
assert settle(1, [head(10, BUCKET)] * 41) == (1, 41), 'did not stop when the ledger never advanced'
print('Oracle exact-price/package-time/receipt-time assertions: passed')
print('Oracle history-bucket settle: passed')
