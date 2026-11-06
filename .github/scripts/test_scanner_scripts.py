#!/usr/bin/env python3
"""Offline scanner scope, failure propagation and Scout cleanup regressions."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import scope_scanner_output as scope

SCRIPTS = Path(__file__).resolve().parent


class ScannerScripts(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.work = Path(self.temp.name)
        self.bin = self.work / "bin"
        self.bin.mkdir()
        self.env = {**os.environ, "PATH": f"{self.bin}:{os.environ['PATH']}", "HOME": str(self.work / "home")}

    def stub(self, name, body):
        path = self.bin / name
        path.write_text("#!/bin/sh\n" + body)
        path.chmod(0o755)

    def run_script(self, name, **env):
        return subprocess.run(["bash", str(SCRIPTS / name)], cwd=self.work,
                              env={**self.env, **env}, capture_output=True, text=True)

    def test_scope(self):
        paths = [f"{prefix}lib.rs" for prefix in scope.IN_SCOPE]
        self.assertEqual(len(paths), 16)  # Nine contracts, six interfaces, common.
        excluded = ["mock/mock-oracle/src/lib.rs", "vendor/common/src/lib.rs",
                    "tests/fuzz/src/lib.rs", "contracts/pool/tests/test.rs",
                    "contracts/pool/src/../../../../outside.rs"]
        self.assertTrue(all(scope.in_scope(p) and scope.in_scope(str(scope.ROOT / p)) for p in paths))
        self.assertTrue(all(not scope.in_scope(p) for p in excluded))
        report = {"scanned": paths + excluded, "detector_responses": {
            "test": {"findings": [{"severity": "HIGH", "instances": [{"path": p} for p in paths + excluded]}]}}}
        result = subprocess.run([sys.executable, str(SCRIPTS / "scope_scanner_output.py")],
                                input=json.dumps(report), capture_output=True, text=True, check=True)
        narrowed = json.loads(result.stdout)
        self.assertEqual(narrowed["scanned"], sorted(paths))
        self.assertEqual(narrowed["detector_responses"]["test"]["findings"][0]["instances"], [{"path": p} for p in paths])

    def test_scanner_failures(self):
        self.stub("soroban-scanner", 'cat "$SCAN_FIXTURE"\n')
        fixture = self.work / "scan.json"
        report = {"scanned": ["contracts/pool/src/lib.rs"], "detector_responses": {}}
        for data, success in [(json.dumps(report), True), ("invalid json", False), ("{}", False),
                              (json.dumps({**report, "scanned": []}), False),
                              (json.dumps({**report, "detector_responses": {"test": {}}}), False),
                              (json.dumps({**report, "detector_responses": {"test": {"findings": {}}}}), False),
                              (json.dumps({**report, "detector_responses": {"test": {"findings": [{"instances": {}}]}}}), False)]:
            fixture.write_text(data)
            with self.subTest(data=data):
                self.assertEqual(self.run_script("run_scanner.sh", SCAN_FIXTURE=str(fixture)).returncode == 0, success)
        fixture.write_text(json.dumps(report))
        self.stub("python3", "exit 127\n")
        self.assertEqual(self.run_script("run_scanner.sh", SCAN_FIXTURE=str(fixture)).returncode, 127)

    def test_scout_cleanup_rejects_source_and_symlinks(self):
        for directory in (".git", "contracts", "target"):
            path = self.work / directory
            path.mkdir()
            (path / "keep").write_text("keep")
        (self.work / "target/scout-audit").symlink_to(self.work / "contracts", target_is_directory=True)
        for output in (".git", "contracts", ".", "target", "target/scout-audit", "target/scout-audit/../..", str(self.work / ".git")):
            with self.subTest(output=output):
                result = self.run_script("run_scout.sh", SCOUT_OUTPUT_DIR=output)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Refusing to clean Scout output", result.stderr)
                self.assertTrue(all((self.work / path / "keep").exists() for path in (".git", "contracts", "target")))

    def test_scout_accepts_dedicated_output(self):
        self.stub("mktemp", "exit 73\n")  # Stop after the guard, before cleanup or Scout.
        for output in ("target/scout-audit", "target/scout-audit/nested", str(self.work / "target/scout-audit")):
            with self.subTest(output=output):
                self.assertEqual(self.run_script("run_scout.sh", SCOUT_OUTPUT_DIR=output).returncode, 73)

    def test_scout_stages_existing_files_and_dangling_links(self):
        names = ["retained.rs", "untracked space.rs", "line\nbreak.rs", "-option.rs"]
        for name in names:
            (self.work / name).write_text(name)
        (self.work / "dangling").symlink_to("absent")
        listed = self.work / "git-files"
        listed.write_bytes(b"\0".join(name.encode() for name in [*names, "deleted.rs", "dangling"]) + b"\0")
        self.stub("git", 'cat "$SCOUT_FILE_LIST"\nexit "${GIT_EXIT:-0}"\n')
        staged = self.work / "staged"
        staged.mkdir()
        script = (SCRIPTS / "run_scout.sh").read_text()
        block = "git ls-files -z" + script.split("git ls-files -z", 1)[1].split('\nfind "$work_dir/contracts"', 1)[0]
        env = {**self.env, "file_list": str(self.work / "source-list"), "raw_file_list": str(self.work / "raw-list"),
               "work_dir": str(staged), "SCOUT_FILE_LIST": str(listed)}
        result = subprocess.run(["bash", "-c", "set -euo pipefail\n" + block], cwd=self.work,
                                env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual({path.name for path in staged.iterdir()}, {*names, "dangling"})
        self.assertTrue(all((staged / name).read_text() == name for name in names))
        self.assertEqual(os.readlink(staged / "dangling"), "absent")
        for contents, status in [(b"deleted.rs\0", 0), (b"", 0), (b"retained.rs\0", 42)]:
            listed.write_bytes(contents)
            result = subprocess.run(["bash", "-c", "set -euo pipefail\n" + block], cwd=self.work,
                                    env={**env, "GIT_EXIT": str(status)}, capture_output=True, text=True)
            self.assertEqual(result.returncode, status or 1)


if __name__ == "__main__":
    unittest.main()
