#!/usr/bin/env python3
"""Offline oracle assertions reject changed prices/timestamps and missing receipts."""
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
print('Oracle exact-price/package-time/receipt-time assertions: passed')
