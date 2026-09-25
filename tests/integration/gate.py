#!/usr/bin/env python3
"""Fail-closed E2E case/action validation; no network access."""
import csv
import json
import sys
import re
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

if not __debug__:
    raise RuntimeError('release verification requires Python assertions; unset PYTHONOPTIMIZE')
from resources import check as check_resources
from receipts import verify as verify_receipt
from artifacts import CONTRACTS, digest

ACTION_FIELDS = 'seq phase label status fn hash instructions read_bytes write_bytes resource_fee note'.split()
GOOD = {'ok', 'read', 'xfail', 'sim-ok', 'retry', 'diagnostic'}


def rows(path, fields):
    with path.open(newline='') as f:
        reader = csv.DictReader(f, delimiter='\t')
        if reader.fieldnames != fields:
            raise ValueError(f'{path}: invalid header')
        result = list(reader)
    if not result or any(None in r or any(v is None for v in r.values()) for r in result):
        raise ValueError(f'{path}: empty or malformed records')
    return result


def mark_incomplete(run, reason):
    """Keep partial execution visible after a timeout, signal, or phase failure."""
    metadata = json.loads((run / 'metadata.json').read_text())
    path = run / 'cases.tsv'
    with path.open(newline='') as f:
        seen = {r['id'] for r in csv.DictReader(f, delimiter='\t')}
    with path.open('a') as f:
        writer = csv.writer(f, delimiter='\t', lineterminator='\n')
        for case in metadata['selected_cases']:
            if case not in seen:
                writer.writerow([case, 'incomplete', 0, 0])
    (run / 'interruption.json').write_text(json.dumps({'reason': reason,
        'observed_at': datetime.now(timezone.utc).isoformat()}) + '\n')
    write_summary(run, 'incomplete', reason=reason)


def write_summary(run, status, reason=None, exit_code=None):
    """Summarize partial evidence too; only validate() can establish a pass."""
    def table(name):
        path = run / name
        if not path.exists():
            return []
        with path.open(newline='') as stream:
            return list(csv.DictReader(stream, delimiter='\t'))
    metadata = json.loads((run/'metadata.json').read_text())
    actions, cases = table('actions.tsv'), table('cases.tsv')
    attempts = []
    path = run/'attempts.jsonl'
    for line in path.read_text().splitlines() if path.exists() else []:
        attempt = json.loads(line)
        receipt = run / attempt['receipt'] if attempt['receipt'] else None
        attempt['receipt_status'] = (json.loads(receipt.read_text()).get('result', {}).get('status', 'UNKNOWN')
            if receipt is not None and receipt.exists() else 'UNKNOWN' if attempt['hash'] else 'NOT_SUBMITTED')
        attempts.append(attempt)
    path = run/'summary.json'
    previous = json.loads(path.read_text()) if path.exists() else {}
    active_path = run/'active-attempt.json'
    active = json.loads(active_path.read_text()) if active_path.exists() else None
    if active:
        stderr = run/active['stderr']
        hashes = re.findall(r'Signing transaction: ([0-9a-f]{64})', stderr.read_text() if stderr.exists() else '')
        active['hash'] = hashes[-1] if hashes else None
        active['receipt_status'] = 'UNKNOWN'
    if status == 'completed':
        status = 'incomplete' if (run/'interruption.json').exists() else ('failed' if exit_code else previous.get('status', 'completed'))
        if status == 'running':
            status = 'completed'
    summary = dict(status=status, reason=reason if reason is not None else previous.get('reason'),
        exit_code=exit_code if exit_code is not None else previous.get('exit_code'),
        started_at=metadata.get('started_at'), updated_at=datetime.now(timezone.utc).isoformat(),
        run_id=metadata.get('run_id', run.name), lane=metadata.get('lane'),
        workflow_run_id=metadata.get('workflow_run_id'), workflow_run_attempt=metadata.get('workflow_run_attempt'),
        selected_cases=metadata['selected_cases'],
        cases=dict(Counter(c.get('status', 'malformed') for c in cases)),
        actions=dict(Counter(a.get('status', 'malformed') for a in actions)), attempts=attempts,
        active_attempt=active)
    # Atomic replacement preserves an earlier summary if a process is killed.
    temporary = path.with_suffix('.json.tmp')
    temporary.write_text(json.dumps(summary, indent=2)+'\n')
    temporary.replace(path)


def validate(run, expected_lane=None):
    if (run/'interruption.json').exists() or (run/'active-attempt.json').exists():
        raise ValueError('run was interrupted; start a fresh run')
    metadata = json.loads((run / 'metadata.json').read_text())
    if expected_lane is not None and metadata['lane'] != expected_lane:
        raise ValueError('lane identity differs from release gate selection')
    manifest = json.loads((Path(__file__).parent / 'cases.json').read_text())
    ids = [c['id'] for c in manifest]
    if len(ids) != len(set(ids)):
        raise ValueError('duplicate manifest case IDs')
    expected = {c['id'] for c in manifest if metadata['lane'] in c['lanes']}
    if not expected or set(metadata['selected_cases']) != expected:
        raise ValueError('unknown lane or selected cases differ from required manifest')
    if len(metadata['selected_cases']) != len(expected):
        raise ValueError('duplicate selected case IDs')
    for field in ['configuration_sha256', 'case_manifest_sha256', 'sdk_lock_sha256']:
        if not re.fullmatch(r'[0-9a-f]{64}', metadata[field]):
            raise ValueError(f'invalid metadata {field}')
    if metadata['case_manifest_sha256'] != digest(Path(__file__).with_name('cases.json')) or metadata['sdk_lock_sha256'] != digest(Path(__file__).parent/'sdk/package-lock.json'):
        raise ValueError('case manifest or SDK lock changed since execution')
    if not re.fullmatch(r'[0-9a-f]{40}', metadata['source_sha']) or type(metadata['instruction_leeway']) is not int or metadata['instruction_leeway'] != 20000000:
        raise ValueError('invalid source SHA or instruction policy')
    if metadata['network'] != 'testnet' or metadata['network_passphrase'] != 'Test SDF Network ; September 2015' or not metadata['rpc_url'].startswith('https://'):
        raise ValueError('invalid testnet identity')
    if not re.match(r'stellar 28\.', metadata['cli_version']) or (metadata['sdk_version'],metadata['stellar_sdk_version']) != ('1.0.221','16.3.0'):
        raise ValueError('unexpected tool versions')
    candidate = json.loads((run / 'candidate.json').read_text())
    if set(candidate['artifacts']) != {f'{c}.wasm' for c in CONTRACTS}:
        raise ValueError('incomplete production candidate')
    if candidate['source_sha'] != metadata['source_sha'] or not candidate['artifacts'] or any(not re.fullmatch(r'[0-9a-f]{64}', h) for h in candidate['artifacts'].values()):
        raise ValueError('missing/mismatched candidate identity')
    actions = rows(run / 'actions.tsv', ACTION_FIELDS)
    evidence = rows(run / 'evidence.tsv', ['seq', 'execution', 'contract'])
    if len(evidence) != len(actions) or (run / 'active-case').exists():
        raise ValueError('incomplete action evidence or interrupted case')
    deployed={}
    for line in (run/'deployed-artifacts.jsonl').read_text().splitlines():
        item=json.loads(line); name=item['artifact']
        if name not in candidate['artifacts']: continue
        if item['sha256']!=candidate['artifacts'][name] or not re.fullmatch(r'C[A-Z2-7]{55}',item['address']):
            raise ValueError('wrong deployed candidate identity')
        deployed.setdefault(name.removesuffix('.wasm'),set()).add(item['address'])
    submitted = set()
    for i, a in enumerate(actions, 1):
        if a['seq'] != str(i) or not a['label'] or a['status'] not in GOOD:
            raise ValueError(f'action {i}: {a["label"]} {a["status"]}: {a["note"]}')
        proof = evidence[i - 1]
        if proof['seq'] != str(i) or proof['execution'] not in {'assertion', 'simulation', 'transaction', 'rejected_transaction', 'classic_transaction', 'deployment'}:
            raise ValueError(f'action {i}: malformed execution evidence')
        a['execution'] = proof['execution']
        a['contract'] = proof['contract']
        if proof['execution'] in {'transaction', 'rejected_transaction', 'classic_transaction', 'deployment'}:
            if not re.fullmatch(r'[0-9a-f]{64}', a['hash']):
                raise ValueError(f'action {i}: missing submitted hash')
            if a['hash'] in submitted:
                raise ValueError(f'action {i}: transaction counted twice')
            submitted.add(a['hash'])
            receipt = json.loads((run / 'logs' / f'{a["hash"]}.receipt.json').read_text())
            result = receipt['result']
            status = 'FAILED' if proof['execution'] == 'rejected_transaction' else 'SUCCESS'
            decoded_resources = verify_receipt(receipt, a['hash'], metadata['network_passphrase'], status,
                proof['contract'] if proof['execution'] not in {'classic_transaction','deployment'} else None,
                a['fn'] if proof['execution'] not in {'classic_transaction','deployment'} else None)
            if proof['execution'] in {'transaction','deployment'}:
                resources = json.loads((run / 'logs' / f'{a["hash"]}.resources.json').read_text())
                if resources != decoded_resources:
                    raise ValueError(f'action {i}: resource sidecar differs from committed envelope')
                check_resources(json.loads((run / 'network-limits.json').read_text()), resources, result)
    cases = rows(run / 'cases.tsv', ['id', 'status', 'first_action', 'last_action'])
    seen = set()
    previous_end = 0
    definitions = {c["id"]: c for c in manifest}
    for c in cases:
        if c['id'] in seen or c['id'] not in expected or c['status'] != 'pass':
            raise ValueError(f'invalid/failed/duplicate case: {c}')
        seen.add(c['id'])
        lo, hi = int(c['first_action']), int(c['last_action'])
        if not previous_end < lo <= hi <= len(actions):
            raise ValueError(f'missing case evidence: {c["id"]}')
        previous_end = hi
        evidence = actions[lo - 1:hi]
        if not any(a['status'] in {'ok', 'read', 'xfail', 'sim-ok'} for a in evidence):
            raise ValueError(f'case lacks successful evidence: {c["id"]}')
        required_methods = set(definitions[c['id']]['contract_methods']) - {'prerequisite'}
        observed_methods = {a['fn'] for a in evidence if a['status'] in {'ok', 'read', 'xfail', 'sim-ok'}}
        for required in definitions[c['id']].get('required_actions', []):
            matches = [a for a in evidence if a['label'] == required['label'] and a['fn'] == required['method'] and a['status'] == required['status'] and a['execution'] in required['execution'] and (not required.get('contract') or a['contract'] in deployed.get(required['contract'],set()))]
            if len(matches) < required['count']:
                raise ValueError(f'{c["id"]}: missing required action {required}')
        if required_methods - observed_methods:
            raise ValueError(f'{c["id"]}: missing method evidence {sorted(required_methods - observed_methods)}')
    if seen != expected:
        raise ValueError(f'incomplete cases: {sorted(expected - seen)}')
    return len(cases)


if __name__ == '__main__':
    try:
        if sys.argv[1] == 'mark-incomplete':
            mark_incomplete(Path(sys.argv[2]), sys.argv[3])
            sys.exit(0)
        if sys.argv[1] == 'summary':
            write_summary(Path(sys.argv[2]), sys.argv[3], exit_code=int(sys.argv[4]) if len(sys.argv) > 4 else None)
            sys.exit(0)
        count = validate(Path(sys.argv[1]))
        write_summary(Path(sys.argv[1]), 'passed')
        print(f'GREEN: {count} required cases completed; no failed actions')
    except (ValueError, KeyError, OSError, IndexError, TypeError, AssertionError) as error:
        if len(sys.argv) > 1 and (Path(sys.argv[1])/'metadata.json').exists():
            try:
                write_summary(Path(sys.argv[1]), 'failed', reason=str(error))
            except (ValueError, KeyError, OSError, TypeError):
                pass
        print(f'FAILED: {error}', file=sys.stderr)
        sys.exit(1)
