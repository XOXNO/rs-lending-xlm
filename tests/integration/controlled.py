#!/usr/bin/env python3
"""Bind existing Rust controlled-ledger tests to candidate evidence."""
import hashlib
import json
import re
import shutil
import sys
from pathlib import Path

if not __debug__:
    raise RuntimeError('release verification requires Python assertions; unset PYTHONOPTIMIZE')

MANIFEST = Path(__file__).with_name('controlled-cases.json')


def collect(log, candidate):
    cases = json.loads(MANIFEST.read_text())
    assert cases and len({c['id'] for c in cases}) == len(cases), 'invalid controlled cases'
    text = log.read_text()
    assert not re.search(r'^test result: FAILED\.|^error: (?:could not compile|test failed|doctest failed)', text, re.M), 'workspace checks failed in controlled log'
    for case in cases:
        matches = re.findall(r'^test '+re.escape(case['test'])+r'(?: - should panic)? \.\.\. (\w+)$',text,re.M)
        assert matches == ['ok'], f'controlled case missing/duplicate/not passed: {case["id"]}: {matches}'
    return dict(candidate=candidate,manifest_sha256=hashlib.sha256(MANIFEST.read_bytes()).hexdigest(),
                log_sha256=hashlib.sha256(log.read_bytes()).hexdigest(),cases=cases,status='pass')


def verify(proof, candidate, log=None):
    assert re.fullmatch(r'[0-9a-f]{64}', proof['log_sha256']), 'missing controlled log hash'
    if log is not None:
        assert hashlib.sha256(log.read_bytes()).hexdigest()==proof['log_sha256'], 'controlled log changed'
    assert proof['candidate']==candidate and proof['status']=='pass', 'controlled tests used a different candidate'
    assert proof['manifest_sha256']==hashlib.sha256(MANIFEST.read_bytes()).hexdigest(), 'controlled manifest changed'
    assert proof['cases']==json.loads(MANIFEST.read_text()), 'controlled case evidence incomplete'


if __name__=='__main__':
    log, directory = Path(sys.argv[1]),Path(sys.argv[2])
    proof=collect(log,json.loads((directory/'candidate.json').read_text()))
    shutil.copyfile(log,directory/'controlled-tests.log')
    (directory/'controlled.json').write_text(json.dumps(proof,indent=2)+'\n')
    print(f'Controlled ledger proofs: {len(proof["cases"])} passed')
