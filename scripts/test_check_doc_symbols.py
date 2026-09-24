"""Checks for the doc-symbol gate: skills/**/*.md are scanned even
before git tracks them, everything else stays tracked-only, and the
external-service skill trees are exempt without exempting the rest of skills/."""
import contextlib
import io
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import check_doc_symbols


class DocSymbolsTest(unittest.TestCase):
    def test_untracked_skills_are_scanned_but_untracked_docs_are_not(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            for d in ("src", "docs", "skills/new"):
                (root / d).mkdir(parents=True)
            (root / "src/lib.rs").write_text("pub fn real_symbol() {}\n")
            (root / "docs/tracked.md").write_text("Calls `real_symbol`.\n")
            (root / "docs/untracked.md").write_text("Calls `ghost_in_docs`.\n")
            skill = root / "skills/new/SKILL.md"
            skill.write_text("Calls `real_symbol`.\n")
            listing = subprocess.CompletedProcess(
                args=[], returncode=0, stdout="src/lib.rs\ndocs/tracked.md\n"
            )
            with patch.object(check_doc_symbols, "ROOT", root), patch.object(
                check_doc_symbols.subprocess, "run", return_value=listing
            ), contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(check_doc_symbols.main(), 0)
                skill.write_text("Calls `real_symbol` and `ghost_in_skill`.\n")
                self.assertEqual(check_doc_symbols.main(), 1)
                text = output.getvalue()
                self.assertIn("skills/new/SKILL.md:1  ghost_in_skill", text)
                self.assertNotIn("ghost_in_docs", text)
                self.assertIn("unknown symbols: 1", text)

    def test_external_doc_dirs_are_exempt_but_other_skills_are_not(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            external = "skills/xoxno-lending-data"
            for d in ("src", external, "skills/xoxno-lending-contracts"):
                (root / d).mkdir(parents=True)
            (root / "src/lib.rs").write_text("pub fn real_symbol() {}\n")
            # A name only the other repo defines: exempt, so no finding.
            (root / external / "api.md").write_text("Returns `AssetPageDto`.\n")
            contracts = root / "skills/xoxno-lending-contracts/SKILL.md"
            contracts.write_text("Calls `real_symbol`.\n")
            listing = subprocess.CompletedProcess(args=[], returncode=0, stdout="src/lib.rs\n")
            with patch.object(check_doc_symbols, "ROOT", root), patch.object(
                check_doc_symbols.subprocess, "run", return_value=listing
            ), contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(check_doc_symbols.main(), 0)
                # The exemption must not leak to the rest of skills/.
                contracts.write_text("Calls `ghost_in_contracts`.\n")
                self.assertEqual(check_doc_symbols.main(), 1)
                text = output.getvalue()
                self.assertIn("ghost_in_contracts", text)
                self.assertNotIn("AssetPageDto", text)


if __name__ == "__main__":
    unittest.main()
