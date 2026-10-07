#!/usr/bin/env python3
"""Check archived bytes and native APR verdicts; no symbolic execution or certificate replay."""
from pathlib import Path, PurePosixPath
import hashlib
import json
import sys
import tarfile
import tempfile

if not __debug__:
    raise SystemExit("Run without Python optimization: checks use assertions.")
sys.setrecursionlimit(8000)
root = Path(__file__).resolve().parent

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

for line in (root / 'SHA256SUMS').read_text().splitlines():
    digest, name = line.split('  ', 1)
    path = PurePosixPath(name)
    assert not path.is_absolute() and '..' not in path.parts
    assert sha(root / name) == digest, name
result = json.loads((root / 'result.json').read_text())
archive = root / 'native-proof.tar.gz'
assert sha(archive) == result['archive_sha256']

with tempfile.TemporaryDirectory(prefix='komet-borrow-proof-') as directory:
    extracted = Path(directory)
    with tarfile.open(archive, 'r:gz') as bundle:
        members = bundle.getmembers()
        assert len(members) == result['archive_files']
        assert len({m.name for m in members}) == len(members)
        for member in members:
            path = PurePosixPath(member.name)
            assert member.isfile() and not path.is_absolute() and '..' not in path.parts
        bundle.extractall(extracted, members=members)
    manifest = (extracted / 'MANIFEST.sha256').read_text().splitlines()
    assert len(manifest) == len(members) - 1
    for line in manifest:
        digest, name = line.split('  ', 1)
        assert sha(extracted / name) == digest, name
    assert sha(extracted / 'wasm/pool_math_proof.wasm') == result['wasm_sha256']

    # Inspect with the exact archived pyk source; installed Komet dependencies are required.
    sys.path.insert(0, str(extracted / 'toolchain'))
    from pyk.proof.reachability import APRProof
    assert Path(sys.modules['pyk.proof.reachability'].__file__).is_relative_to(extracted)

    positive = APRProof.read_proof_data(extracted / 'positive', 'test_borrow_double')
    negative = APRProof.read_proof_data(extracted / 'negative', 'test_borrow_double_wrong')
    assert positive.status.name == 'PASSED' and not positive.pending and not positive.failing
    assert negative.status.name == 'FAILED' and not negative.pending
    assert [node.id for node in negative.failing] == [4]
    for name, proof in (('positive', positive), ('negative', negative)):
        claim = result['claim'] if name == 'positive' else result['negative_control']['claim']
        data = json.loads((extracted / name / claim / 'proof.json').read_text())
        graph = json.loads((extracted / name / claim / 'kcfg/kcfg.json').read_text())
        assert not data['admitted'] and not data['bounded'] and not data['circularity']
        assert not data['node_refutations'] and not data['subproof_ids']
        assert not graph['vacuous']
        if name == 'positive':
            assert not graph['stuck']
            assert len(graph['nodes']) == 47 and sum(e['depth'] for e in graph['edges']) == 5871
            assert {(c['source'], c['target']) for c in graph['covers']} == {
                (node, 2) for node in (13, 15, 22, 23, 40, 41, 42, 43, 44, 45, 46, 47)
            }
    review = json.loads((extracted / 'evidence/negative/independent-review.json').read_text())
    failure = review['assertion_failure']
    assert failure['actual']['args'][0]['token'] == 'false'
    assert failure['expected']['args'][0]['token'] == 'true'
    for key in ('actual', 'expected'):
        node = negative.kcfg.node(4).cterm.config.to_dict()
        for component in failure[key + '_path']:
            node = node[component]
        assert node == failure[key]
    assert json.loads((extracted / 'evidence/positive/process.json').read_text())['exit_code'] == 0
    assert json.loads((extracted / 'evidence/negative/process.json').read_text())['exit_code'] == 1

print('PASSED: archive hashes; native APR has no pending/failing leaves; false control rejected.')
print('This checks saved artifacts, not an independent replay of execution or a proof certificate.')
