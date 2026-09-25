#!/usr/bin/env python3
"""Offline checks of the actual preflight target, without rebuilding candidates."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from artifacts import CONTRACTS, digest

ROOT = Path(__file__).resolve().parents[2]
MAKE = shutil.which('make')


class Preflight(unittest.TestCase):
    def test_native_optimized_build_outputs_and_fixture_isolation(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            for name in ('scripts/build_e2e_wasm.sh', 'scripts/strip_spec_docs.py',
                         'tests/integration/artifacts.py'):
                target = base / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / name, target)
            binaries = base / 'bin'
            binaries.mkdir()
            stellar = binaries / 'stellar'
            stellar.write_text(f'#!{sys.executable}\n' + '''
import os, sys
from pathlib import Path
args = sys.argv[1:]
if args[0] == 'xdr':
    sys.exit(0)
assert args[:2] == ['contract', 'build'] and '--optimize' in args
assert os.environ['RUSTFLAGS'] == '-C link-arg=-zstack-size=16384'
assert os.environ['CARGO_BUILD_RUSTFLAGS'] == os.environ['RUSTFLAGS']
assert 'CARGO_ENCODED_RUSTFLAGS' not in os.environ
if os.environ.get('FAIL_BUILD'):
    sys.exit(23)
pkg = args[args.index('--package') + 1].replace('-', '_')
out = Path(args[args.index('--out-dir') + 1])
out.mkdir(parents=True, exist_ok=True)
# Only --out-dir contains optimized bytes. Cargo's raw output must not be used.
raw = Path(os.environ['CARGO_TARGET_DIR']) / 'wasm32v1-none/release'
raw.mkdir(parents=True, exist_ok=True)
(raw / (pkg + '.wasm')).write_bytes(b'UNOPTIMIZED')
(out / (pkg + '.wasm')).write_bytes(b'\\0asm\\1\\0\\0\\0\\0\\x0f\\x0econtractspecv0')
''')
            stellar.chmod(0o755)
            git = binaries / 'git'
            git.write_text('#!/bin/sh\necho ' + 'a' * 40 + '\n')
            git.chmod(0o755)
            env = dict(os.environ, PATH=str(binaries) + os.pathsep + os.environ['PATH'],
                       CARGO_TARGET_DIR=str(base / 'custom target'),
                       RUSTFLAGS='wrong', CARGO_ENCODED_RUSTFLAGS='wrong')
            script = base / 'scripts/build_e2e_wasm.sh'
            def build(mode, **overrides):
                return subprocess.run(['bash', str(script), mode], env=env | overrides,
                                      capture_output=True, text=True)
            result = build('candidate')
            self.assertEqual(result.returncode, 0, result.stderr)
            candidates = base / 'artifacts/wasm/deploy'
            self.assertEqual({p.stem for p in candidates.glob('*.wasm')}, set(CONTRACTS))
            before = {p.name: p.read_bytes() for p in candidates.iterdir()}
            self.assertTrue(all(before[c + '.wasm'] == b'\0asm\1\0\0\0\0\x0f\x0econtractspecv0' for c in CONTRACTS))
            result = build('fixtures')
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(before, {p.name: p.read_bytes() for p in candidates.iterdir()})
            fixtures = base / 'artifacts/wasm/fixtures'
            self.assertEqual(len(list(fixtures.glob('*.wasm'))), 6)
            self.assertTrue(all(p.read_bytes() == b'\0asm\1\0\0\0\0\x0f\x0econtractspecv0' for p in fixtures.glob('*.wasm')))
            self.assertEqual(build('candidate', FAIL_BUILD='1').returncode, 23)
            self.assertEqual(build('fixtures', FAIL_BUILD='1').returncode, 23)

    def test_existing_candidates_and_failures(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            binaries = base / 'bin'
            binaries.mkdir()
            for name in 'bash python3 jq xxd curl base64 awk grep tr dirname git'.split():
                binaries.joinpath(name).symlink_to(shutil.which(name))
            stellar = binaries / 'stellar'
            candidate = base / 'candidate with spaces'
            candidate.mkdir()
            for contract in CONTRACTS:
                (candidate / f'{contract}.wasm').write_bytes(contract.encode())
            manifest = dict(source_sha=subprocess.check_output(
                ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                artifacts={f'{c}.wasm': digest(candidate / f'{c}.wasm') for c in CONTRACTS})
            guard = base / 'guard.mk'
            guard.write_text('candidate-wasm integration-fixtures:\n\t@echo UNEXPECTED_BUILD; exit 99\n')
            env = dict(os.environ, PATH=str(binaries), WASM_DIR=str(candidate),
                       NETWORK='testnet', INSTRUCTION_LEEWAY='2000000')
            for key in ['RUN_TS', 'MAKEFLAGS', 'MFLAGS', 'NETWORKS_FILE', 'BASH_ENV']:
                env.pop(key, None)

            cases = [
                ('valid', 'stellar 28.0.0 (commit)\nstellar-xdr 28.0.0', 0, True),
                ('newer', 'stellar 29.1.2', 0, True),
                ('old', 'stellar 27.9.9', 0, False),
                ('malformed', 'stellar banana', 0, False),
                ('bad_minor', 'stellar 28.nope.0', 0, False),
                ('wrong_program', 'other 28.0.0', 0, False),
                ('empty', '', 0, False),
                ('failed_command', 'stellar 28.0.0', 1, False),
                ('missing_tool', 'stellar 28.0.0', 0, False),
                ('changed_candidate', 'stellar 28.0.0', 0, False),
                ('missing_manifest', 'stellar 28.0.0', 0, False),
                ('wrong_source', 'stellar 28.0.0', 0, False),
            ]
            for name, version, code, success in cases:
                with self.subTest(name=name):
                    stellar.write_text(f'#!/bin/bash\nprintf "%s\\n" "{version}"\nexit {code}\n')
                    stellar.chmod(0o755)
                    manifest_path = candidate / 'candidate.json'
                    current = dict(manifest)
                    if name == 'wrong_source':
                        current['source_sha'] = '0' * 40
                    manifest_path.write_text(json.dumps(current))
                    artifact = candidate / f'{CONTRACTS[0]}.wasm'
                    artifact.write_bytes(CONTRACTS[0].encode())
                    if name == 'changed_candidate':
                        artifact.write_bytes(b'changed')
                    if name == 'missing_manifest':
                        manifest_path.unlink()
                    if name == 'missing_tool':
                        binaries.joinpath('curl').unlink()
                    elif not binaries.joinpath('curl').exists():
                        binaries.joinpath('curl').symlink_to(shutil.which('curl'))
                    result = subprocess.run(
                        [MAKE, '--no-print-directory', '-f', str(ROOT / 'Makefile'),
                         '-f', str(guard), 'integration-preflight'],
                        cwd=ROOT, env=env, capture_output=True, text=True)
                    self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
                    self.assertEqual('Preflight complete.' in result.stdout, success)
                    self.assertNotIn('UNEXPECTED_BUILD', result.stdout)
                    self.assertNotIn('integer expression expected', result.stderr)

    def test_minimum_version_guard(self):
        for minimum, success in [('28.0', True), ('28.1', False), ('29.0', False),
                                 ('invalid', False), ('28.nope', False)]:
            with self.subTest(minimum=minimum):
                result = subprocess.run(['bash', '-c', '''
source "$1"
stellar() { echo 'stellar 28.0.0'; }
STELLAR_CLI_MIN_VERSION="$2"
check_stellar_version
''', '_', str(ROOT / 'tests/integration/lib/core.sh'), minimum],
                    capture_output=True, text=True)
                self.assertEqual(result.returncode == 0, success, result.stderr)


if __name__ == '__main__':
    unittest.main()
