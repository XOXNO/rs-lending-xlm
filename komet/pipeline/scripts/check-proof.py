#!/usr/bin/env python3
"""Check one Komet APR proof directory against the pipeline's evidence standard.

Usage: check-proof.py <proof-dir> <test-id> <pass|fail|reject> <out.json>

pass: native status PASSED; no pending, failing, stuck, vacuous or bounded
      nodes; not admitted; no circularity, refutations or subproofs.
fail: native status FAILED; no pending nodes; every failing leaf finished the
      Wasm call (empty instruction stack) and is about to compare the returned
      value: <k> starts with expectResult(SCVal:Bool(true)) while the host stack
      holds SCVal:Bool(false). A crash, timeout, stuck host call or trap is
      not an accepted negative control.

Runs inside the prover image (uses its pyk to load the graph).
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path
from typing import Any

from pyk.proof.reachability import APRProof


def find_cell(term: Any, label: str) -> Any | None:
    stack = [term]
    while stack:
        t = stack.pop()
        if isinstance(t, dict):
            if t.get('node') == 'KApply' and t['label']['name'] == label:
                return t
            stack.extend(t.get('args', []))
            if t.get('node') == 'KSequence':
                stack.extend(t.get('items', []))
    return None


def pretty(t: Any) -> str:
    n = t.get('node')
    if n == 'KApply':
        args = ', '.join(pretty(a) for a in t['args'])
        return f"{t['label']['name']}({args})" if t['args'] else t['label']['name']
    if n == 'KToken':
        return t['token']
    if n == 'KVariable':
        return t['name']
    if n == 'KSequence':
        return ' ~> '.join(pretty(a) for a in t['items'])
    return str(n)


def leaf_outcome(config: dict) -> dict:
    k = find_cell(config, '<k>')
    instrs = find_cell(config, '<instrs>')
    host = find_cell(config, '<hostStack>')
    k_items = k['args'][0]
    first = k_items['items'][0] if k_items.get('node') == 'KSequence' and k_items['items'] else k_items
    return {
        'k_head': pretty(first)[:200],
        'instrs': pretty(instrs['args'][0])[:200] if instrs else None,
        'host_stack': pretty(host['args'][0])[:200] if host else None,
    }


def is_trap(outcome: dict) -> bool:
    """A leaf stopped by a Wasm trap: `trap` heads the instruction stack."""
    return outcome['instrs'] is not None and outcome['instrs'].startswith('trap')


def main() -> int:
    proof_dir, pid, expect, out = Path(sys.argv[1]), sys.argv[2], sys.argv[3], Path(sys.argv[4])
    proof = APRProof.read_proof_data(proof_dir, pid)
    data = json.loads((proof_dir / pid / 'proof.json').read_text())
    graph = json.loads((proof_dir / pid / 'kcfg' / 'kcfg.json').read_text())
    inputs_path = proof_dir / pid / 'inputs.json'
    res: dict[str, Any] = {
        'id': pid,
        'expect': expect,
        'status': proof.status.name,
        'nodes': len(graph['nodes']),
        'edges': len(graph['edges']),
        'rewrite_steps': sum(e['depth'] for e in graph['edges']),
        'splits': len(graph.get('splits', [])),
        'covers': len(graph.get('covers', [])),
        'pending': [n.id for n in proof.pending],
        'failing': [n.id for n in proof.failing],
        'stuck': graph.get('stuck', []),
        'vacuous': graph.get('vacuous', []),
        'admitted': data.get('admitted'),
        'bounded': data.get('bounded'),
        'circularity': data.get('circularity'),
        'node_refutations': data.get('node_refutations'),
        'subproof_ids': data.get('subproof_ids'),
        'inputs_sha256': hashlib.sha256(inputs_path.read_bytes()).hexdigest() if inputs_path.exists() else None,
    }
    if inputs_path.exists():
        inputs = json.loads(inputs_path.read_text())
        res['input_fingerprint'] = {k: v for k, v in inputs.items() if k != 'extra_module'}
    errors: list[str] = []
    common_clean = {
        'pending': res['pending'] == [],
        'admitted': not res['admitted'],
        'bounded': not res['bounded'],
        'circularity': not res['circularity'],
        'refutations': not res['node_refutations'],
        'subproofs': not res['subproof_ids'],
    }
    errors += [f'{k} not clean' for k, ok in common_clean.items() if not ok]
    if expect == 'pass':
        if res['status'] != 'PASSED':
            errors.append(f"status {res['status']} != PASSED")
        for key in ('failing', 'stuck', 'vacuous'):
            if res[key]:
                errors.append(f'{key} nodes present: {res[key]}')
    elif expect == 'fail':
        if res['status'] != 'FAILED':
            errors.append(f"status {res['status']} != FAILED")
        if not res['failing']:
            errors.append('no failing leaf')
        res['failing_leaves'] = {}
        for nid in res['failing']:
            outcome = leaf_outcome(proof.kcfg.node(nid).cterm.config.to_dict())
            res['failing_leaves'][nid] = outcome
            ok = (
                outcome['k_head'].startswith('expectResult(SCVal:Bool(true))')
                and outcome['instrs'] in ('.K', '', '#EmptyK')
                and (outcome['host_stack'] or '').startswith('hostStackList(SCVal:Bool(false)')
            )
            if not ok:
                errors.append(f'failing leaf {nid} is not a completed false assertion: {outcome}')
    elif expect == 'reject':
        # Rejection claim: the harness returns true on the accepted domain and
        # `false` right after the validator call. Every failing leaf must stop
        # inside the call (a trap), never complete with `false`.
        res['failing_leaves'] = {}
        for nid in res['failing']:
            outcome = leaf_outcome(proof.kcfg.node(nid).cterm.config.to_dict())
            res['failing_leaves'][nid] = outcome
            if (outcome['host_stack'] or '').startswith('hostStackList(SCVal:Bool(false)'):
                errors.append(f'failing leaf {nid} returned false: the validator accepted an input it must reject')
            elif not is_trap(outcome):
                errors.append(f'failing leaf {nid} is not a trap: {outcome}')
        if not res['failing']:
            errors.append('no rejecting (trap) leaf: rejection domain not exercised')
        if res['status'] != 'FAILED':
            errors.append(f"status {res['status']} != FAILED")
    else:
        errors.append(f'unknown expectation {expect}')
    res['errors'] = errors
    res['verdict'] = 'OK' if not errors else 'REJECTED'
    out.write_text(json.dumps(res, indent=1, sort_keys=True) + '\n')
    print(json.dumps({k: res[k] for k in ('id', 'expect', 'status', 'nodes', 'rewrite_steps', 'verdict', 'errors')}))
    return 0 if not errors else 1


if __name__ == '__main__':
    sys.exit(main())
