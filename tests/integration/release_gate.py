#!/usr/bin/env python3
"""Bind the full seven-lane proof to the exact release files."""
import json
import sys
from pathlib import Path

if not __debug__:
    raise RuntimeError('release verification requires Python assertions; unset PYTHONOPTIMIZE')
from artifacts import check, distribution
from gate import validate
from controlled import verify as verify_controlled

LANES = {'agg', 'liq', 'stress', 'flash', 'blend', 'production', 'sdk'}


def collect(runs, base, directory=None):
    candidate = None
    completed = {}
    controlled = None
    for lane in sorted(LANES):
        run = runs / f'{base}-{lane}'
        completed[lane] = validate(run, expected_lane=lane)
        current = json.loads((run / 'candidate.json').read_text())
        if candidate is not None and current != candidate:
            raise ValueError('lanes tested different candidates')
        candidate = current
        proof = json.loads((run / 'controlled.json').read_text())
        verify_controlled(proof, current, run/'controlled-tests.log')
        if controlled is not None and controlled != proof:
            raise ValueError('lanes used different controlled-ledger evidence')
        controlled = proof
    result = {'status': 'pass', 'lanes': completed, 'candidate': candidate, 'controlled': controlled}
    # Local standalone runs have no packaged SDK bundle. Publication requires it.
    if directory is not None:
        if check(directory) != candidate:
            raise ValueError('distribution differs from tested candidate')
        result['distribution'] = distribution(directory)
    return result


def verify(candidate, proof, packaged):
    if proof.get('status') != 'pass' or set(proof.get('lanes', {})) != LANES:
        raise ValueError('full E2E gate did not pass')
    if any(type(n) is not int or n <= 0 for n in proof['lanes'].values()):
        raise ValueError('missing completed cases')
    if proof['candidate'] != candidate:
        raise ValueError('publication files differ from tested candidate')
    if proof.get('distribution') != packaged or not packaged:
        raise ValueError('publication distribution differs from E2E proof')
    verify_controlled(proof['controlled'], candidate)


if __name__ == '__main__':
    try:
        if sys.argv[1] == 'collect':
            runs, base = Path(sys.argv[2]), sys.argv[3]
            directory = Path(sys.argv[4]) if len(sys.argv) > 4 else None
            (runs / f'{base}-release-gate.json').write_text(json.dumps(collect(runs, base, directory), indent=2)+'\n')
        elif sys.argv[1] == 'verify':
            proof=json.loads(Path(sys.argv[3]).read_text())
            candidate=check(Path(sys.argv[2]))
            verify(candidate, proof, distribution(Path(sys.argv[2])))
            verify_controlled(proof['controlled'],candidate,Path(sys.argv[2])/'controlled-tests.log')
        else:
            raise ValueError('expected collect/verify')
    except (ValueError, AssertionError, KeyError, OSError) as e:
        raise SystemExit(f'Publication blocked: {e}')
