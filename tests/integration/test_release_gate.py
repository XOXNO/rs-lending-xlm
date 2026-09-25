#!/usr/bin/env python3
"""Dry-run publication guard: injected failures and artifact swaps block it."""
from pathlib import Path
from copy import deepcopy
import json, hashlib, subprocess, sys
from controlled import MANIFEST
from controlled import collect as collect_controlled
import tempfile
from release_gate import LANES, verify
import release_gate
from artifacts import CONTRACTS, DISTRIBUTION_FILES, distribution

candidate = {'source_sha': 'candidate', 'artifacts': {'controller.wasm': 'a'*64}}
proof = {'status': 'pass', 'lanes': {l:1 for l in LANES}, 'candidate': candidate, 'controlled': {'candidate':candidate,'status':'pass','manifest_sha256':hashlib.sha256(MANIFEST.read_bytes()).hexdigest(),'log_sha256':'c'*64,'cases':json.loads(MANIFEST.read_text())}}
packaged = {'files': {'candidate.json': 'd'*64}, 'manifest_sha256': 'e'*64}
proof['distribution'] = packaged
verify(candidate, proof, packaged)
for mutation in ('failure', 'partial', 'empty', 'different_sha', 'different_hash'):
    broken = deepcopy(proof)
    if mutation == 'failure': broken['status'] = 'fail'
    if mutation == 'partial': del broken['lanes']['sdk']
    if mutation == 'empty': broken['lanes']['sdk'] = 0
    if mutation == 'different_sha': broken['candidate']['source_sha'] = 'other'
    if mutation == 'different_hash': broken['candidate']['artifacts']['controller.wasm'] = 'b'*64
    try:
        verify(candidate, broken, packaged)
    except (ValueError,AssertionError,KeyError):
        continue
    raise AssertionError(f'publication incorrectly allowed: {mutation}')
print('Release guard dry run passed: failures and artifact substitution block publication')

# The CLI must bind every uploaded byte, not only the production code. Fake
# module bytes suffice here: this regression tests packaging, not execution.
with tempfile.TemporaryDirectory() as directory:
    root=Path(directory); dist=root/'dist'; dist.mkdir()
    for name in DISTRIBUTION_FILES:
        (dist/name).write_bytes(b'original '+name.encode())
    # Valid module containing only a custom docs section. Both versions have
    # identical executable code; exact SDK bytes still belong to the release.
    (dist/'sdk-controller.wasm').write_bytes(b'\0asm\1\0\0\0\0\6\4docsA')
    bundled={'source_sha':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
        'artifacts':{f'{c}.wasm':hashlib.sha256((dist/f'{c}.wasm').read_bytes()).hexdigest() for c in CONTRACTS}}
    (dist/'candidate.json').write_text(json.dumps(bundled))
    (dist/'sdk-manifest.json').write_text(json.dumps({'sdk': 'metadata'}))
    log=dist/'controlled-tests.log'
    log.write_text('\n'.join('test '+case['test']+' ... ok' for case in json.loads(MANIFEST.read_text()))+'\n')
    controlled=collect_controlled(log,bundled)
    packaged=distribution(dist,create=True)
    original_validate=release_gate.validate
    release_gate.validate=lambda run,expected_lane: 1
    try:
        for lane in LANES:
            run=root/f'run-{lane}'; run.mkdir()
            (run/'candidate.json').write_text(json.dumps(bundled))
            (run/'controlled.json').write_text(json.dumps(controlled))
            (run/'controlled-tests.log').write_bytes(log.read_bytes())
        proof=release_gate.collect(root,'run',dist)
        assert proof['distribution']==packaged
        local=release_gate.collect(root,'run')
        assert 'distribution' not in local
        try: verify(bundled,local,packaged)
        except ValueError: pass
        else: raise AssertionError('standalone proof authorized publication without distribution')
    finally:
        release_gate.validate=original_validate
    proof_path=root/'proof.json'; proof_path.write_text(json.dumps(proof))
    def publication(succeeds):
        result=subprocess.run([sys.executable,str(Path(__file__).with_name('release_gate.py')),
            'verify',str(dist),str(proof_path)],capture_output=True,text=True)
        assert (result.returncode==0)==succeeds, result.stdout+result.stderr
    publication(True)
    sdk=dist/'sdk-controller.wasm'; original_sdk=sdk.read_bytes()
    sdk.write_bytes(original_sdk[:-1]+b'B')
    publication(False)
    sdk.write_bytes(original_sdk)
    # SDK metadata-only changes must fail even when executable code is intact.
    for name in ['sdk-controller.wasm','sdk-mock_reflector.wasm','sdk-manifest.json',
                 'controller.wasm.sha256','candidate.json','distribution.json']:
        path=dist/name; original=path.read_bytes()
        path.write_bytes(original+b'\n')
        publication(False)
        path.write_bytes(original)
    for name in ['sdk-controller.wasm','sdk-manifest.json','controller.wasm.sha256','distribution.json']:
        path=dist/name; original=path.read_bytes(); path.unlink()
        publication(False)
        path.write_bytes(original)
    for name in ['unexpected.wasm','unexpected.wasm.sha256']:
        path=dist/name; path.write_bytes(b'unbound')
        publication(False)
        try: distribution(dist,create=True)
        except AssertionError: pass
        else: raise AssertionError('unexpected upload asset entered distribution manifest')
        path.unlink()
    # Rehashing a substituted SDK file cannot replace the E2E-bound manifest.
    (dist/'sdk-controller.wasm').write_bytes(b'substituted')
    distribution(dist,create=True)
    publication(False)
print('Complete distribution substitution checks passed')

# Selected time tests passing must not hide a different failing workspace test.
with tempfile.TemporaryDirectory() as directory:
    log=Path(directory)/'controlled.log'
    passed='\n'.join('test '+case['test']+' ... ok' for case in json.loads(MANIFEST.read_text()))
    for failure in ['test result: FAILED. 1 passed; 1 failed;', 'error: could not compile `other-contract`']:
        log.write_text(passed+'\n'+failure+'\n')
        try: collect_controlled(log,candidate)
        except AssertionError: pass
        else: raise AssertionError('workspace failure hidden by selected controlled cases')

# Keep the actual workflow publication behind both job success and exact-byte
# verification; dry-run dispatch must never enter the GitHub release mutation.
workflow = (Path(__file__).resolve().parents[2]/'.github/workflows/release.yml').read_text()
publish = workflow.split('  publish:',1)[1]
assert 'needs: [build, testnet-e2e]' in publish
assert publish.index('release_gate.py verify') < publish.index('gh release upload')
assert "if: github.event_name != 'workflow_dispatch' || !inputs.dry_run" in publish
assert "if: github.event_name == 'workflow_dispatch' && inputs.dry_run && inputs.inject_e2e_failure" in workflow
assert workflow.index('Inject failed E2E gate') < workflow.index('Run parallel testnet e2e')
assert workflow.index('sdk_manifest.py dist') < workflow.index('artifacts.py distribution-create dist') < workflow.index('name: Upload artifact')
assert 'collect tests/integration/runs "$RUN_TS" artifacts/wasm/deploy' in workflow
assert workflow.count('dist/distribution.json') == 3  # Attestation, upload, publication.
for name in ['e2e.yml','release.yml']:
    content=(Path(__file__).resolve().parents[2]/'.github/workflows'/name).read_text()
    cargo_step=content.split('cargo test --workspace --no-fail-fast 2>&1 | tee controlled-tests.log',1)[0].rsplit('run: |',1)[1]
    assert 'set -o pipefail' in cargo_step, f'{name}: tee masks failed workspace tests'
