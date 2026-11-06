"""Offline negative controls for reports and Make failure propagation."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import coverage_report

ROOT = Path(__file__).resolve().parents[1]


class ScriptTools(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        (self.base / 'scripts').symlink_to(ROOT / 'scripts', target_is_directory=True)

    def test_coverage_paths_and_counts(self):
        source, report = self.base / 'input.lcov', self.base / 'report.md'
        with patch.object(coverage_report, 'ROOT', Path('/renamed/checkout')):
            for path in ('/renamed/checkout/contracts/pool/src/lib.rs', '/cached/checkout/common/src/lib.rs',
                         'contracts/pool/src/lib.rs'):
                source.write_text(f'SF:{path}\nLF:10\nLH:8\n')
                self.assertEqual(coverage_report.write_report(source, report, 'merged'), (8, 10, 80.0))
        for path in ('/home/.cargo/registry/hyper/src/common/lock.rs',
                     '/work/checkout/mock/mock-foo/src/common/helper.rs',
                     '/vendor/something/contracts/pool/src/lib.rs',
                     'vendor/contracts/pool/src/lib.rs', '../common/src/lib.rs', 'common/tests/lib.rs'):
            self.assertFalse(coverage_report.keep(path, 'merged'), path)
        for data in ('', 'SF:/outside/lib.rs\nLF:10\nLH:8\n',
                     'SF:contracts/pool/src/lib.rs\nLF:0\nLH:0\n',
                     'SF:contracts/pool/src/lib.rs\nLF:1\nLH:2\n',
                     'SF:contracts/pool/src/lib.rs\nLF:1\n'):
            source.write_text(data)
            with self.assertRaises(ValueError):
                coverage_report.write_report(source, report, 'merged')

    def test_scout_incomplete_reports_fail(self):
        valid = self.base / 'pool.json'
        valid.write_text(json.dumps({'summary': {'by_severity': {}}, 'findings': []}))
        command = [sys.executable, str(ROOT / 'scripts/scout-summary.py')]
        self.assertEqual(subprocess.run(command + [str(valid)], capture_output=True).returncode, 0)
        for data in ('{', '{}', '{"summary":{"by_severity":{}},"findings":null}',
                     '{"summary":{"by_severity":{}},"findings":[{"vulnerability_id":"real"}]}',
                     '{"summary":{"by_severity":{"high":1}},"findings":[]}'):
            (self.base / 'controller.json').write_text(data)
            result = subprocess.run(command + [str(self.base)], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn('No findings', result.stdout)
        for args in ([], [str(self.base / 'missing.json')]):
            self.assertNotEqual(subprocess.run(command + args, capture_output=True).returncode, 0)

    def test_make_propagates_test_and_report_failures(self):
        binaries = self.base / 'bin'
        binaries.mkdir()
        cargo = binaries / 'cargo'
        cargo.write_text(f'#!{sys.executable}\n' + '''
import os, pathlib, sys
args=sys.argv[1:]; mode=os.environ.get('REVIEW_FAILURE')
if args[:2]==['llvm-cov','report']:
    if mode=='report': sys.exit(43)
    pathlib.Path(args[args.index('--output-path')+1]).write_text('SF:contracts/pool/src/lib.rs\\nLF:10\\nLH:8\\n')
sys.exit(42 if mode=='harness' and 'test-harness' in args else 0)
''')
        cargo.chmod(0o755)
        env = dict(os.environ, PATH=str(binaries) + os.pathsep + os.environ['PATH'])
        for failure in ('', 'harness', 'report'):
            result = subprocess.run(['make', '-f', str(ROOT / 'Makefile'), 'coverage-merged',
                                     f'COV_DIR={self.base / "coverage"}', f'TEST_HARNESS_DIR={self.base / "harness"}'],
                                    cwd=self.base, env=env | {'REVIEW_FAILURE': failure}, capture_output=True, text=True)
            self.assertEqual(result.returncode == 0, failure == '', result.stdout + result.stderr)
            self.assertNotIn('(ignored)', result.stderr)

    def test_deploy_rejects_missing_or_unstrippable_artifacts(self):
        source, deploy = self.base / 'source', self.base / 'deploy'
        source.mkdir()
        deploy.mkdir()
        (source / 'controller.wasm').write_bytes(b'not reached')
        for invalid in (False, True):
            if invalid:
                (source / 'pool.wasm').write_bytes(b'invalid WASM')
            (deploy / 'pool.wasm').write_bytes(b'stale')
            result = subprocess.run(['make', '-f', str(ROOT / 'Makefile'), '-o', 'optimize', 'deploy-artifacts',
                                     'WASM_SIZE_CONTRACTS=pool controller', f'OPTIMIZED_DIR={source}',
                                     f'DEPLOY_DIR={deploy}', 'MAKE=true'], cwd=self.base, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((deploy / 'pool.wasm').exists())
            self.assertFalse((deploy / 'controller.wasm').exists())


if __name__ == '__main__':
    unittest.main()
