#!/usr/bin/env python3
"""Offline checks of the actual preflight target, without rebuilding candidates."""
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from artifacts import CONTRACTS, digest

ROOT = Path(__file__).resolve().parents[2]
MAKE = shutil.which('make')


class Preflight(unittest.TestCase):
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
