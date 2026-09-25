#!/usr/bin/env python3
"""Offline attempt retention and interrupted-run summary regressions."""
import csv
import json
import os
import subprocess
import tempfile
from pathlib import Path

import gate
from artifacts import CONTRACTS, digest

HERE = Path(__file__).resolve().parent


def shell(body, interrupted=False):
    with tempfile.TemporaryDirectory() as directory:
        run = Path(directory)
        setup = f'''
set -uo pipefail
source "{HERE}/lib/core.sh"
source "{HERE}/lib/invoke.sh"
RUN_DIR={directory}; LOG_DIR="$RUN_DIR/logs"; ACTIONS_TSV="$RUN_DIR/actions.tsv"
PHASE=test; ADMIN=admin; RPC_URL=unused; NET_ARGS=(--network testnet)
XDG_CONFIG_HOME="$RUN_DIR/private"
mkdir -p "$LOG_DIR" "$XDG_CONFIG_HOME/stellar/identity"
printf 'mock key' > "$XDG_CONFIG_HOME/stellar/identity/admin.toml"
printf 'seq\\tphase\\tlabel\\tstatus\\tfn\\thash\\tinstructions\\tread_bytes\\twrite_bytes\\tresource_fee\\tnote\\n' > "$ACTIONS_TSV"
backoff_sleep() {{ :; }}
sleep() {{ :; }}
'''
        result = subprocess.run(['bash', '-c', setup+body], text=True, capture_output=True)
        assert result.returncode == 0, result.stdout+result.stderr
        attempts_path = run/'attempts.jsonl'
        attempts = [json.loads(line) for line in attempts_path.read_text().splitlines()] if attempts_path.exists() else []
        assert [a['id'] for a in attempts] == list(range(1, len(attempts)+1))
        assert len({a['stdout'] for a in attempts}) == len(attempts)
        for a in attempts:
            assert (run/a['stdout']).is_file() and (run/a['stderr']).is_file()
            assert a['observed_at'] and a['phase'] == 'test'
            assert a['started_at'] <= a['observed_at']
        assert (run/'active-attempt.json').exists() == interrupted
        outputs = [(run/a['stdout']).read_text() for a in attempts]
        actions = list(csv.DictReader((run/'actions.tsv').open(), delimiter='\t'))
        assert all(len(a) == 11 for a in actions), 'actions.tsv interface changed'
        return attempts, outputs, actions


attempts, outputs, actions = shell('''
n=0
stellar() { n=$((n+1)); echo "$n"; [ "$n" -ge 3 ]; }
view repeated contract -- balance >/dev/null || exit 1
view repeated contract -- balance >/dev/null || exit 1
''')
assert [a['cli_exit'] for a in attempts] == [1, 1, 0, 0]
assert [a['attempt'] for a in attempts] == [1, 2, 3, 1]
assert outputs == ['1\n', '2\n', '3\n', '4\n']

attempts, outputs, actions = shell('''
n=0
stellar() {
    n=$((n+1)); echo "$n"
    if [ "$n" = 1 ]; then echo 'Contract not found' >&2; return 1; fi
    echo "Signing transaction: $(printf '%064d' 1)" >&2
}
tx_status() { echo SUCCESS; }
fetch_resources() { RES_INSTR=1 RES_READ=0 RES_WRITE=0 RES_FEE=1; }
inv mutation admin contract -- supply >/dev/null || exit 1
''')
assert [a['cli_exit'] for a in attempts] == [1, 0]
assert attempts[0]['hash'] is None and attempts[1]['hash'] == '0'*63+'1'
assert [a['status'] for a in actions] == ['retry', 'ok']

for message in ['timeout', 'Trapped', 'ResourceLimitExceeded', 'Error(Contract, #24)']:
    attempts, _, actions = shell(f'''
n=0
stellar() {{ n=$((n+1)); echo "Signing transaction: $(printf '%064d' 1)" >&2; echo '{message}' >&2; return 1; }}
tx_status() {{ echo UNKNOWN; }}
if inv mutation admin contract -- supply; then exit 1; fi
[ "$n" = 1 ] || exit 1
''')
    assert len(attempts) == 1 and actions[0]['status'] == 'FAIL'

attempts, _, actions = shell('''
stellar() { echo "Signing transaction: $(printf '%064d' 1)" >&2; exit 130; }
(inv interrupted admin contract -- supply) && exit 1
[ -f "$RUN_DIR/active-attempt.json" ] && grep -q 'Signing transaction:' "$LOG_DIR/interrupted.err"
''', interrupted=True)
assert not attempts and not actions

attempts, outputs, actions = shell('''
n=0
cli_upload() {
    n=$((n+1))
    if [ "$n" = 1 ]; then echo 'Wasm does not exist' >&2; return 1; fi
    printf '%064d' "$n"
}
verify_deployed_wasm() { return 0; }
run_deploy "$LOG_DIR/upload.out" "$LOG_DIR/upload.err" -- cli_upload || exit 1
run_deploy "$LOG_DIR/upload.out" "$LOG_DIR/upload.err" -- cli_upload || exit 1
''')
assert [a['attempt'] for a in attempts] == [1, 2, 1]
assert len(set(a['action_seq'] for a in attempts)) == 1  # No action between uploads.
assert outputs == ['', '0'*63+'2', '0'*63+'3']

attempts, outputs, actions = shell('''
n=0
stellar() {
    n=$((n+1)); echo "$n"
    if [ "$n" = 1 ]; then echo 'Contract not found' >&2; else echo 'Error(Contract, #24)' >&2; fi
    return 1
}
xfail rejected 'Error\\(Contract, #24\\)' admin contract -- borrow || exit 1
''')
assert [a['cli_exit'] for a in attempts] == [1, 1]
assert outputs == ['1\n', '2\n'] and actions[-1]['status'] == 'xfail'

with tempfile.TemporaryDirectory() as directory:
    run = Path(directory)
    (run/'logs').mkdir()
    metadata = dict(lane='agg', selected_cases=['finished', 'interrupted'], run_id='audit',
        started_at='2026-09-25T00:00:00+00:00', workflow_run_id='123', workflow_run_attempt='2')
    (run/'metadata.json').write_text(json.dumps(metadata))
    (run/'cases.tsv').write_text('id\tstatus\tfirst_action\tlast_action\nfinished\tpass\t1\t1\n')
    (run/'actions.tsv').write_text('\t'.join(gate.ACTION_FIELDS)+'\n1\ttest\tok\tok\tassert\t\t\t\t\t\t\n')
    (run/'attempts.jsonl').write_text(json.dumps(dict(id=1, hash='a'*64, receipt='logs/tx.receipt.json'))+'\n')
    gate.write_summary(run, 'running')
    summary = json.loads((run/'summary.json').read_text())
    assert summary['attempts'][0]['receipt_status'] == 'UNKNOWN'
    assert summary['workflow_run_attempt'] == '2'
    (run/'logs/tx.receipt.json').write_text(json.dumps({'result': {'status': 'FAILED'}}))
    gate.mark_incomplete(run, 'cancelled')
    gate.write_summary(run, 'completed', exit_code=130)
    summary = json.loads((run/'summary.json').read_text())
    assert summary['status'] == 'incomplete' and summary['reason'] == 'cancelled'
    assert summary['cases'] == {'pass': 1, 'incomplete': 1}
    assert summary['attempts'][0]['receipt_status'] == 'FAILED'
    assert summary['exit_code'] == 130 and summary['started_at'] == metadata['started_at']
    assert json.loads((run/'interruption.json').read_text())['observed_at']
    (run/'active-attempt.json').write_text(json.dumps(dict(label='pending', stderr='logs/pending.err')))
    (run/'logs/pending.err').write_text('Signing transaction: '+'b'*64+'\n')
    gate.write_summary(run, 'completed', exit_code=130)
    active = json.loads((run/'summary.json').read_text())['active_attempt']
    assert active['hash'] == 'b'*64 and active['receipt_status'] == 'UNKNOWN'
    # Even a cancellation after the last case cannot become green later.
    try:
        gate.validate(run)
    except ValueError as error:
        assert 'interrupted' in str(error)
    else:
        raise AssertionError('interrupted run accepted')

# Resume retains the original start time and refuses another CI run identity.
with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    wasm, binary = root/'wasm', root/'bin'
    wasm.mkdir(); binary.mkdir()
    (binary/'stellar').write_text('#!/bin/sh\nprintf "stellar 28.0.0\\n"\n')
    (binary/'stellar').chmod(0o755)
    for name in CONTRACTS:
        (wasm/(name+'.wasm')).write_bytes(b'fixture')
    (wasm/'candidate.json').write_text(json.dumps(dict(
        source_sha=subprocess.check_output(['git','rev-parse','HEAD'],cwd=HERE,text=True).strip(),
        artifacts={name+'.wasm':digest(wasm/(name+'.wasm')) for name in CONTRACTS})))
    (root/'limits.json').write_text('{}')
    (root/'networks.json').write_text('{}')
    script = f'''
set -uo pipefail
source "{HERE}/lib/core.sh"
INTEG_DIR="{HERE}"; RUN_DIR="{root}/run"; LOG_DIR="$RUN_DIR/logs"
STATE_ENV="$RUN_DIR/state.env"; ACTIONS_TSV="$RUN_DIR/actions.tsv"
RUN_TS=identity-test; E2E_LANE=agg; WASM_DIR="{wasm}"
NETWORKS_FILE="{root}/networks.json"; E2E_LIMITS_FILE="{root}/limits.json"
RPC_URL=https://unused; NETWORK_PASSPHRASE='Test SDF Network ; September 2015'
init_run
cp "$RUN_DIR/metadata.json" "{root}/first.json"
E2E_RESUME=1
init_run
cmp "$RUN_DIR/metadata.json" "{root}/first.json" || exit 1
printf '{{}}' > "$RUN_DIR/active-attempt.json"
if (init_run); then exit 1; fi
rm "$RUN_DIR/active-attempt.json"
export GITHUB_RUN_ATTEMPT=3
if (init_run); then exit 1; fi
'''
    result = subprocess.run(['bash','-c',script],cwd=HERE.parents[1],text=True,capture_output=True,
        env=dict(os.environ,PATH=str(binary)+os.pathsep+os.environ['PATH'],GITHUB_RUN_ID='123',GITHUB_RUN_ATTEMPT='2'))
    assert result.returncode == 0, result.stdout+result.stderr
    metadata = json.loads((root/'first.json').read_text())
    assert metadata['run_id'] == 'identity-test' and metadata['started_at']
    assert metadata['workflow_run_id'] == '123' and metadata['workflow_run_attempt'] == '2'

print('Attempt retention, strict retries, and cancellation summary regressions passed')

# Lost CLI response: committed SUCCESS recovers exactly once from native proof,
# while raw rc/output remain immutable in attempts.jsonl.
receipt=json.loads((HERE/'fixtures/receipt.json').read_text())
receipt_hash=receipt['result']['txHash']
contract='CDDVDGITCUZTHQSYFRV37AC3263SMK2FZYYVJ74EBLESTFFF5UOVODZW'
for malformed in [False,True]:
    body=f'''
INTEG_DIR="{HERE}"; NETWORK_PASSPHRASE='Test SDF Network ; September 2015'
cp "{HERE}/fixtures/receipt.json" "$LOG_DIR/{receipt_hash}.receipt.json"
'''
    if malformed:
        body+='''jq '.result.resultMetaXdr="AA=="' "$LOG_DIR/'''+receipt_hash+'''.receipt.json" > "$LOG_DIR/bad.json"
mv "$LOG_DIR/bad.json" "$LOG_DIR/'''+receipt_hash+'''.receipt.json"
'''
    body+=f'''
n=0
stellar() {{ n=$((n+1)); echo 'Signing transaction: {receipt_hash}' >&2; echo SendRequest >&2; return 1; }}
tx_status() {{ echo SUCCESS; }}
fetch_resources() {{ RES_INSTR=1 RES_READ=0 RES_WRITE=0 RES_FEE=1; }}
rc=0; inv recovered admin {contract} -- deploy_controller > "$LOG_DIR/result" || rc=$?
[ "$n" = 1 ] && [ "$rc" {'!=' if malformed else '='} 0 ] || exit 1
'''
    if not malformed:
        body+='''grep -q CCTE6RN6NK4KWLXL22UBL5DGF3XZ4LT5XBLN4AVOKIKLWVF6D7U7UQFB "$LOG_DIR/result" || exit 1
[ -s "$LOG_DIR/recovered.recovered.json" ] || exit 1
'''
    attempts, outputs, actions=shell(body)
    assert len(attempts)==1 and attempts[0]['cli_exit']==1 and outputs==['']
    assert actions[0]['status']==('FAIL' if malformed else 'ok')

for status in ['FAILED','UNKNOWN']:
    attempts,outputs,actions=shell(f'''
n=0
stellar() {{ n=$((n+1)); echo 'Signing transaction: {receipt_hash}' >&2; echo ResourceLimitExceeded >&2; return 1; }}
tx_status() {{ echo {status}; }}
recover_output() {{ echo forbidden >&2; exit 88; }}
if inv failed admin {contract} -- deploy_controller; then exit 1; fi
[ "$n" = 1 ] || exit 1
''')
    assert len(attempts)==1 and actions[0]['status']=='FAIL'

# run_deploy applies the same terminal-success reconciliation, then still checks
# candidate bytes. Native deployment/return binding is covered in test_receipts.
attempts,outputs,actions=shell(f'''
n=0
cli_deploy() {{ n=$((n+1)); echo 'Signing transaction: {receipt_hash}' >&2; echo SendRequest >&2; return 1; }}
tx_status() {{ echo SUCCESS; }}
fetch_resources() {{ :; }}
recover_output() {{ [ "$3" = command ] && [ "$4" = cli_deploy ] || return 1; echo '"CCTE6RN6NK4KWLXL22UBL5DGF3XZ4LT5XBLN4AVOKIKLWVF6D7U7UQFB"' > "$2"; }}
verify_deployed_wasm() {{ [ -s "$1" ]; }}
run_deploy "$LOG_DIR/deploy.out" "$LOG_DIR/deploy.err" -- cli_deploy || exit 1
[ "$n" = 1 ] && [ -s "$LOG_DIR/deploy.out" ] || exit 1
''')
assert len(attempts)==1 and attempts[0]['cli_exit']==1 and outputs==['']
print('Confirmed-success recovery preserves failed CLI evidence and never resubmits')

# Only read-only bytecode fetch retries transport failure. A successful read with
# wrong bytes remains a distinct mismatch; no retry can conceal it.
for mismatch in [False,True]:
    attempts,outputs,actions=shell(f'''
INTEG_DIR="{HERE}"
id=CAFVLWR3CPMH4CYZDJOKNK2FIJ54RR5PSADCQEBNFFDB3SBASL6EBQHD
printf '%s' "$id" > "$LOG_DIR/id.out"
printf candidate > "$LOG_DIR/candidate.wasm"
n=0
stellar() {{
    n=$((n+1))
    if [ "$n" = 1 ]; then echo 'client error: Connect' >&2; return 1; fi
    local last=''; for last in "$@"; do :; done
    printf {'wrong' if mismatch else 'candidate'} > "$last"
}}
rc=0; verify_deployed_wasm "$LOG_DIR/id.out" --wasm "$LOG_DIR/candidate.wasm" || rc=$?
[ "$n" = 2 ] && [ "$rc" {'!=' if mismatch else '='} 0 ] || exit 1
[ -s "$LOG_DIR/$id.fetch1.err" ] && [ -f "$LOG_DIR/$id.fetch2.out" ] || exit 1
''')
    assert any(a['label']=='deployed_hash' and a['status']=='FAIL' for a in actions)==mismatch
print('Read-only fetch retry preserves distinct transport and bytecode mismatch evidence')

for status in ['FAILED','UNKNOWN']:
    attempts,outputs,actions=shell(f'''
n=0
cli_deploy() {{ n=$((n+1)); echo 'Signing transaction: {receipt_hash}' >&2; echo ResourceLimitExceeded >&2; return 1; }}
tx_status() {{ echo {status}; }}
recover_output() {{ echo forbidden >&2; exit 88; }}
if run_deploy "$LOG_DIR/deploy.out" "$LOG_DIR/deploy.err" -- cli_deploy; then exit 1; fi
[ "$n" = 1 ] && [ ! -s "$LOG_DIR/deploy.out" ] || exit 1
''')
    assert len(attempts)==1 and actions[0]['status']=='FAIL'
print('Failed/unknown submitted deployments remain sticky without return recovery')

# Every native upload/deploy receives the declared policy before constructors.
# A matching caller flag is canonicalized; conflicts/duplicates never submit.
import shlex
policy_shapes=[
    ['upload','--wasm','/tmp/pool with spaces.wasm','--source','admin','--network','testnet'],
    ['deploy','--wasm','/tmp/pool.wasm','--source','admin','--rpc-url','https://rpc.invalid','--','--admin','OWNER'],
    ['deploy','--wasm-hash','a'*64,'--source','admin','--','--name','literal words'],
    ['upload','--instruction-leeway','2000000','--wasm','/tmp/pool.wasm'],
    ['deploy','--instruction-leeway=2000000','--wasm','/tmp/pool.wasm','--','--instruction-leeway','constructor field'],
]
for argv in policy_shapes:
    command=shlex.join(['stellar','contract',*argv])
    expected=['contract',argv[0],'--instruction-leeway','2000000']
    remaining=argv[1:]
    if remaining[:1]==['--instruction-leeway']: remaining=remaining[2:]
    if remaining[:1]==['--instruction-leeway=2000000']: remaining=remaining[1:]
    expected+=remaining
    attempts,outputs,actions=shell(f'''
INSTRUCTION_LEEWAY=2000000
stellar() {{ printf '%s\\n' "$@" > "$LOG_DIR/command-args"; printf '%064d' 1; }}
verify_deployed_wasm() {{ :; }}
run_deploy "$LOG_DIR/policy.out" "$LOG_DIR/policy.err" -- {command} || exit 1
python3 - "$LOG_DIR/command-args" {shlex.quote(json.dumps(expected))} <<'PYPOLICY'
import json,sys
from pathlib import Path
assert Path(sys.argv[1]).read_text().splitlines()==json.loads(sys.argv[2])
PYPOLICY
''')
    assert len(attempts)==1 and attempts[0]['cli_exit']==0
for options in [
    ['--instruction-leeway','1000000'],['--instruction-leeway=1000000'],
    ['--instruction-leeway','2000000','--instruction-leeway=2000000'],
    ['--instructions','2000000'],['--instructions=2000000'],['--instruction-leeway'],
]:
    command=shlex.join(['stellar','contract','upload','--wasm','/tmp/pool.wasm',*options])
    attempts,outputs,actions=shell(f'''
INSTRUCTION_LEEWAY=2000000
n=0; stellar() {{ n=$((n+1)); return 0; }}
if run_deploy "$LOG_DIR/policy.out" "$LOG_DIR/policy.err" -- {command}; then exit 1; fi
[ "$n" = 0 ] || exit 1
''')
    assert not attempts and actions[0]['label']=='deployment_policy' and actions[0]['status']=='FAIL'
print('Native deployment policy reaches uploads and constructors without conflicting flags')
