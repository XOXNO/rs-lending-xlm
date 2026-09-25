#!/usr/bin/env python3
"""Canonical file hashes; used before deployment and before publication."""
import hashlib
import json
import subprocess
import sys
from pathlib import Path

if not __debug__:
    raise RuntimeError('release verification requires Python assertions; unset PYTHONOPTIMIZE')

CONTRACTS = 'controller pool governance price_aggregator position_nft defindex_strategy aggregator xoxno-oracle-adapter'.split()
SDK_CONTRACTS = 'controller pool position_nft price_aggregator governance mock_reflector mock_redstone'.split()
DISTRIBUTION_FILES = ({f'{c}.wasm' for c in CONTRACTS}
    | {f'{c}.wasm.sha256' for c in CONTRACTS}
    | {f'sdk-{c}.wasm' for c in SDK_CONTRACTS}
    | {'sdk-manifest.json', 'candidate.json'})


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check(directory):
    manifest = json.loads((directory / 'candidate.json').read_text())
    assert set(manifest['artifacts']) == {f'{c}.wasm' for c in CONTRACTS}, 'candidate set mismatch'
    for name, expected in manifest['artifacts'].items():
        assert digest(directory / name) == expected, f'candidate hash mismatch: {name}'
    assert manifest['source_sha'] == subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(), 'candidate source SHA mismatch'
    return manifest


def distribution(directory, create=False):
    """Bind every release-upload input, including the manifest's exact bytes."""
    check(directory)
    published = {p.name for pattern in ('*.wasm', '*.wasm.sha256') for p in directory.glob(pattern)}
    published.update({'sdk-manifest.json', 'candidate.json'})
    assert published == DISTRIBUTION_FILES, 'distribution file set mismatch'
    hashes = {name: digest(directory / name) for name in sorted(published)}
    path = directory / 'distribution.json'
    if create:
        path.write_text(json.dumps(hashes, indent=2) + '\n')
    else:
        assert json.loads(path.read_text()) == hashes, 'distribution hash mismatch'
    return {'files': hashes, 'manifest_sha256': digest(path)}


if __name__ == '__main__':
    mode, directory = sys.argv[1], Path(sys.argv[2])
    if mode == 'create':
        manifest = dict(source_sha=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                        rustflags='-C link-arg=-zstack-size=16384',
                        artifacts={f'{c}.wasm': digest(directory / f'{c}.wasm') for c in CONTRACTS})
        (directory / 'candidate.json').write_text(json.dumps(manifest, indent=2) + '\n')
        for name, hash_ in manifest['artifacts'].items():
            (directory / f'{name}.sha256').write_text(f'{hash_}  {name}\n')
    elif mode == 'check':
        check(directory)
    elif mode in {'distribution-create', 'distribution-check'}:
        distribution(directory, create=mode == 'distribution-create')
    elif mode == 'policy':
        manifest = check(directory)
        budgets = {}
        for line in Path('configs/wasm_size_budget.txt').read_text().splitlines():
            if line.strip() and not line.lstrip().startswith('#'):
                name, size = line.split()
                budgets[name] = int(size)
        # Existing budgets predate these two release contracts. Keep explicit
        # ceilings below the network WASM-size limit until the shared file grows.
        budgets.update({'aggregator.wasm': 32000, 'xoxno-oracle-adapter.wasm': 32000})
        for name in manifest['artifacts']:
            size = (directory / name).stat().st_size
            assert size <= budgets[name], f'{name}: size {size} exceeds budget {budgets[name]}'
            print(f'OK {name}: {size}/{budgets[name]} bytes')
    else:
        raise SystemExit('expected create/check/policy/distribution-create/distribution-check')
