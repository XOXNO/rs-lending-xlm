#!/usr/bin/env python3
"""Offline attempt retention and interrupted-run summary regressions."""
import csv
import json
import os
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

import gate
from artifacts import CONTRACTS, digest

HERE = Path(__file__).resolve().parent


def shell(body, interrupted=False, bash='bash'):
    with tempfile.TemporaryDirectory() as directory:
        run = Path(directory)
        setup = f'''
set -uo pipefail
source "{HERE}/lib/core.sh"
source "{HERE}/lib/invoke.sh"
RUN_DIR={directory}; LOG_DIR="$RUN_DIR/logs"; ACTIONS_TSV="$RUN_DIR/actions.tsv"
PHASE=test; ADMIN=admin; RPC_URL=unused; NET_ARGS=(--network testnet); NETWORK_PASSPHRASE='Test SDF Network ; September 2015'; INTEG_DIR="{HERE}"
XDG_CONFIG_HOME="$RUN_DIR/private"; E2E_SLOT_DIR="$RUN_DIR/slots"
mkdir -p "$LOG_DIR" "$XDG_CONFIG_HOME/stellar/identity"
printf 'mock key' > "$XDG_CONFIG_HOME/stellar/identity/admin.toml"
printf 'seq\\tphase\\tlabel\\tstatus\\tfn\\thash\\tinstructions\\tread_bytes\\twrite_bytes\\tresource_fee\\tnote\\n' > "$ACTIONS_TSV"
backoff_sleep() {{ :; }}
sleep() {{ :; }}
'''
        result = subprocess.run([bash, '-c', setup+body], text=True, capture_output=True)
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

DRIFT_SETUP = '''
n=0
stellar() { n=$((n+1)); echo "$n"; echo "Signing transaction: $(printf '%064d' "$n")" >&2; }
fetch_resources() { RES_INSTR=1 RES_READ=0 RES_WRITE=0 RES_FEE=1; }
NETWORK_PASSPHRASE=testnet
'''
attempts, _, actions = shell(DRIFT_SETUP + '''
tx_status() { [ "$1" = "$(printf '%064d' 1)" ] && echo FAILED || echo SUCCESS; }
receipt_drift() { echo "$#" >> "$RUN_DIR/drift"; }
inv mutation admin contract -- borrow >/dev/null || exit 1
[ "$(cat "$RUN_DIR/drift" | tr '\n' ' ')" = "5 7 " ] || exit 1
grep -q $'^1\trejected_transaction\tcontract$' "$RUN_DIR/evidence.tsv"
''')
assert [a['status'] for a in actions] == ['retry', 'ok']
assert [a['hash'] for a in actions] == ['0'*63+'1', '0'*63+'2'] and len(attempts) == 2
print('A verified FAILED footprint-limit receipt is retried once and proven by the pair check')

for tx_status, drift, calls, statuses in [
    ('echo FAILED', 'return 1', 1, ['FAIL']),
    ('echo FAILED', ':', 2, ['retry', 'FAIL']),
    ('[ "$1" = "$(printf \'%064d\' 1)" ] && echo FAILED || echo SUCCESS', '[ "$#" -eq 5 ]', 2, ['retry', 'FAIL']),
    ('echo UNKNOWN', ':', 1, ['FAIL']),
]:
    attempts, _, actions = shell(DRIFT_SETUP + f'''
tx_status() {{ {tx_status}; }}
receipt_drift() {{ {drift}; }}
if inv mutation admin contract -- borrow >/dev/null; then exit 1; fi
[ "$n" = {calls} ]
''')
    assert [a['status'] for a in actions] == statuses, (tx_status, drift, actions)
attempts, _, actions = shell(DRIFT_SETUP + '''
tx_status() { echo FAILED; }
receipt_drift() { :; }
if INV_MAX_ATTEMPTS=1 inv mutation admin contract -- borrow >/dev/null; then exit 1; fi
[ "$n" = 1 ]
''')
assert [a['status'] for a in actions] == ['FAIL']
attempts, _, actions = shell(DRIFT_SETUP + '''
stellar() {
    n=$((n+1)); echo "$n"
    if [ "$n" = 1 ]; then echo "Signing transaction: $(printf '%064d' 1)" >&2; else echo 'Contract not found' >&2; return 1; fi
}
tx_status() { echo FAILED; }
receipt_drift() { :; }
if inv mutation admin contract -- borrow >/dev/null; then exit 1; fi
[ "$n" = 2 ]
''')
assert [a['status'] for a in actions] == ['retry', 'FAIL']
attempts, _, actions = shell(DRIFT_SETUP + '''
stellar() { n=$((n+1)); echo "Signing transaction: $(printf '%064d' 1)" >&2; echo 'Error(Contract, #102)' >&2; return 1; }
tx_status() { echo FAILED; }
receipt_drift() { echo 'Traceback: ValueError: failure is not a storage footprint limit' >&2; return 1; }
if inv mutation admin contract -- borrow >/dev/null; then exit 1; fi
grep -q 'not a storage footprint limit' "$LOG_DIR/$(printf '%064d' 1).drift.err"
''')
assert [a['status'] for a in actions] == ['FAIL'] and 'Error(Contract, #102)' in actions[0]['note'] and 'Traceback' not in actions[0]['note']
print('Non-footprint failures, a second failure, an unchanged footprint and UNKNOWN stay final')

SDK_SETUP = '''
source "{HERE}/flows/sdk.sh"
export RUN_DIR
ALICE=alice CONTROLLER=controller NETWORK_PASSPHRASE=testnet INTEG_DIR="{HERE}"
stellar() { echo secret; }
cat > "$RUN_DIR/node" <<'NODE'
#!/bin/bash
cat >/dev/null
n=$(( $(cat "$RUN_DIR/node.count" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$RUN_DIR/node.count"
printf '%064d' "$n" > "$4.hash"
[ "$n" -gt "${NODE_FAILS:-1}" ] || { echo 'unconfirmed/failed transaction' >&2; exit 1; }
printf '{"hash":"%064d","value":7}' "$n"
NODE
chmod +x "$RUN_DIR/node"; NODE_BIN="$RUN_DIR/node"
'''.replace('{HERE}', str(HERE))
attempts, _, actions = shell(SDK_SETUP + '''
receipt_drift() { echo "$#" >> "$RUN_DIR/drift"; }
[ "$(sdk_inv sdk_borrow buildStellarBorrowTx '{}')" = 7 ] || exit 1
[ "$(cat "$RUN_DIR/drift" | tr '\n' ' ')" = "5 7 " ] && [ -f "$LOG_DIR/sdk_borrow.hash" ] && [ -f "$LOG_DIR/sdk_borrow_retry.hash" ]
''')
assert [(a['status'], a['fn'], a['hash']) for a in actions] == [('retry', 'borrow', '0'*63+'1'), ('ok', 'borrow', '0'*63+'2')]
attempts, _, actions = shell(SDK_SETUP + '''
receipt_drift() { return 1; }
if sdk_inv sdk_borrow buildStellarBorrowTx '{}' >/dev/null; then exit 1; fi
[ "$(cat "$RUN_DIR/node.count")" = 1 ]
''')
assert [a['status'] for a in actions] == ['FAIL']
for fails, drift, expect_error, calls, statuses in [
    (2, ':', '', 2, ['retry', 'FAIL']),
    (1, '[ "$#" -eq 5 ] || { echo "ValueError: retry is not the same call" >&2; return 1; }', '', 2, ['retry', 'FAIL']),
    (1, ':', 'AmountMustBePositive', 1, ['FAIL']),
]:
    attempts, _, actions = shell(SDK_SETUP + f'''
export NODE_FAILS={fails}
receipt_drift() {{ {drift}; }}
if EXPECT_ERROR={expect_error} sdk_inv sdk_borrow buildStellarBorrowTx '{{}}' >/dev/null; then exit 1; fi
[ "$(cat "$RUN_DIR/node.count")" = {calls} ]
''')
    assert [a['status'] for a in actions] == statuses, (fails, drift, actions)
    assert ('not the same call' in actions[-1]['note']) == ('same call' in drift), actions[-1]['note']
print('The published SDK path retries a verified footprint-limit failure once, as a fresh process')

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
    metadata = dict(lane='agg-core', selected_cases=['finished', 'interrupted'], run_id='audit',
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
RUN_TS=identity-test; E2E_LANE=agg-core; WASM_DIR="{wasm}"
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

for status, error in [('FAILED','ResourceLimitExceeded'),('UNKNOWN','ResourceLimitExceeded'),('UNKNOWN','TxInsufficientFee')]:
    attempts,outputs,actions=shell(f'''
n=0
cli_deploy() {{ n=$((n+1)); echo 'Signing transaction: {receipt_hash}' >&2; echo {error} >&2; return 1; }}
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
    ['upload','--instruction-leeway','20000000','--wasm','/tmp/pool.wasm'],
    ['deploy','--instruction-leeway=20000000','--wasm','/tmp/pool.wasm','--','--instruction-leeway','constructor field'],
]
for argv in policy_shapes:
    command=shlex.join(['stellar','contract',*argv])
    expected=['contract',argv[0],'--filter-logs','stellar_cli::assembled=trace','--instruction-leeway','20000000']
    remaining=argv[1:]
    if remaining[:1]==['--instruction-leeway']: remaining=remaining[2:]
    if remaining[:1]==['--instruction-leeway=20000000']: remaining=remaining[1:]
    expected+=remaining
    attempts,outputs,actions=shell(f'''
INSTRUCTION_LEEWAY=20000000
stellar() {{
    if [ "$1 $2" = 'fees stats' ]; then echo unavailable >&2; return 1; fi
    printf '%s\\n' "$@" > "$LOG_DIR/command-args"; printf '%064d' 1;
}}
verify_deployed_wasm() {{ :; }}
run_deploy "$LOG_DIR/policy.out" "$LOG_DIR/policy.err" -- {command} || exit 1
[ -f "$LOG_DIR/deployment-fee-stats.json" ] && grep -q unavailable "$LOG_DIR/deployment-fee-stats.err" || exit 1
python3 - "$LOG_DIR/command-args" {shlex.quote(json.dumps(expected))} <<'PYPOLICY'
import json,sys
from pathlib import Path
assert Path(sys.argv[1]).read_text().splitlines()==json.loads(sys.argv[2])
PYPOLICY
''')
    assert len(attempts)==1 and attempts[0]['cli_exit']==0
for options in [
    ['--instruction-leeway','1000000'],['--instruction-leeway=1000000'],
    ['--instruction-leeway','20000000','--instruction-leeway=20000000'],
    ['--instructions','20000000'],['--instructions=20000000'],['--instruction-leeway'],
]:
    command=shlex.join(['stellar','contract','upload','--wasm','/tmp/pool.wasm',*options])
    attempts,outputs,actions=shell(f'''
INSTRUCTION_LEEWAY=20000000
n=0; stellar() {{ [ "$1 $2" = 'fees stats' ] && return 0; n=$((n+1)); return 0; }}
if run_deploy "$LOG_DIR/policy.out" "$LOG_DIR/policy.err" -- {command}; then exit 1; fi
[ "$n" = 0 ] || exit 1
''')
    assert not attempts and actions[0]['label']=='deployment_policy' and actions[0]['status']=='FAIL'
print('Native deployment policy reaches uploads and constructors without conflicting flags')

attempts, _, _ = shell('''
fee_reads=0
stellar() {
    if [ "$1 $2" = 'fees stats' ]; then
        fee_reads=$((fee_reads+1)); echo '{"sorobanInclusionFee":{"max":"200"},"latestLedger":123}'; return 0
    fi
    printf '%064d' 1
}
verify_deployed_wasm() { :; }
run_deploy "$LOG_DIR/first.out" "$LOG_DIR/first.err" -- stellar contract upload --wasm unused || exit 1
run_deploy "$LOG_DIR/second.out" "$LOG_DIR/second.err" -- stellar contract upload --wasm unused || exit 1
[ "$fee_reads" = 1 ] && jq -e '.latestLedger == 123' "$LOG_DIR/deployment-fee-stats.json" >/dev/null || exit 1
[ ! -e "$LOG_DIR/deployment-fee-stats.json.tmp" ] || exit 1
''')
assert len(attempts) == 2
print('Deployment fee diagnostics retain one RPC snapshot without changing fee policy')

attempts, _, actions = shell('''
n=0
stellar() {
    n=$((n+1)); echo "$n"
    if [ "$n" = 1 ]; then echo 'error: preflight queue full' >&2; return 1; fi
    echo "Signing transaction: $(printf '%064d' 1)" >&2
}
tx_status() { echo SUCCESS; }
fetch_resources() { RES_INSTR=1 RES_READ=0 RES_WRITE=0 RES_FEE=1; }
inv mutation admin contract -- supply >/dev/null || exit 1
[ "$n" = 2 ]
''')
assert [a['status'] for a in actions] == ['retry', 'ok'] and 'pre-sign transport or overload failure, attempt 1' in actions[0]['note']
assert len(attempts) == 2 and len({a['hash'] for a in attempts if a['hash']}) == 1
hex_429 = 'Error(Auth, InvalidAction) tx/429f209e 20429517'
for call, status, error in [('inv mutation admin contract -- supply', 'UNKNOWN', '429 Too Many Requests'),
                            ("xfail rejected 'Error\\(Contract, #24\\)' admin contract -- borrow", 'FAILED', '429 Too Many Requests'),
                            ('inv mutation admin contract -- supply', 'UNKNOWN', 'Error(Contract, #7) status_code: 503'),
                            ("xfail rejected 'Error\\(Contract, #24\\)' admin contract -- borrow", 'FAILED', 'Error(Contract, #7) status_code: 503'),
                            ('inv mutation admin contract -- supply', 'UNKNOWN', hex_429),
                            ("xfail rejected 'Error\\(Contract, #24\\)' admin contract -- borrow", 'FAILED', hex_429)]:
    signing = '' if 'Error' in error else 'echo "Signing transaction: $(printf \'%064d\' 1)" >&2; '
    attempts, _, actions = shell(f'''
n=0
stellar() {{ n=$((n+1)); {signing}echo '{error}' >&2; return 1; }}
tx_status() {{ echo {status}; }}
if {call} >/dev/null; then exit 1; fi
[ "$n" = 1 ]
''')
    assert len(attempts) == 1 and [a['status'] for a in actions] == ['FAIL'], (call, error, actions)
attempts, _, actions = shell('''
n=0
stellar() {
    n=$((n+1))
    if [ "$n" = 1 ]; then echo 'Transport(Rejected { status_code: 503 })' >&2; else echo 'Error(Contract, #24)' >&2; fi
    return 1
}
xfail rejected 'Error\\(Contract, #24\\)' admin contract -- borrow || exit 1
[ "$n" = 2 ]
''')
assert [a['status'] for a in actions] == ['retry', 'xfail'] and len(attempts) == 2
attempts, outputs, _ = shell('''
n=0
cli_upload() {
    n=$((n+1))
    if [ "$n" = 1 ]; then echo '429 Too Many Requests' >&2; return 1; fi
    printf '%064d' "$n"
}
verify_deployed_wasm() { return 0; }
run_deploy "$LOG_DIR/upload.out" "$LOG_DIR/upload.err" -- cli_upload || exit 1
''')
assert [a['attempt'] for a in attempts] == [1, 2] and outputs == ['', '0'*63+'2']
attempts, _, _ = shell(f'''
n=0
cli_upload() {{ n=$((n+1)); echo '{hex_429}' >&2; return 1; }}
if run_deploy "$LOG_DIR/upload.out" "$LOG_DIR/upload.err" -- cli_upload; then exit 1; fi
[ "$n" = 1 ]
''')
assert len(attempts) == 1
attempts, _, actions = shell(DRIFT_SETUP + '''
stellar() {
    n=$((n+1))
    if [ "$n" = 1 ]; then echo "Signing transaction: $(printf '%064d' 1)" >&2; return 0; fi
    echo 'error: preflight queue full' >&2; return 1
}
tx_status() { echo FAILED; }
receipt_drift() { :; }
if inv mutation admin contract -- borrow >/dev/null; then exit 1; fi
[ "$n" = 2 ]
''')
assert [a['status'] for a in actions] == ['retry', 'FAIL'] and len(attempts) == 2
print('Pre-sign overload retries only while no signed envelope exists')

SAC = 'C' + 'A'*54 + 'B'
for deployed, succeeds in [(SAC, True), ('C' + 'A'*54 + 'C', False)]:
    outcome = f'grep -qx "1\tdeployment\t{SAC}" "$RUN_DIR/evidence.tsv" && [ "$USDC_SAC" = {SAC} ]' if succeeds else '[ -z "${USDC_SAC:-}" ]'
    attempts, _, actions = shell(f'''
source "{HERE}/lib/assets.sh"
STATE_ENV="$RUN_DIR/state.env"; ADMIN_ADDR=GADMIN
n=0
stellar() {{
    case "$1 $2" in
        'contract id') echo {SAC};;
        'contract invoke') [ -f "$RUN_DIR/deployed" ];;
        'contract asset') n=$((n+1)); touch "$RUN_DIR/deployed"; echo "Signing transaction: $(printf '%064d' 1)" >&2; echo '"{deployed}"';;
        *) return 1;;
    esac
}}
tx_status() {{ echo SUCCESS; }}
fetch_resources() {{ RES_INSTR=11 RES_READ=12 RES_WRITE=13 RES_FEE=14; }}
rc=0; issue_sac USDC_SAC USDC || rc=$?
[ "$n" = 1 ] && [ "$rc" {'=' if succeeds else '!='} 0 ] || exit 1
{outcome}
''')
    assert len(attempts) == 1 and attempts[0]['label'] == 'sac_USDC' and attempts[0]['hash'] == '0'*63+'1'
    row = [(a['label'], a['status'], a['fn'], a['hash'], a['instructions'], a['read_bytes'], a['write_bytes'], a['resource_fee']) for a in actions]
    assert row == [('issue_sac_USDC', 'ok' if succeeds else 'FAIL', 'asset_deploy', '0'*63+'1') + (('11', '12', '13', '14') if succeeds else ('',)*4)], row
print('SAC deployments are submitted through run_deploy and recorded as verified deployments')

attempts, _, actions = shell(f'''
source "{HERE}/lib/assert.sh"
source "{HERE}/flows/stress.sh"
DAVE_DUAL_ACCT=7; PRIMARY_HUB_ID=1; PRIMARY_SPOKE_ID=1; STRESS_UNIT=10000000
DAVE=dave; DAVE_ADDR=dave_address; CAROL=carol; CAROL_ADDR=carol_address; CONTROLLER=controller
phase() {{ :; }}
stress_sac() {{ echo "token$1"; }}
hub_key() {{ echo key; }}
pay_vec() {{ echo '[]'; }}
_view_int() {{ case "$1" in *before*) echo 100;; *) echo 10000100;; esac; }}
balance() {{ if [ -f "$RUN_DIR/sent" ]; then echo 10000000000; else echo 0; fi; }}
latest_ledger() {{ local n=10; [ ! -f "$RUN_DIR/ledger" ] || n=$(cat "$RUN_DIR/ledger"); echo $((n+1)) > "$RUN_DIR/ledger"; echo "$n"; }}
inv() {{ :; }}
stellar() {{
    case "$1 $2" in
        'tx hash') printf '%064d\\n' 1;;
        'tx send') touch "$RUN_DIR/sent";;
    esac
}}
tx_status() {{ echo SUCCESS; }}
fetch_resources() {{ RES_INSTR=1; RES_READ=2; RES_WRITE=3; RES_FEE=4; }}
view() {{ echo '[{{"a":1,"b":1,"c":1,"d":1,"e":1}},{{"a":1,"b":1,"c":1,"d":1,"e":1}}]'; }}
flow_stress_delayed || exit 1
''')
assert [a['label'] for a in attempts] == ['stress_delayed_borrow'] and attempts[0]['started_at'] <= attempts[0]['observed_at']
print('The delayed stress submission records when its send started')

attempts, _, actions = shell(f'''
source "{HERE}/flows/production.sh"
phase() {{ :; }}
STATE_ENV="$RUN_DIR/state.env"; CHANNELS="chan1 chan2"
INTEG_DIR="$RUN_DIR/fake"; REPO_ROOT=unused; FIXTURE_WASM_DIR=fixtures; WASM_DIR=wasm; ADMIN_ADDR=GADMIN
mkdir -p "$INTEG_DIR"
cat > "$INTEG_DIR/production_config.py" <<'FAKE'
import json, sys
if sys.argv[1] == 'plan':
    print(json.dumps({{'CREFLECTOR': {{'kind': 'Reflector'}}, 'CTOKEN': {{'kind': 'Token', 'decimals': 7, 'name': 'T'}}}}))
else:
    open(sys.argv[3] + '/fixtures.json', 'w').write(json.dumps(dict(bases={{}}, seeds=[], pools=[])))
FAKE
stellar() {{
    [ "$1 $2" != 'fees stats' ] || return 0
    local a prev='' src=''
    for a; do [ "$prev" != --source ] || src="$a"; prev="$a"; done
    echo "${{E2E_JOB##*-}} $src" >> "$RUN_DIR/sources"
    echo "Signing transaction: $(printf '%064d' "${{E2E_JOB##*-}}")" >&2; echo '"{SAC}"'
}}
tx_status() {{ printf '{{"result":{{"status":"SUCCESS"}}}}' > "$LOG_DIR/$1.receipt.json"; echo SUCCESS; }}
fetch_resources() {{ printf '{{"resources":{{"instructions":%d,"disk_read_bytes":2,"write_bytes":3}},"resource_fee":4}}\\n' "$((10#$1))" > "$LOG_DIR/$1.resources.json"; }}
verify_deployed_wasm() {{ :; }}
deploy_protocol() {{ :; }}; prod_deploy_protocol || exit 1
[ "$(sort "$RUN_DIR/sources" | tr '\\n' ' ')" = '1 chan1 2 chan2 ' ] || exit 2
for n in 1 2; do
    [ -f "$LOG_DIR/$(printf '%064d' "$n").resources.json" ] && [ -s "$LOG_DIR/fixture_$n.err" ] || exit 3
done
[ "$(cut -f2 "$RUN_DIR/evidence.tsv" | tail -n +2 | tr '\\n' ' ')" = 'deployment deployment ' ] || exit 4
python3 - "$RUN_DIR" <<'CHECK'
import csv, sys
from pathlib import Path
sys.path.insert(0, '{HERE}')
import gate
run = Path(sys.argv[1])
gate.check_attempts(run, list(csv.DictReader((run/'actions.tsv').open(newline=''), delimiter='\\t')))
CHECK
''')
assert [(a['label'], a['hash'], a['instructions'], a['resource_fee']) for a in actions] == [
    (f'production_fixture_{n}', f'{n:064d}', str(n), '4') for n in (1, 2)], actions
assert [(a['label'], a['hash']) for a in attempts] == [(f'fixture_{n}', f'{n:064d}') for n in (1, 2)], attempts
print('Grouped production fixture deployments carry their channel source, committed hash and sidecar resources')

GROUP_SETUP = r'''
STATE_ENV="$RUN_DIR/state.env"; CHANNELS="chan1 chan2"
hash_of() { python3 -c 'import hashlib, sys; print(hashlib.sha256(sys.argv[1].encode()).hexdigest())' "$1"; }
stellar() {
    printf '%s %s\n' "${E2E_JOB:-parent}" "$*" >> "$RUN_DIR/calls"
    local k
    k=$(grep -c "^${E2E_JOB:-parent} contract invoke" "$RUN_DIR/calls")
    echo '"ok"'
    echo "Signing transaction: $(hash_of "${E2E_JOB:-parent}$k")" >&2
}
status_of() { echo SUCCESS; }
tx_status() { local st; st=$(status_of "$1"); printf '{"result":{"status":"%s"}}' "$st" > "$LOG_DIR/$1.receipt.json"; echo "$st"; }
fetch_resources() { RES_INSTR=1 RES_READ=0 RES_WRITE=0 RES_FEE=1; }
latest_ledger() { echo 1; }
case_files() { printf '{"selected_cases":["c"]}' > "$RUN_DIR/metadata.json"; printf 'id\tstatus\tfirst_action\tlast_action\n' > "$RUN_DIR/cases.tsv"; }
gate_attempts() {
python3 - "$RUN_DIR" "{HERE}" <<'PYGATE'
import csv, sys
from pathlib import Path
sys.path.insert(0, sys.argv[2])
import gate
run = Path(sys.argv[1])
actions = list(csv.DictReader((run/'actions.tsv').open(newline=''), delimiter='\t'))
evidence = list(csv.DictReader((run/'evidence.tsv').open(newline=''), delimiter='\t'))
assert [a['seq'] for a in actions] == [e['seq'] for e in evidence] == [str(n) for n in range(1, len(actions)+1)]
gate.check_attempts(run, actions)
PYGATE
}
'''.replace('{HERE}', str(HERE))

# G1: jobs sharing a label replay in spawn order with contiguous ids and mapped action_seq.
attempts, _, actions = shell(GROUP_SETUP + r'''
jobf() { inv lbl admin c -- supply >/dev/null; }
group_begin g 2 || exit 1
group_spawn jobf; group_spawn jobf; group_spawn jobf
group_end || exit 2
gid=${GROUP_LAST##*.}
for n in 1 2 3; do
    [ "$(awk -F'\t' -v n="$n" 'NR==n+1 {print $6}' "$ACTIONS_TSV")" = "$(hash_of "$gid-${n}1")" ] || exit 3
done
[ -s "$LOG_DIR/lbl.out" ] && [ ! -e "$RUN_DIR/active-attempt.json" ] || exit 4
gate_attempts || exit 5
''')
assert [(a['seq'], a['status']) for a in actions] == [('1', 'ok'), ('2', 'ok'), ('3', 'ok')], actions
assert len({a['hash'] for a in actions}) == 3 and [a['id'] for a in attempts] == [1, 2, 3]
assert [a['action_seq'] for a in attempts] == [next(int(r['seq']) for r in actions if r['hash'] == a['hash']) for a in attempts]

# G2: a footprint drift retry inside a job keeps its receipt pair and the next job's row.
attempts, _, actions = shell(GROUP_SETUP + r'''
receipt_drift() { printf '%s %s\n' "$2" "${7:-none}" >> "$RUN_DIR/drift"; }
status_of() { if [ "$1" = "$(hash_of "${E2E_JOB%-*}-11")" ]; then echo FAILED; else echo SUCCESS; fi; }
jobf() { inv lbl admin c -- borrow >/dev/null; }
group_begin g 2 || exit 1
group_spawn jobf; group_spawn jobf
group_end || exit 2
gid=${GROUP_LAST##*.}
h1=$(hash_of "$gid-11"); h1b=$(hash_of "$gid-12"); h2=$(hash_of "$gid-21")
[ "$(cat "$RUN_DIR/drift")" = "$h1 none
$h1 $h1b" ] || exit 3
[ "$(awk -F'\t' 'NR>1 {print $4, $6}' "$ACTIONS_TSV")" = "retry $h1
ok $h1b
ok $h2" ] || exit 4
gate_attempts || exit 5
''')
assert [a['status'] for a in actions] == ['retry', 'ok', 'ok'] and [a['action_seq'] for a in attempts] == [1, 1, 3]
print('Group jobs replay in spawn order with contiguous attempt ids, mapped action_seq and the drift pair')

# G3: a job killed after signing keeps the group marker, fails the group, and the summary names its hash.
attempts, _, actions = shell(GROUP_SETUP + r'''
case_files
stellar() {
    echo "Signing transaction: $(hash_of "$E2E_JOB")" >&2
    [ "${E2E_JOB##*-}" != 1 ] || exit 130
    echo '"ok"'
}
jobf() { inv lbl admin c -- supply >/dev/null; }
group_begin x 2 || exit 1
group_spawn jobf; group_spawn jobf
if group_end; then exit 2; fi
[ -f "$RUN_DIR/active-attempt.json" ] && [ -f "$RUN_DIR/jobs/x.$GROUP_ID/1/active.json" ] && [ "$GROUP_LAST" = "$RUN_DIR/jobs/x.$GROUP_ID" ] || exit 3
python3 "$INTEG_DIR/gate.py" summary "$RUN_DIR" completed 1 || exit 4
python3 - "$RUN_DIR" "$INTEG_DIR" "$(hash_of "$GROUP_ID-1")" "$(hash_of "$GROUP_ID-2")" <<'PY' || exit 5
import json, sys
from pathlib import Path
sys.path.insert(0, sys.argv[2])
import gate
run = Path(sys.argv[1])
summary = json.loads((run/'summary.json').read_text())
active = summary['active_attempt']
assert active['group'].startswith('jobs/x.') and sys.argv[3] in active['hashes'], active
assert [(a['label'], a['hash'], a['receipt_status']) for a in summary['group_attempts']] == [('lbl', sys.argv[4], 'SUCCESS')], summary['group_attempts']
try:
    gate.validate(run)
except ValueError as error:
    assert 'interrupted' in str(error)
else:
    raise AssertionError('interrupted group accepted')
PY
''', interrupted=True)
assert [(a['label'], a['status']) for a in actions] == [('group_x_job_1', 'FAIL'), ('lbl', 'ok'), ('group_x_unfinished', 'FAIL')], actions
assert actions[0]['note'] == 'job exited 130' and actions[2]['note'].startswith('an attempt did not finish in job(s) 1;'), actions
print('A job interrupted after signing keeps the marker; the summary lists its hash and group attempts')

# G4: serial-only helpers refuse inside a job before any CLI or RPC call.
attempts, _, actions = shell(GROUP_SETUP + r'''
source "{HERE}/lib/assets.sh"; source "{HERE}/lib/protocol.sh"; source "{HERE}/flows/sdk.sh"; source "{HERE}/flows/stress.sh"
source "{HERE}/flows/production.sh"
curl() { echo curl >> "$RUN_DIR/calls"; return 7; }
REPO_ROOT="$RUN_DIR/repo"; mkdir -p "$REPO_ROOT/configs"; printf 'echo script.sh >> %q\n' "$RUN_DIR/calls" > "$REPO_ROOT/configs/script.sh"
jobf() {
    prod_ops validateConfigs
    xfail r1 'x' admin c -- borrow
    xfail_sim r2 'x' admin c -- borrow
    sim_probe r3 admin c -- supply
    sdk_inv r4 buildStellarSupplyTx '{}'
    run_case r5 true
    create_market r6 1 sac 7 '{}' '{}'
    classic_batch r7 change_trust admin "trust:USDC:GISSUER"
    swap_xlm_to admin GADDR sac 1 r8
    flow_stress_delayed
    group_begin r9 1 reads
    group_spawn true
    group_end
    return 0
}
group_begin g 1 || exit 1
group_spawn jobf
if group_end; then exit 2; fi
[ ! -e "$RUN_DIR/calls" ] || exit 3
'''.replace('{HERE}', str(HERE)))
assert [(a['label'], a['status'], a['fn']) for a in actions] == [('operator_validateConfigs', 'FAIL', 'prod_ops'),
    ('r1', 'FAIL', 'xfail'), ('r2', 'FAIL', 'xfail'), ('r3', 'FAIL', 'sim_probe'), ('r4', 'FAIL', 'sdk_inv'),
    ('r5', 'FAIL', 'run_case'), ('create_market_r6', 'FAIL', 'create_market'), ('r7', 'FAIL', 'classic_batch'),
    ('r8', 'FAIL', 'swap_xlm_to'), ('stress_delayed_borrow', 'FAIL', 'flow_stress_delayed'), ('group_r9', 'FAIL', 'group_begin'),
    ('group_spawn_true', 'FAIL', 'group_spawn'), ('group_end', 'FAIL', 'group_end')], actions
assert not attempts
print('Serial-only helpers refuse in a job with one FAIL row each and no CLI call')

# G5: job state reaches state.env and the parent only through the replay.
shell(GROUP_SETUP + r'''
jobf() { save_state K 'v 1'; [ "$K" = 'v 1' ]; }
group_begin g 1 reads || exit 1
group_spawn jobf
[ ! -e "$STATE_ENV" ] && [ -z "${K:-}" ] || exit 2
group_end || exit 3
[ "$K" = 'v 1' ] && [ "$(grep '^K=' "$STATE_ENV")" = 'K=v\ 1' ] || exit 4
''')

# G6/G7: one pending transaction per source and the runner-wide simulation slots.
for mode, extra, call in [('', '', 'inv lbl admin c -- supply'), (' reads', 'INTEG_DIR="$RUN_DIR"; E2E_SIM_SLOTS=1', 'view lbl c -- balance')]:
    shell(GROUP_SETUP + extra + r'''
stellar() {
    echo "start $E2E_JOB" >> "$RUN_DIR/span"; command sleep 0.3; echo "end $E2E_JOB" >> "$RUN_DIR/span"
    echo '"1"'; echo "Signing transaction: $(hash_of "$E2E_JOB")" >&2
}
jobf() { CALL >/dev/null; }
group_begin g 2MODE || exit 1
group_spawn jobf; group_spawn jobf
group_end || exit 2
[ "$(awk '{print $1}' "$RUN_DIR/span" | tr '\n' ' ')" = 'start end start end ' ] || exit 3
[ "$(sed -n 1p "$RUN_DIR/span" | cut -d' ' -f2)" = "$(sed -n 2p "$RUN_DIR/span" | cut -d' ' -f2)" ] || exit 4
'''.replace('CALL', call).replace('MODE', mode))
print('Jobs never overlap on one source, and E2E_SIM_SLOTS bounds concurrent simulations')

# G8: jobs spawned inside a while-read loop cannot consume the loop input.
shell(GROUP_SETUP + r'''
jobf() { cat >/dev/null; echo "$1" >> "$RUN_DIR/seen"; }
printf 'a\nb\nc\n' > "$RUN_DIR/lines"
group_begin g 1 reads || exit 1
while IFS= read -r x; do group_spawn jobf "$x"; done < "$RUN_DIR/lines"
group_end || exit 2
[ "$(tr '\n' ' ' < "$RUN_DIR/seen")" = 'a b c ' ] || exit 3
''')

# G9: the case range covers every replayed row.
attempts, _, actions = shell(GROUP_SETUP + r'''
case_files
record before ok assert
jobf() { record inside ok assert; }
casef() { record pre ok assert; group_begin g 2 reads || return 1; group_spawn jobf; group_spawn jobf; group_end || return 1; record post ok assert; }
run_case c casef || exit 1
tail -1 "$RUN_DIR/cases.tsv" | awk -F'\t' '{exit !($1=="c" && $2=="pass" && $3+0==2 && $4+0==6)}' || exit 2
''')
assert [a['label'] for a in actions] == ['before', 'pre', 'g_ledger_floor', 'inside', 'inside', 'post']

# G10: a deployment in a job keeps its attempt, action row and deployed-artifact line.
CID = 'C' + 'A'*54 + 'B'
attempts, _, actions = shell(GROUP_SETUP + r'''
W="$RUN_DIR/c.wasm"; printf wasm > "$W"; CID=CID_VALUE
stellar() {
    printf '%s %s\n' "${E2E_JOB:-parent}" "$*" >> "$RUN_DIR/calls"
    case "$1 $2" in
        'fees stats') echo '{}';;
        'contract deploy') echo "Signing transaction: $(hash_of "$E2E_JOB")" >&2; echo "\"$CID\"";;
        'contract fetch') local last; for last; do :; done; cp "$W" "$last";;
        *) return 1;;
    esac
}
jobf() {
    run_deploy "$LOG_DIR/dep.out" "$LOG_DIR/dep.err" -- stellar contract deploy --wasm "$W" --source admin || return 1
    echo "$DEPLOY_ATTEMPTS" > "$RUN_DIR/job_attempts"
    record dep ok deploy "$(extract_signing_hash "$LOG_DIR/dep.err")" '' '' '' '' "$CID" deployment "$CID"
}
group_begin g 2 || exit 1
group_spawn jobf
group_end || exit 2
grep -qx 'deploy attempts: 1' "$GROUP_LAST/1/stderr" || exit 3
[ "$(cat "$RUN_DIR/job_attempts")" = 1 ] || exit 7
[ "$(jq -r .address "$RUN_DIR/deployed-artifacts.jsonl")" = "$CID" ] && [ -s "$LOG_DIR/deployment-fee-stats.json" ] || exit 4
[ "$(awk -F'\t' 'NR==2 {print $2}' "$RUN_DIR/evidence.tsv")" = deployment ] || exit 5
gate_attempts || exit 6
'''.replace('CID_VALUE', CID))
assert [(a['label'], a['status'], a['fn']) for a in actions] == [('dep', 'ok', 'deploy')] and actions[0]['hash']
assert [(a['label'], a['method'], a['action_seq'], a['hash']) for a in attempts] == [('dep', 'deploy', 1, actions[0]['hash'])]
print('A grouped deployment replays its attempt, action and deployed-artifact line')

# G11: a job that dies under set -u fails the case even when the caller drops group_end's status.
attempts, _, actions = shell(GROUP_SETUP + r'''
case_files
badjob() { echo "$UNSET_VAR"; }
casef() { group_begin g 1 reads; group_spawn badjob; group_end; return 0; }
if run_case c casef; then exit 1; fi
[ "$(tail -1 "$RUN_DIR/cases.tsv" | cut -f1,2)" = "$(printf 'c\tfail')" ] || exit 2
''')
assert ('group_g_job_1', 'FAIL', 'job exited without status') in [(a['label'], a['status'], a['note']) for a in actions], actions

# G12: an exiting lane kills every job tree before the summary.
attempts, _, actions = shell(GROUP_SETUP + r'''
case_files
write_report() { :; }
slowjob() {
    sh -c 'echo $PPID' >> "$RUN_DIR/pids"
    command sleep 10 & echo "$!" >> "$RUN_DIR/pids"; wait "$!"
    inv lbl admin c -- supply >/dev/null
}
(
    trap 'finish_run $?' EXIT
    group_begin g 2 || exit 9
    group_spawn slowjob
    n=0
    until [ -f "$RUN_DIR/pids" ] && [ "$(wc -l < "$RUN_DIR/pids")" -ge 2 ] || [ "$n" -ge 200 ]; do command sleep 0.05; n=$((n+1)); done
    exit 1
)
[ "$(wc -l < "$RUN_DIR/pids")" -ge 2 ] || exit 2
for pid in $(cat "$RUN_DIR/pids"); do
    n=0
    while job_alive "$pid" && [ "$n" -lt 40 ]; do command sleep 0.05; n=$((n+1)); done
    ! job_alive "$pid" || exit 3
done
python3 - "$RUN_DIR" <<'PY' || exit 4
import json, re, sys
from pathlib import Path
run = Path(sys.argv[1])
summary = json.loads((run/'summary.json').read_text())
active = summary['active_attempt']
assert summary['status'] == 'incomplete' and active['group'].startswith('jobs/g.'), summary
signed = {h for f in (run/'logs').glob('*.err') for h in re.findall(r'Signing transaction: ([0-9a-f]{64})', f.read_text())}
assert signed <= set(active['hashes']), (signed, active)
PY
''', interrupted=True)
print('A failed job, a lane exit and an open group all fail closed and leave no live job')

# G13: parent writes while a group is open are refused, and an open group at case return aborts the case.
attempts, _, actions = shell(GROUP_SETUP + r'''
source "{HERE}/flows/sdk.sh"; source "{HERE}/flows/production.sh"
REPO_ROOT="$RUN_DIR/repo"; mkdir -p "$REPO_ROOT/configs"; printf 'echo script.sh >> %q\n' "$RUN_DIR/calls" > "$REPO_ROOT/configs/script.sh"
ALICE=admin CONTROLLER=c NODE_BIN=false
jobf() { command sleep 0.3; }
group_begin g 2 reads || exit 1
group_spawn jobf
if prod_ops validateConfigs; then exit 10; fi
if sdk_inv parent_sdk buildStellarSupplyTx '{}'; then exit 11; fi
if inv parent admin c -- supply; then exit 2; fi
if record parent_row ok assert; then exit 3; fi
if save_state P 1; then exit 4; fi
if group_begin nested 1 reads; then exit 5; fi
if group_end; then exit 6; fi
[ ! -e "$RUN_DIR/calls" ] && [ -z "${P:-}" ] || exit 7
if group_spawn jobf; then exit 8; fi
if group_end; then exit 9; fi
'''.replace('{HERE}', str(HERE)))
assert [(a['label'], a['status']) for a in actions] == [('g_ledger_floor', 'ok'), ('group_g_parent_writes', 'FAIL'), ('group_spawn_jobf', 'FAIL'), ('group_end', 'FAIL')], actions
assert actions[1]['note'].startswith('6 parent writes refused while the group was open: prod_ops validateConfigs;sdk_inv parent_sdk;begin_attempt parent;'), actions
attempts, _, actions = shell(GROUP_SETUP + r'''
f() { command sleep 0.3; }
(group_begin g 2 reads; group_spawn f; die fatal_lbl boom)
[ -f "$RUN_DIR/active-attempt.json" ] || exit 1
''', interrupted=True)
assert [(a['label'], a['status'], a['note']) for a in actions] == [('g_ledger_floor', 'ok', 'latest=1 floor=0'), ('fatal_lbl', 'FAIL', 'boom')], actions
attempts, _, actions = shell(GROUP_SETUP + r'''
jobf() {
    inv bad_source '../x' c -- supply
    run_deploy "$LOG_DIR/dep.out" "$LOG_DIR/dep.err" -- stellar contract deploy --wasm w
    return 0
}
group_begin g 1 || exit 1
group_spawn jobf
if group_end; then exit 2; fi
[ ! -e "$RUN_DIR/calls" ] || exit 3
''')
assert [(a['label'], a['status'], a['note']) for a in actions] == [('bad_source', 'FAIL', 'no source lock'), ('dep', 'FAIL', 'no source lock')], actions
attempts, _, actions = shell(GROUP_SETUP + r'''
case_files
f() { command sleep 0.3; }
casef() { group_begin x 2 reads; group_spawn f; return 0; }
if run_case c casef; then exit 1; fi
[ -f "$RUN_DIR/active-attempt.json" ] && [ "$(tail -1 "$RUN_DIR/cases.tsv" | cut -f1,2)" = "$(printf 'c\tfail')" ] || exit 2
''', interrupted=True)

# G14: %q journaling keeps a replayed row byte-identical to the serial row.
shell(GROUP_SETUP + r'''
note=$(printf "a\037b'c")
jobf() { record lbl ok assert '' '' '' '' '' "$note" transaction CX; }
record lbl ok assert '' '' '' '' '' "$note" transaction CX
group_begin g 1 reads || exit 1
group_spawn jobf
group_end || exit 2
cmp <(sed -n 2p "$ACTIONS_TSV" | cut -f2-) <(sed -n 4p "$ACTIONS_TSV" | cut -f2-) || exit 3
cmp <(sed -n 2p "$RUN_DIR/evidence.tsv" | cut -f2-) <(sed -n 4p "$RUN_DIR/evidence.tsv" | cut -f2-) || exit 4
[ "$(sed -n 4p "$ACTIONS_TSV" | cut -f11)" = "$note" ] || exit 5
''')

# G15: under /bin/bash and set -u, a read group needs no channels and a write group refuses without them.
attempts, _, actions = shell(GROUP_SETUP + r'''
unset CHANNELS
INTEG_DIR="$RUN_DIR"
stellar() { echo '"1"'; }
jobf() { view v c -- balance >/dev/null; }
group_begin r 2 reads || exit 1
group_spawn jobf; group_spawn jobf
group_end || exit 2
if group_begin w 2; then exit 3; fi
CHANNELS=chan1
if group_begin 'bad name' 2; then exit 4; fi
if group_begin z 0; then exit 5; fi
if group_begin z 2 writes; then exit 6; fi
[ ! -e "$RUN_DIR/active-attempt.json" ] && [ ! -e "$RUN_DIR/calls" ] || exit 7
''', bash='/bin/bash')
assert [(a['label'], a['status']) for a in actions] == [('r_ledger_floor', 'ok'), ('v', 'read'), ('v', 'read')] + [(f'group_{n}', 'FAIL') for n in ('w', 'bad name', 'z', 'z')], actions
print('Parent writes, case-level open groups, journal quoting and channel-free reads all hold')

# G16: a spawn from a pipeline or a command substitution is refused before any job starts.
attempts, _, actions = shell(GROUP_SETUP + r"""
case_files
jobf() { inv "lbl_$1" admin c -- supply >/dev/null; }
casef() {
    group_begin g 2 || return 1
    printf 'a\nb\n' | while IFS= read -r x; do group_spawn jobf "$x"; done
    out=$(group_spawn jobf c)
    group_end
    return 0
}
if run_case c casef; then exit 1; fi
[ ! -e "$RUN_DIR/calls" ] && [ -z "$(ls "$RUN_DIR"/jobs/g.*/ | grep -x '[0-9][0-9]*')" ] || exit 2
""", interrupted=True)
assert [(a['label'], a['status'], a['note']) for a in actions] == [
    ('group_g_untracked', 'FAIL', 'group_spawn jobf in a subshell; group_spawn jobf in a subshell; group_spawn jobf in a subshell'),
    ('c', 'FAIL', 'case returned 1')], actions

# G17: a job spawned by a subshell that bypasses the guard, or a job that leaves a live process, keeps the marker.
for spawn, gap in [('( GROUP_SUBSHELL=$BASH_SUBSHELL; group_spawn jobf )', 'job directories [1] differ from the 0 spawned jobs'),
                   ('group_spawn bgjob', 'job 1 left a live process')]:
    attempts, _, actions = shell(GROUP_SETUP + r"""
case_files
jobf() { inv lbl admin c -- supply >/dev/null; }
bgjob() { ( until [ -e "$RUN_DIR/go" ]; do command sleep 0.05; done; inv lbl admin c -- supply >/dev/null ) & return 0; }
casef() { group_begin g 2 || return 1; SPAWN; group_end; touch "$RUN_DIR/go"; return 0; }
if run_case c casef; then exit 1; fi
n=0
until [ -s "$(echo "$RUN_DIR"/jobs/g.*/1/actions.part)" ] || [ "$n" -ge 100 ]; do command sleep 0.05; n=$((n+1)); done
[ -s "$(echo "$RUN_DIR"/jobs/g.*/1/actions.part)" ] || exit 2
""".replace('SPAWN', spawn), interrupted=True)
    assert [(a['label'], a['status']) for a in actions][0] == ('group_g_untracked', 'FAIL') and gap in actions[0]['note'], actions
    assert actions[-1]['label'] == 'c' and not attempts and not any(a['label'] == 'lbl' for a in actions), actions

# G18: a kept group marker refuses every later serial attempt and group until the lane stops.
attempts, _, actions = shell(GROUP_SETUP + r"""
case_files
stellar() {
    printf '%s %s\n' "${E2E_JOB:-parent}" "$*" >> "$RUN_DIR/calls"
    echo "Signing transaction: $(hash_of "${E2E_JOB:-parent}")" >&2
    [ -z "${E2E_JOB:-}" ] || until [ -e "$RUN_DIR/go" ]; do command sleep 0.05; done
    echo '"ok"'
}
jobf() { inv lbl_bg admin c -- supply >/dev/null & return 0; }
casef() {
    group_begin g 2 || return 1
    group_spawn jobf
    n=0; until [ -s "$RUN_DIR/calls" ] || [ "$n" -ge 200 ]; do command sleep 0.05; n=$((n+1)); done
    group_end
    inv serial_after admin c -- supply >/dev/null
    group_begin h 1 reads
    touch "$RUN_DIR/go"
    return 0
}
if run_case c casef; then exit 1; fi
n=0; until [ -s "$(echo "$RUN_DIR"/jobs/g.*/1/attempts.part)" ] || [ "$n" -ge 100 ]; do command sleep 0.05; n=$((n+1)); done
! grep -q '^parent' "$RUN_DIR/calls" && grep -q '"group"' "$RUN_DIR/active-attempt.json" || exit 2
""", interrupted=True)
assert [(a['label'], a['status']) for a in actions] == [('group_g_untracked', 'FAIL'), ('group_g_unfinished', 'FAIL'),
    ('serial_after', 'FAIL'), ('group_h', 'FAIL'), ('c', 'FAIL')], actions
assert actions[2]['note'] == actions[3]['note'] == 'an unfinished group marker is still active' and not attempts, actions
print('Untracked spawns, live job processes and kept group markers fail the case and keep the marker')

# G19: mock-oracle, mock-RedStone and SAC deployments in a job sign with the job's channel and replay as deployments with sidecar resources.
SAC = 'C' + 'A'*54 + 'B'
MOCK_ID = 'C' + 'A'*54 + 'C'
MOCKRS_ID = 'C' + 'A'*54 + 'D'
attempts, _, actions = shell(GROUP_SETUP + r'''
source "{HERE}/lib/oracle.sh"; source "{HERE}/lib/assets.sh"
FIXTURE_WASM_DIR="$RUN_DIR"; printf wasm > "$RUN_DIR/mock_oracle.wasm"; printf wasmrs > "$RUN_DIR/mock_redstone.wasm"; ADMIN_ADDR=GADMIN
stellar() {
    local a prev='' src='' wasm='' id=''
    for a; do [ "$prev" != --source ] || src="$a"; [ "$prev" != --wasm ] || wasm="${a##*/}"; [ "$prev" != --id ] || id="$a"; prev="$a"; done
    case "$1 $2" in
        'fees stats') echo '{}';;
        'contract id') echo SAC_ID;;
        'contract invoke') [ -f "$RUN_DIR/sac_live" ];;
        'contract asset') echo "asset $src" >> "$RUN_DIR/sources"; touch "$RUN_DIR/sac_live"
            echo "Signing transaction: $(hash_of "$E2E_JOB-asset")" >&2; echo '"SAC_ID"';;
        'contract deploy') echo "deploy $wasm $src" >> "$RUN_DIR/sources"
            echo "Signing transaction: $(hash_of "$E2E_JOB-$wasm")" >&2
            if [ "$wasm" = mock_redstone.wasm ]; then echo '"MOCKRS_ID"'; else echo '"MOCK_ID"'; fi;;
        'contract fetch') for a; do :; done
            if [ "$id" = MOCKRS_ID ]; then cp "$RUN_DIR/mock_redstone.wasm" "$a"; else cp "$RUN_DIR/mock_oracle.wasm" "$a"; fi;;
        *) return 1;;
    esac
}
fetch_resources() { printf '{"resources":{"instructions":11,"disk_read_bytes":12,"write_bytes":13},"resource_fee":14}\n' > "$LOG_DIR/$1.resources.json"; }
jobf() { MOCK=''; MOCKRS=''; deploy_mock_reflector || return 1; deploy_mock_redstone || return 1; issue_sac SAC_USDC USDC; }
group_begin g 2 || exit 1
group_spawn jobf
group_end || exit 2
[ "$(tr '\n' ' ' < "$RUN_DIR/sources")" = 'deploy mock_oracle.wasm chan1 deploy mock_redstone.wasm chan1 asset chan1 ' ] || exit 3
[ "$MOCK" = MOCK_ID ] && [ "$MOCKRS" = MOCKRS_ID ] && [ "$SAC_USDC" = SAC_ID ] || exit 3
for h in $(awk -F'\t' 'NR>1 {print $6}' "$ACTIONS_TSV"); do [ -f "$LOG_DIR/$h.resources.json" ] || exit 4; done
[ "$(cut -f2,3 "$RUN_DIR/evidence.tsv" | tail -n +2 | tr '\t\n' ': ')" = 'deployment:MOCK_ID deployment:MOCKRS_ID deployment:SAC_ID ' ] && [ -s "$LOG_DIR/deploy_mock.out" ] && [ -s "$LOG_DIR/deploy_mockrs.out" ] || exit 5
gate_attempts || exit 6
'''.replace('{HERE}', str(HERE)).replace('SAC_ID', SAC).replace('MOCKRS_ID', MOCKRS_ID).replace('MOCK_ID', MOCK_ID))
assert [(a['label'], a['status'], a['fn'], a['instructions'], a['read_bytes'], a['write_bytes'], a['resource_fee']) for a in actions] == [
    ('deploy_mock_reflector', 'ok', 'deploy', '11', '12', '13', '14'), ('deploy_mock_redstone', 'ok', 'deploy', '11', '12', '13', '14'),
    ('issue_sac_USDC', 'ok', 'asset_deploy', '11', '12', '13', '14')], actions
assert [a['label'] for a in attempts] == ['deploy_mock', 'deploy_mockrs', 'sac_USDC'] and len({a['hash'] for a in actions}) == 3 and all(a['hash'] for a in actions)
print('Grouped mock-oracle, mock-RedStone and SAC deployments sign with the job channel and replay as deployments with sidecar resources')

# R1: a read group records its ledger floor, then replays each view in spawn order with contiguous attempts.
attempts, _, actions = shell(GROUP_SETUP + r'''
stellar() {
    local n; n=$(( $(cat "$RUN_DIR/count.$E2E_JOB" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$RUN_DIR/count.$E2E_JOB"
    [ "${E2E_JOB##*-}" != 2 ] || [ "$n" -gt 1 ] || { echo 'transient' >&2; return 1; }
    command sleep "0.$(( 4 - ${E2E_JOB##*-} ))"
    echo "\"${E2E_JOB##*-}\""
}
group_begin g 8 reads || exit 1
for n in 1 2 3; do group_spawn view "v$n" c -- balance; done
group_end || exit 2
gate_attempts || exit 3
''')
assert [(a['label'], a['status'], a['fn']) for a in actions] == [('g_ledger_floor', 'ok', 'assert'), ('v1', 'read', 'balance'), ('v2', 'read', 'balance'), ('v3', 'read', 'balance')], actions
assert actions[0]['note'] == 'latest=1 floor=0', actions
assert [(a['label'], a['attempt'], a['cli_exit'], a['action_seq']) for a in attempts] == [('v1', 1, 0, 2), ('v2', 1, 1, 3), ('v2', 2, 0, 3), ('v3', 1, 0, 4)], attempts

# R2: the floor is the highest committed ledger over CLI and SDK receipts, and the group waits until the RPC reaches it.
R_SETUP = GROUP_SETUP + r'''
source "{HERE}/flows/sdk.sh"
export LOG_DIR
ALICE=alice CONTROLLER=controller
cat > "$RUN_DIR/node" <<'NODE'
#!/bin/bash
cat >/dev/null
h=$(printf '%064d' 7)
printf '%s' "$h" > "$4.hash"
printf '{"jsonrpc":"2.0","id":1,"result":{"status":"SUCCESS","ledger":100}}' > "$LOG_DIR/$h.receipt.json"
printf '{"hash":"%s","value":7}' "$h"
NODE
chmod +x "$RUN_DIR/node"; NODE_BIN="$RUN_DIR/node"
tx_status() { printf '{"jsonrpc":"2.0","id":1,"result":{"status":"SUCCESS","ledger":90}}' > "$LOG_DIR/$1.receipt.json"; echo SUCCESS; }
latest_ledger() {
    local n; n=$(( $(cat "$RUN_DIR/polls" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$RUN_DIR/polls"
    if [ "$n" -eq 1 ]; then echo "$LATEST_FIRST"; else echo "$LATEST_NEXT"; fi
}
printf '{"result":{"status":"SUCCESS","ledger":500}}' > "$LOG_DIR/$(printf '%064d' 9).receipt.1.json"
printf '{"result":{"status":"NOT_FOUND","latestLedger":600}}' > "$LOG_DIR/$(printf '%064d' 8).receipt.json"
readf() { group_begin g 8 reads || return 1; group_spawn view v c -- balance; group_end; }
'''.replace('{HERE}', str(HERE))
attempts, _, actions = shell(R_SETUP + r'''
LATEST_FIRST=99 LATEST_NEXT=100
inv cli admin c -- supply >/dev/null || exit 1
[ "$(sdk_inv sdk_borrow buildStellarBorrowTx '{}')" = 7 ] || exit 2
readf || exit 3
[ "$(cat "$RUN_DIR/polls")" = 2 ] && grep -q ' contract invoke .* balance$' "$RUN_DIR/calls" || exit 4
''')
assert [(a['label'], a['status']) for a in actions] == [('cli', 'ok'), ('sdk_borrow', 'ok'), ('g_ledger_floor', 'ok'), ('v', 'read')], actions
assert actions[2]['note'] == 'latest=100 floor=100', actions

# R3: an RPC that never reaches the floor fails the group before any job or view call.
attempts, _, actions = shell(R_SETUP + r'''
LATEST_FIRST=99 LATEST_NEXT=99
printf '{"jsonrpc":"2.0","id":1,"result":{"status":"FAILED","ledger":100}}' > "$LOG_DIR/$(printf '%064d' 1).receipt.json"
if readf; then exit 1; fi
[ "$(cat "$RUN_DIR/polls")" = 31 ] && [ ! -e "$RUN_DIR/calls" ] && [ ! -e "$RUN_DIR/jobs" ] || exit 2
''')
assert [(a['label'], a['status']) for a in actions] == [('g_ledger_floor', 'FAIL')], actions
assert actions[0]['note'] in ('latest=99 below floor=100 after 0 s', 'latest=99 below floor=100 after 1 s'), actions
assert not attempts

# R5: a slow RPC cannot stretch the floor wait past 30 s, and each poll gets only the time left.
attempts, _, actions = shell(R_SETUP + r'''
printf '{"jsonrpc":"2.0","id":1,"result":{"status":"SUCCESS","ledger":100}}' > "$LOG_DIR/$(printf '%064d' 1).receipt.json"
sleep() { SECONDS=$((SECONDS + 10)); }
latest_ledger() { echo "$1" >> "$RUN_DIR/limits"; echo "$1 $(( ${E2E_RPC_DEADLINE:-0} - $(date +%s) ))" >> "$RUN_DIR/deadlines"; echo 99; }
if readf; then exit 1; fi
[ ! -e "$RUN_DIR/calls" ] && [ ! -e "$RUN_DIR/jobs" ] || exit 2
set -- $(cat "$RUN_DIR/limits")
[ "$#" = 3 ] && [ "$1" -ge 29 ] && [ "$1" -le 30 ] && [ $(($1 - $2)) -ge 10 ] && [ $(($2 - $3)) -ge 10 ] && [ "$3" -ge 1 ] || exit 3
while read -r limit left; do [ "$left" -le "$limit" ] && [ "$left" -ge $((limit - 1)) ] || exit 4; done < "$RUN_DIR/deadlines"
''')
assert [(a['label'], a['status']) for a in actions] == [('g_ledger_floor', 'FAIL')], actions
assert actions[0]['note'] in ('latest=99 below floor=100 after 30 s', 'latest=99 below floor=100 after 31 s'), actions
assert not attempts

# R4: a committed receipt without a ledger fails the floor closed before any poll or view call.
attempts, _, actions = shell(R_SETUP + r'''
LATEST_FIRST=100 LATEST_NEXT=100
printf '{"jsonrpc":"2.0","id":1,"result":{"status":"SUCCESS"}}' > "$LOG_DIR/$(printf '%064d' 1).receipt.json"
if readf; then exit 1; fi
[ ! -e "$RUN_DIR/polls" ] && [ ! -e "$RUN_DIR/calls" ] && [ ! -e "$RUN_DIR/jobs" ] || exit 2
''')
assert [(a['label'], a['status'], a['note']) for a in actions] == [('g_ledger_floor', 'FAIL', 'unreadable receipt ledgers in the lane logs')], actions
print('Read groups wait for the ledger floor of every CLI and SDK receipt, and fail closed before any view')

# O1-O3: owner_submit signs the owner auth entry through stdin, logs the hash before its single send, and takes the return from the receipt.
OWNER_SETUP = r'''
ADMIN_ADDR=GADMIN; E2E_SRC=chan1; export RUN_DIR
latest_ledger() { echo 100; }
built() { grep -c '^contract invoke' "$RUN_DIR/calls"; }
stellar() {
    printf '%s\n' "$*" >> "$RUN_DIR/calls"
    local h input
    case "$1 $2" in
        'contract invoke') echo "BUILT$(built)";;
        'keys secret') echo SECRETX;;
        'tx simulate') input=$(cat)
            case "$* $input" in
                *'--auth-mode enforce'*' AUTH:SIM:BUILT'*) echo "PREPARED:$input";;
                *'--auth-mode'*) return 1;;
                *' BUILT'*) echo "SIM:$input";;
                *) return 1;;
            esac;;
        'tx sign') input=$(cat); [ "${input#PREPARED:AUTH:SIM:BUILT}" != "$input" ] || return 1; echo "SIGNED:$input";;
        'tx hash') cat >/dev/null; printf '%064d\n' "$(built)";;
        'tx send') input=$(cat); h=$(printf '%064d' "$(built)")
            [ "${input#SIGNED:PREPARED:AUTH:SIM:BUILT}" != "$input" ] || return 1
            grep -qx "Signing transaction: $h" "$LOG_DIR/lbl.err" || { echo "send before the signing line of $h" >> "$RUN_DIR/violations"; return 9; }
            echo "$h" >> "$RUN_DIR/sends"; echo '{"status":"SUCCESS"}';;
        *) return 1;;
    esac
}
cat > "$RUN_DIR/node" <<'NODE'
#!/bin/bash
printf '%s\n' "$*" >> "$RUN_DIR/node.argv"
[ "$(cat)" = SECRETX ] && [ "${1##*/}" = sign_auth.mjs ] || exit 3
printf 'AUTH:%s\n' "$(cat "$2")"
NODE
chmod +x "$RUN_DIR/node"; NODE_BIN="$RUN_DIR/node"
fetch_resources() { RES_INSTR=1 RES_READ=0 RES_WRITE=0 RES_FEE=1; }
eval "real_$(declare -f recover_output)"
recover_output() { echo "$*" >> "$RUN_DIR/recover"; real_recover_output "$@"; }
python3() { if [ "${1##*/}" = receipts.py ]; then echo '"77"'; else command python3 "$@"; fi; }
sources_are_chan1() {
    [ "$(grep -oE -- '--(source|sign-with-key) [^ ]+' "$RUN_DIR/calls" | sort -u)" = '--sign-with-key chan1
--source chan1' ]
}
'''
attempts, _, actions = shell(OWNER_SETUP + r'''
tx_status() { echo SUCCESS; }
out=$(inv_owner lbl contract -- create_liquidity_pool --hub_id 1) || exit 1
[ "$out" = '"77"' ] && [ "$(cat "$LOG_DIR/lbl.out")" = "$(cat "$LOG_DIR/lbl.recovered.json")" ] || exit 2
[ "$(cat "$RUN_DIR/recover")" = "$(printf '%064d' 1) $LOG_DIR/lbl.out invoke contract create_liquidity_pool" ] || exit 3
[ ! -e "$RUN_DIR/violations" ] && [ "$(cat "$RUN_DIR/sends")" = "$(printf '%064d' 1)" ] || exit 4
[ "$(cat "$RUN_DIR/node.argv")" = "$INTEG_DIR/sdk/sign_auth.mjs $LOG_DIR/lbl.sim.xdr GADMIN 160 $NETWORK_PASSPHRASE" ] || exit 5
! grep -rq SECRETX "$RUN_DIR/node.argv" "$LOG_DIR" || exit 6
[ "$(grep -c '^tx send' "$RUN_DIR/calls")" = 1 ] && sources_are_chan1 || exit 7
''')
assert [(a['label'], a['status'], a['fn'], a['hash']) for a in actions] == [('lbl', 'ok', 'create_liquidity_pool', '0'*63+'1')], actions
assert [(a['label'], a['cli_exit'], a['hash']) for a in attempts] == [('lbl', 0, '0'*63+'1')], attempts
attempts, _, actions = shell(OWNER_SETUP + r'''
tx_status() { if [ "$1" = "$(printf '%064d' 1)" ]; then echo FAILED; else echo SUCCESS; fi; }
receipt_drift() { echo "$*" >> "$RUN_DIR/drift"; }
if inv_owner lbl contract -- create_liquidity_pool >/dev/null; then exit 1; fi
[ "$(built)" = 1 ] && sources_are_chan1 && [ ! -e "$RUN_DIR/drift" ] || exit 2
[ "$(cat "$RUN_DIR/sends")" = "$(printf '%064d' 1)" ] && [ ! -e "$RUN_DIR/violations" ] && [ ! -e "$RUN_DIR/recover" ] || exit 3
''')
assert [(a['status'], a['hash']) for a in actions] == [('FAIL', '0'*63+'1')], actions
attempts, _, actions = shell(OWNER_SETUP + r'''
tx_status() { echo UNKNOWN; }
if inv_owner lbl contract -- create_liquidity_pool >/dev/null; then exit 1; fi
[ "$(cat "$RUN_DIR/sends")" = "$(printf '%064d' 1)" ] && [ "$(built)" = 1 ] && [ ! -e "$RUN_DIR/recover" ] || exit 2
''')
assert [(a['status'], a['hash']) for a in actions] == [('FAIL', '0'*63+'1')], actions
print('owner_submit signs the owner entry from stdin, logs its hash before one send, recovers the receipt value and never re-signs after a FAILED receipt')

THROTTLED = r'''
INTEG_DIR="$RUN_DIR"
sleep() { echo "$1" >> "$RUN_DIR/slept"; }
H=$(printf '%064d' 5)
curl() {
    local n; n=$(( $(cat "$RUN_DIR/curls" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$RUN_DIR/curls"
    if [ "$n" -le "${CURL_429:-0}" ]; then printf '{"error":"rate_limited","retry_after":60}\n429'; return; fi
    case "$*" in
        *getLatestLedger*) printf '{"jsonrpc":"2.0","id":1,"result":{"sequence":100}}\n200';;
        *getTransaction*)
            if [ -n "${LANDED:-}" ]; then
                printf '{"jsonrpc":"2.0","id":1,"result":{"status":"SUCCESS","txHash":"%s","ledger":9,"envelopeXdr":"e","resultXdr":"r","resultMetaXdr":"m"}}\n200' "$H"
            else
                printf '{"jsonrpc":"2.0","id":1,"result":{"status":"NOT_FOUND","latestLedger":9}}\n200'
            fi;;
    esac
}
backoffs() {
    set -- $(cat "$RUN_DIR/slept" 2>/dev/null)
    [ "$#" -ge "$BACKOFFS" ] || return 1
    local n=0 s low
    for s; do
        n=$((n + 1)); [ "$n" -le "$BACKOFFS" ] || break
        low=${AFTER:-$((8 << (n - 1)))}; [ "$low" -le 60 ] || low=60
        [ "$s" -ge "$low" ] && [ "$s" -le $((low + low / 2)) ] || return 1
    done
}
'''
shell(THROTTLED + r'''
CURL_429=2 LANDED=1 BACKOFFS=2 AFTER=60
[ "$(tx_status "$H")" = SUCCESS ] && [ "$(cat "$RUN_DIR/curls")" = 3 ] && [ -s "$LOG_DIR/$H.receipt.json" ] || exit 1
backoffs && [ "$(wc -l < "$RUN_DIR/slept")" -eq 2 ] || exit 2
''')
shell(THROTTLED + r'''
CURL_429=1 BACKOFFS=1 AFTER=60
[ "$(latest_ledger 5)" = 100 ] && [ "$(cat "$RUN_DIR/curls")" = 2 ] && backoffs || exit 1
''')
shell(THROTTLED + r'''
CURL_429=99 THROTTLE_RETRIES=3 BACKOFFS=3 AFTER=60
if latest_ledger 5; then exit 1; fi
[ "$(cat "$RUN_DIR/curls")" = 4 ] && backoffs && [ "$(wc -l < "$RUN_DIR/slept")" -eq 3 ] || exit 2
''')
attempts, _, actions = shell(THROTTLED + r'''
BACKOFFS=1 n=0
stellar() { n=$((n+1)); if [ "$n" = 1 ]; then echo '❌ error: Request rejected `429`' >&2; return 1; fi; echo '"7"'; }
[ "$(view throttled contract -- balance)" = '"7"' ] && backoffs || exit 1
n=0; rm "$RUN_DIR/slept"
stellar() { n=$((n+1)); if [ "$n" = 1 ]; then echo 'error: tx ab4290cd rejected {"i128":"429"} "u32":429}' >&2; return 1; fi; echo '"8"'; }
[ "$(view hexsafe contract -- balance)" = '"8"' ] && [ ! -e "$RUN_DIR/slept" ] || exit 2
n=0
stellar() { n=$((n+1)); if [ "$n" = 1 ]; then printf 'error: No status yet:  Transport(\n  Rejected {\n    status_code: 429,\n  },\n)\n' >&2; return 1; fi; echo '"9"'; }
[ "$(view transport contract -- balance)" = '"9"' ] && backoffs || exit 3
''')
assert [(a['label'], a['cli_exit']) for a in attempts] == [('throttled', 1), ('throttled', 0), ('hexsafe', 1), ('hexsafe', 0), ('transport', 1), ('transport', 0)], attempts
assert [(a['label'], a['status']) for a in actions] == [('throttled', 'read'), ('hexsafe', 'read'), ('transport', 'read')], actions
for always in (False, True):
    attempts, _, actions = shell(THROTTLED + r'''
BACKOFFS=2 n=0
id=CAFVLWR3CPMH4CYZDJOKNK2FIJ54RR5PSADCQEBNFFDB3SBASL6EBQHD
printf '%s' "$id" > "$LOG_DIR/id.out"; printf candidate > "$LOG_DIR/candidate.wasm"
stellar() {
    n=$((n+1))
    if [ "$n" -le ALWAYS ]; then echo '❌ error: Request rejected `429`' >&2; return 1; fi
    local last=''; for last in "$@"; do :; done
    printf candidate > "$last"
}
rc=0; verify_deployed_wasm "$LOG_DIR/id.out" --wasm "$LOG_DIR/candidate.wasm" || rc=$?
backoffs && [ -s "$LOG_DIR/$id.fetch1.err" ] && [ -s "$LOG_DIR/$id.fetch2.err" ] || exit 1
if [ ALWAYS = 2 ]; then [ "$rc" = 0 ] && [ "$n" = 3 ] && [ -s "$LOG_DIR/$id.deployed.wasm" ] || exit 2
else [ "$rc" != 0 ] && [ "$n" = 7 ] && [ "$(wc -l < "$RUN_DIR/slept")" -eq 6 ] || exit 3; fi
'''.replace('ALWAYS', '99' if always else '2'))
    assert [a['status'] for a in actions] == ([] if not always else ['FAIL']), actions
    assert not always or actions[0]['label'] == 'deployed_fetch' and '429' in actions[0]['note'], actions
print('HTTP 429 on getTransaction, getLatestLedger, a view and a bytecode fetch backs off exponentially, bounded, hex-safe')

shell(THROTTLED + r'''
for payload in '{"jsonrpc":"2.0","id":1,"method":"sendTransaction","params":{"transaction":"AAAA"}}' '[{"method":"getLatestLedger"}]' 'garbage'; do
    if rpc_post 5 "$payload" 2>"$RUN_DIR/refused"; then exit 1; fi
    grep -q 'only read-only JSON-RPC methods' "$RUN_DIR/refused" || exit 2
done
[ ! -e "$RUN_DIR/curls" ] || exit 3
''')
print('rpc_post refuses sendTransaction and any non-read payload before curl runs')

attempts, _, actions = shell(THROTTLED + r'''
LANDED=1 BACKOFFS=1 n=0
fetch_resources() { RES_INSTR=1 RES_READ=0 RES_WRITE=0 RES_FEE=1; }
stellar() {
    n=$((n+1))
    if [ "$n" = 1 ]; then echo '❌ error: Request rejected `429`' >&2; return 1; fi
    echo "Signing transaction: $H" >&2; echo '"1"'
}
inv presign admin contract -- supply > "$RUN_DIR/presign.out" || exit 2
[ "$(cat "$RUN_DIR/presign.out")" = '"1"' ] && [ "$n" = 2 ] && backoffs && [ "$(wc -l < "$RUN_DIR/slept")" -eq 1 ] || exit 1
''')
assert [(a['label'], a['cli_exit'], a['hash']) for a in attempts] == [('presign', 1, None), ('presign', 0, '0'*63+'5')], attempts
assert [a['status'] for a in actions] == ['retry', 'ok'], actions
print('A pre-sign 429 on inv backs off with the throttle schedule, then one signed send confirms')

shell(THROTTLED + r'''
BACKOFFS=2
calls() { cat "$RUN_DIR/calls" 2>/dev/null || echo 0; }
tick() { echo $(( $(calls) + 1 )) > "$RUN_DIR/calls"; calls; }
stellar() {
    echo "$*" | grep -q -- '--send=no' || exit 9
    if [ "$(tick)" -le 2 ]; then echo '❌ error: Request rejected `429`' >&2; return 1; fi
    echo '{"price":"20000000000000"}'
}
REFLECTOR_CEX=R; source "HERE_DIR/lib/protocol.sh"; source "HERE_DIR/lib/assets.sh"
[ "$(reflector_band XLM 9)" = '182000000000000000 218000000000000000' ] && [ "$(calls)" = 3 ] && backoffs || exit 1
rm "$RUN_DIR/calls" "$RUN_DIR/slept"
stellar() { tick >/dev/null; echo 'error: HostError: Error(Contract, #100)' >&2; return 1; }
CONTROLLER=C PRIMARY_SPOKE_ID=1; hub_key() { echo key; }
if market_listing_exists 1 S; then exit 2; fi
[ "$(calls)" = 1 ] && [ ! -e "$RUN_DIR/slept" ] || exit 3
rm "$RUN_DIR/calls"
stellar() { if [ "$(tick)" = 1 ]; then echo '❌ error: Request rejected `429`' >&2; return 1; fi; echo '{}'; }
market_listing_exists 1 S && [ "$(calls)" = 2 ] || exit 4
rm "$RUN_DIR/calls" "$RUN_DIR/slept"
stellar() { tick >/dev/null; echo '❌ error: Request rejected `429`' >&2; return 1; }
THROTTLE_RETRIES=2 BACKOFFS=2
if sac_live CSAC; then exit 5; fi
[ "$(calls)" = 3 ] && backoffs && [ "$(wc -l < "$RUN_DIR/slept")" -eq 2 ] || exit 6
'''.replace('HERE_DIR', str(HERE)))
print('Inline CLI reads retry only a 429 within THROTTLE_RETRIES; other errors keep their first result')

shell(THROTTLED + r'''
now=$(date +%s)
if E2E_RPC_DEADLINE=$((now + 5)) throttle_sleep 1; then exit 1; fi
[ ! -e "$RUN_DIR/slept" ] || exit 2
E2E_RPC_DEADLINE=$((now + 100)) throttle_sleep 1 && [ "$(wc -l < "$RUN_DIR/slept")" -eq 1 ] || exit 3
throttle_sleep 1 3600 && s=$(tail -n1 "$RUN_DIR/slept") && [ "$s" -ge 120 ] && [ "$s" -le 180 ] || exit 7
throttle_sleep 1 45 && s=$(tail -n1 "$RUN_DIR/slept") && [ "$s" -ge 45 ] && [ "$s" -le 67 ] || exit 8
if E2E_RPC_DEADLINE=bad throttle_sleep 1; then exit 4; fi
rm "$RUN_DIR/slept"; CURL_429=99
if E2E_RPC_DEADLINE=$((now + 5)) latest_ledger 5; then exit 5; fi
[ "$(cat "$RUN_DIR/curls")" = 1 ] && [ ! -e "$RUN_DIR/slept" ] || exit 6
''')
print('E2E_RPC_DEADLINE bounds every throttle backoff; a read that cannot wait fails at once')

attempts, _, actions = shell(OWNER_SETUP + r'''
eval "base_$(declare -f stellar)"
stellar() { echo "$1 $2 ${E2E_RPC_DEADLINE:-none}" >> "$RUN_DIR/deadlines"; base_stellar "$@"; }
tx_status() { echo SUCCESS; }
start=$(date +%s)
[ "$(inv_owner lbl contract -- create_liquidity_pool)" = '"77"' ] || exit 1
while read -r a b d; do
    case "$a $b" in
        'tx send') [ "$d" -ge $((start + 120)) ] && [ "$d" -le $(( $(date +%s) + 120 )) ] || exit 2; sent=$d;;
        'tx simulate') [ "$d" = none ] || enforce=$d;;
        *) [ "$d" = none ] || exit 3;;
    esac
done < "$RUN_DIR/deadlines"
[ -n "${sent:-}" ] && [ "${enforce:-}" = "$sent" ] && [ "$(grep -c '^tx simulate none$' "$RUN_DIR/deadlines")" = 1 ] || exit 4
''')
assert [a['status'] for a in actions] == ['ok'], actions
print('owner_submit bounds the RPC slot wait of its post-signature simulate and send inside the auth expiry')

attempts, _, actions = shell(THROTTLED + r'''
CURL_429=1 n=0
stellar() { n=$((n+1)); echo "Signing transaction: $H" >&2; echo '❌ error: Request rejected `429`' >&2; return 1; }
if inv sent admin contract -- supply; then exit 1; fi
[ "$n" = 1 ] || exit 2
cli_deploy() { n=$((n+1)); echo "Signing transaction: $H" >&2; echo '❌ error: Request rejected `429`' >&2; return 1; }
if run_deploy "$LOG_DIR/deploy.out" "$LOG_DIR/deploy.err" -- cli_deploy; then exit 3; fi
[ "$n" = 2 ] && [ "$DEPLOY_ATTEMPTS" = 1 ] && [ ! -s "$LOG_DIR/deploy.out" ] || exit 4
''')
assert [(a['label'], a['hash']) for a in attempts] == [('sent', '0'*63+'5'), ('deploy', '0'*63+'5')], attempts
assert [(a['label'], a['status']) for a in actions] == [('sent', 'FAIL'), ('deployment_receipt', 'FAIL')], actions
assert all('status=UNKNOWN' in a['note'] for a in actions), actions
attempts, _, actions = shell(OWNER_SETUP + THROTTLED + r'''
eval "base_$(declare -f stellar)"
stellar() {
    if [ "$1 $2" = 'tx send' ]; then base_stellar "$@" >/dev/null || return; echo '❌ error: Request rejected `429`' >&2; return 1; fi
    base_stellar "$@"
}
H=$(printf '%064d' 1)
if inv_owner lbl contract -- create_liquidity_pool >/dev/null; then exit 1; fi
[ "$(cat "$RUN_DIR/sends")" = "$H" ] && [ "$(built)" = 1 ] && [ ! -e "$RUN_DIR/recover" ] || exit 2
''')
assert [(a['status'], a['hash']) for a in actions] == [('FAIL', '0'*63+'1')], actions
assert 'status=UNKNOWN' in actions[0]['note'], actions
print('A signed send rejected with 429 stays UNKNOWN: one send, no rebuild, sticky FAIL (inv, run_deploy, owner_submit)')

with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    (root/'bin').mkdir(); (root/'lib').mkdir()
    shutil.copy2(HERE/'bin/stellar', root/'bin/stellar')
    shutil.copy2(HERE/'lib/core.sh', root/'lib/core.sh')
    real = root/'real-stellar'
    real.write_text('#!/bin/bash\n[ "$1" != keys ] || { echo local >> "$SPAN"; exit 0; }\n'
                    'k=W; case " $* " in *" --send=no "*) k=R;; esac\necho "+$k" >> "$SPAN"; sleep 0.3; echo "-$k" >> "$SPAN"\n')
    quick = root/'quick-stellar'
    quick.write_text('#!/bin/bash\necho call >> "$SPAN"\n')
    for stub in (real, quick):
        stub.chmod(0o755)
    env = dict({k: v for k, v in os.environ.items() if k != 'E2E_SLOT_FDS'}, E2E_STELLAR=str(real), E2E_RPC_SLOTS='3', E2E_RPC_READ_SLOTS='1', E2E_SLOT_DIR=str(root/'slots'),
               E2E_RPC_WAIT_LOG=str(root/'waits'), SPAN=str(root/'span'))
    env.pop('E2E_RPC_DEADLINE', None)
    lane = r"""set -u
source "$1/lib/core.sh"; source "$1/lib/invoke.sh"
INTEG_DIR="$2"; RPC_URL=unused; PATH="$2/bin:$PATH"
curl() { echo +R >> "$SPAN"; command sleep 0.3; echo -R >> "$SPAN"; printf '{}\n200'; }
pids=()
for i in 1 2 3; do
    stellar contract invoke --id C --send=yes -- f & pids+=("$!")
    stellar contract invoke --id C --send=no -- f & pids+=("$!")
    rpc_post 5 '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' >/dev/null & pids+=("$!")
done
stellar keys address a || exit 1
for pid in "${pids[@]}"; do wait "$pid" || exit 2; done
"""
    lanes = [subprocess.Popen(['bash', '-c', lane, '_', str(HERE), directory], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) for _ in range(2)]
    for proc in lanes:
        out, err = proc.communicate(timeout=120)
        assert proc.returncode == 0, out + err
    events = (root/'span').read_text().split()
    depth = reads = peak = read_peak = 0
    for event in events:
        if event == 'local':
            continue
        step = 1 if event[0] == '+' else -1
        depth += step
        reads += step if event[1] == 'R' else 0
        peak, read_peak = max(peak, depth), max(read_peak, reads)
    assert events.count('+R') == events.count('-R') == 12 and events.count('+W') == events.count('-W') == 6, events
    assert events.count('local') == 2 and 2 <= peak <= 3 and read_peak == 1, events
    (root/'span').unlink(); (root/'waits').unlink()
    env['E2E_STELLAR'] = str(quick)
    slots = [str(root/f'slots/rpc.{i}') for i in (1, 2, 3)]
    holder = subprocess.Popen(['python3', '-c', 'import fcntl,sys,time\nfs=[open(p,"a") for p in sys.argv[1:]]\n'
                               '[fcntl.flock(f,fcntl.LOCK_EX) for f in fs]\nprint("held",flush=True)\ntime.sleep(120)', *slots],
                              stdout=subprocess.PIPE, text=True)
    try:
        assert holder.stdout.readline().strip() == 'held'
        network = [['contract', 'invoke', '--id', 'C', '--send=no'], ['contract', 'fetch', '--id', 'C'], ['tx', 'send'],
                   ['tx', 'simulate'], ['ledger', 'latest'], ['fees', 'stats'], ['contract', 'invoke', '--id', 'C', '--send=yes'],
                   ['keys', 'fund', 'a'], ['keys', 'generate', 'a', '--fund'], ['--quiet', 'contract', 'invoke']]
        local = [['keys', 'address', 'a'], ['keys', 'generate', 'a'], ['xdr', 'decode'], ['tx', 'sign'], ['tx', 'hash'],
                 ['contract', 'id', 'asset'], ['contract', 'alias', 'show', 'x'], ['--version'], ['version']]
        waiting = [subprocess.Popen([str(root/'bin/stellar'), *args], env=env, stderr=subprocess.PIPE, text=True) for args in network]
        for args in local:
            done = subprocess.run([str(root/'bin/stellar'), *args], env=env, capture_output=True, text=True, timeout=10)
            assert done.returncode == 0, (args, done.stderr)
        past = dict(env, E2E_RPC_DEADLINE=str(int(time.time()) - 1))
        done = subprocess.run([str(root/'bin/stellar'), 'tx', 'send'], env=past, capture_output=True, text=True, timeout=10)
        assert done.returncode == 1 and 'no RPC slot free within 0 s' in done.stderr, done.stderr
        presign = subprocess.run(['bash', '-c', 'source "$1/lib/invoke.sh"; grep -qE "$PRESIGN_RE" <<<"$2"', '_', str(HERE), done.stderr])
        assert presign.returncode == 0, done.stderr
        done = subprocess.run([str(root/'bin/stellar'), 'tx', 'send'], env=dict(env, E2E_RPC_DEADLINE='soon'), capture_output=True, text=True, timeout=10)
        assert done.returncode == 1 and 'invalid E2E_RPC_DEADLINE' in done.stderr, done.stderr
        time.sleep(1.5)
        assert all(proc.poll() is None for proc in waiting), [proc.poll() for proc in waiting]
        assert (root/'span').read_text().split() == ['call'] * len(local)
    finally:
        holder.kill(); holder.wait()
    for proc in waiting:
        assert proc.wait(timeout=30) == 0, proc.stderr.read()
    waits = [line.split('\t') for line in (root/'waits').read_text().splitlines()]
    assert len(waits) == len(network) and {w[2] for w in waits} == {'read', 'send'} and all(int(w[3]) >= 1 for w in waits), waits
    holder = subprocess.Popen(['python3', '-c', 'import fcntl,sys,time\nf=open(sys.argv[1],"a")\nfcntl.flock(f,fcntl.LOCK_EX)\nprint("held",flush=True)\ntime.sleep(120)',
                               str(root/'slots/rpc-read.1')], stdout=subprocess.PIPE, text=True)
    try:
        assert holder.stdout.readline().strip() == 'held'
        reads = [['contract', 'invoke', '--id', 'C', '--send=no'], ['contract', 'invoke', '--id', 'C', '--build-only'], ['contract', 'fetch', '--id', 'C'],
                 ['tx', 'simulate'], ['ledger', 'latest'], ['fees', 'stats']]
        blocked = [subprocess.Popen([str(root/'bin/stellar'), *args], env=env, stderr=subprocess.PIPE, text=True) for args in reads]
        for args in [['tx', 'send'], ['contract', 'invoke', '--id', 'C', '--send=yes'], ['contract', 'deploy', '--wasm', 'w']]:
            done = subprocess.run([str(root/'bin/stellar'), *args], env=env, capture_output=True, text=True, timeout=10)
            assert done.returncode == 0, (args, done.stderr)
        time.sleep(1.5)
        assert all(proc.poll() is None for proc in blocked), [proc.poll() for proc in blocked]
    finally:
        holder.kill(); holder.wait()
    for proc in blocked:
        assert proc.wait(timeout=30) == 0, proc.stderr.read()
    started = time.monotonic()
    done = subprocess.run(['bash', '-c', 'exec 150>/dev/null; E2E_SLOT_FDS="7 150" exec "$0" tx send', str(root/'bin/stellar')], env=env, capture_output=True, text=True, timeout=10)
    assert done.returncode == 1 and 'rpc slot pool refused the call: fd 150 already holds a slot' in done.stderr and time.monotonic() - started < 5, done.stderr
    done = subprocess.run(['bash', '-c', 'source "$1/lib/core.sh"; rpc_hold || exit 9; exec "$0" tx send', str(root/'bin/stellar'), str(HERE)],
                          env=env, capture_output=True, text=True, timeout=10)
    assert done.returncode == 1 and 'already holds a slot (nested hold)' in done.stderr, (done.returncode, done.stderr)
    for bash in ['/bin/bash', 'bash']:
        leak = subprocess.run([bash, '-c', '''source "$1/lib/core.sh"; exec 150>>"$2/leaked"; slot_take "$2/pool" 1 150 5 || exit 9
python3 -c 'import fcntl, sys; fcntl.flock(open(sys.argv[1], "a"), fcntl.LOCK_EX | fcntl.LOCK_NB)' "$2/pool.1" 2>/dev/null && exit 8
echo held >&150; exit 0''', '_', str(HERE), str(root)], env=env, capture_output=True, text=True, timeout=30)
        assert leak.returncode == 0 and (root/'pool.1').read_text() == 'held\n' and not (root/'leaked').read_text(), (bash, leak.returncode, leak.stderr)
        (root/'pool.1').write_text('')
    for inherited in ['exec 151>/dev/null', 'exec 151>/dev/null; export E2E_SLOT_FDS=150']:
        done = subprocess.run(['bash', '-c', inherited + '; exec "$0" tx send', str(root/'bin/stellar')], env=env, capture_output=True, text=True, timeout=10)
        assert done.returncode == 0, (inherited, done.stderr)
    for bad, message in [('', 'absolute path'), ('stellar', 'absolute path'), (str(root/'bin/stellar'), 'shim itself')]:
        done = subprocess.run([str(root/'bin/stellar'), 'contract', 'invoke'], env=dict(env, E2E_STELLAR=bad), capture_output=True, text=True, timeout=10)
        assert done.returncode == 1 and message in done.stderr, (bad, done.stderr)
print('Two lanes never exceed E2E_RPC_SLOTS, reads never exceed E2E_RPC_READ_SLOTS; only local CLI verbs bypass the pool;'
      ' a slot timeout is a pre-sign failure, a nested hold is refused at once, and each wait is logged')
