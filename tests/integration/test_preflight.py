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
                         'scripts/sdk_manifest.py', 'tests/integration/artifacts.py'):
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
module = b'\\0asm\\1\\0\\0\\0\\0\\x0f\\x0econtractspecv0'
if 'short-timelock' in args:
    # The end-to-end governance build: its own target directory and a meta stamp.
    assert args[args.index('--features') + 1] == 'short-timelock' and pkg == 'governance'
    assert os.environ['CARGO_TARGET_DIR'].endswith('custom target/e2e-short-timelock')
    module += b'\\0\\x28\\x0econtractmetav0e2e-short-sensitive-delay'
(out / (pkg + '.wasm')).write_bytes(module)
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
            self.assertTrue((base / 'target/optimized/curator.wasm').is_file())
            self.assertTrue((base / 'custom target/wasm32v1-none/release/curator.wasm').is_file())
            self.assertEqual(before, {p.name: p.read_bytes() for p in candidates.iterdir()})
            fixtures = base / 'artifacts/wasm/fixtures'
            self.assertEqual(len(list(fixtures.glob('*.wasm'))), 7)
            short = fixtures / 'governance_e2e.wasm'
            self.assertIn(b'e2e-short-sensitive-delay', short.read_bytes())
            self.assertTrue(all(p.read_bytes() == b'\0asm\1\0\0\0\0\x0f\x0econtractspecv0'
                                for p in fixtures.glob('*.wasm') if p != short))
            # The deployable governance build is not replaced by the short one.
            self.assertFalse((base / 'custom target/wasm32v1-none/release/governance_e2e.wasm').exists())
            self.assertNotIn(b'e2e-short-sensitive-delay', (candidates / 'governance.wasm').read_bytes())

            mocks = {'mock_reflector': 'mock_oracle', 'mock_redstone': 'mock_redstone'}
            default_release = base / 'target/wasm32v1-none/release'
            default_release.mkdir(parents=True)
            for crate in mocks.values():
                (default_release / f'{crate}.wasm').write_bytes(b'DEFAULT mock sentinel')
            outside = base / 'outside'
            outside.mkdir()
            sdk_cases = [
                ('absolute', str(base / 'custom target'), base, b'UNOPTIMIZED'),
                ('relative', 'custom target', outside, b'UNOPTIMIZED'),
                ('default', None, outside, b'DEFAULT mock sentinel'),
                ('missing', str(base / 'missing target'), base, None),
                ('empty', '', base, None),
            ]
            for case, cargo_target, cwd, expected in sdk_cases:
                with self.subTest(sdk_case=case):
                    sdk_env = dict(env)
                    if cargo_target is None:
                        sdk_env.pop('CARGO_TARGET_DIR', None)
                    else:
                        sdk_env['CARGO_TARGET_DIR'] = cargo_target
                    out = base / f'sdk-{case}'
                    result = subprocess.run(
                        [sys.executable, str(base / 'scripts/sdk_manifest.py'), str(out)],
                        cwd=cwd, env=sdk_env, capture_output=True, text=True, timeout=10)
                    if expected is None:
                        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                        self.assertNotIn('wrote 7 files', result.stdout)
                        self.assertFalse((out / 'sdk-manifest.json').exists())
                        if case == 'empty':
                            self.assertIn('CARGO_TARGET_DIR must not be empty', result.stderr)
                            self.assertFalse(out.exists())
                        continue
                    self.assertEqual(result.returncode, 0, result.stderr)
                    sdk_manifest = json.loads((out / 'sdk-manifest.json').read_text())
                    self.assertEqual(set(sdk_manifest), {
                        'controller', 'pool', 'position_nft', 'price_aggregator', 'governance',
                        'mock_reflector', 'mock_redstone'})
                    for name, crate in mocks.items():
                        output = out / f'sdk-{name}.wasm'
                        self.assertEqual(output.read_bytes(), expected)
                        self.assertEqual(sdk_manifest[name]['sha256'], digest(output))
                    for name in ('controller', 'pool', 'position_nft', 'price_aggregator', 'governance'):
                        self.assertEqual((out / f'sdk-{name}.wasm').read_bytes(), before[f'{name}.wasm'])

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
                       NETWORK='testnet', INSTRUCTION_LEEWAY='20000000')
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


    def test_candidate_abi_inspection_failures(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            binaries = base / 'bin'
            binaries.mkdir()
            for name in ('python3', 'tr', 'basename', 'uname'):
                (binaries / name).symlink_to(shutil.which(name))
            scripts = base / 'scripts'
            scripts.mkdir()
            (scripts / 'check_access_control.py').write_text('''import os, sys
assert sys.argv[1:] == ['--list-test-only']
case = os.environ['ABI_CASE']
if case == 'classifier_error': sys.exit('classifier failed')
if case != 'classifier_empty':
    print('governance:' + ('set_price_aggregator' if case == 'exempt' else 'execute_immediate'))
''')
            strings = binaries / 'strings'
            emitter = f'#!{sys.executable}\n' + '''import os, signal, sys
from pathlib import Path
assert Path(sys.argv[1]).is_file()
signal.signal(signal.SIGPIPE, signal.SIG_DFL)
case = os.environ['ABI_CASE']
if case == 'strings_error': sys.exit(23)
word = b'execute_immediate\\n' if case in ('forbidden', 'large_forbidden', 'strings_forbidden_error') else b'benign\\n'
os.write(1, word)
if case == 'large_forbidden':
    for _ in range(256): os.write(1, b'x' * 8192 + b'\\n')
sys.exit(23 if case in ('strings_partial_error', 'strings_forbidden_error') else 0)
'''
            grep = binaries / 'grep'
            grep.write_text(f'#!{sys.executable}\n' + f'''import os, sys
case = os.environ['ABI_CASE']
if case == 'grep_error': sys.exit(2)
if case == 'grep_missing': sys.exit(127)
os.execv({shutil.which('grep')!r}, [{shutil.which('grep')!r}] + sys.argv[1:])
''')
            grep.chmod(0o755)
            artifacts = base / 'candidate with spaces'
            artifacts.mkdir()
            wasm = artifacts / 'governance.wasm'
            bash_env = base / 'bash-env'
            env = dict(os.environ, PATH=str(binaries))
            for key in ('BASH_ENV', 'ENV', 'MAKEFLAGS', 'MFLAGS', 'GNUMAKEFLAGS', 'SHELLOPTS'):
                env.pop(key, None)
            cases = [
                ('clean', True, 'OK   1 test-only'),
                ('forbidden', False, 'exports test-only ABI'),
                ('strings_error', False, 'cannot inspect'),
                ('strings_partial_error', False, 'cannot inspect'),
                ('strings_forbidden_error', False, 'cannot inspect'),
                ('grep_error', False, 'symbol search failed'),
                ('grep_missing', False, 'symbol search failed'),
                ('large_forbidden', False, 'exports test-only ABI'),
                ('missing_strings', False, 'cannot inspect'),
                ('missing_artifact', False, 'no deploy WASM'),
                ('classifier_error', False, 'classifier failed'),
                ('classifier_empty', False, 'reports no test-only entrypoints'),
                ('exempt', True, 'OK   0 test-only'),
            ]
            for errexit, pipefail in ((False, False), (False, True), (True, False), (True, True)):
                bash_env.write_text(('set -e\n' if errexit else 'set +e\n') + 'set ' + ('-o' if pipefail else '+o') + ' pipefail\n')
                for case, success, marker in cases:
                    with self.subTest(case=case, errexit=errexit, pipefail=pipefail):
                        wasm.write_bytes(b'local ABI inspection fixture')
                        strings.write_text(emitter)
                        strings.chmod(0o755)
                        if case == 'missing_strings':
                            strings.unlink()
                        if case == 'missing_artifact':
                            wasm.unlink()
                        result = subprocess.run(
                            [MAKE, '--no-print-directory', '-f', str(ROOT / 'Makefile'),
                             'candidate-testing-abi-check', f'DEPLOY_DIR={artifacts}',
                             'WASM_ABI_EXEMPT_SYMBOLS=set_price_aggregator'],
                            cwd=base, env=env | {'ABI_CASE': case, 'BASH_ENV': str(bash_env)},
                            capture_output=True, text=True, timeout=10)
                        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
                        self.assertIn(marker, result.stdout + result.stderr)
                        if not success:
                            self.assertNotIn('OK   ', result.stdout)


if __name__ == '__main__':
    unittest.main()
