"""Checks for the doc-symbol gate: skills/**/*.md are scanned even
before git tracks them, everything else stays tracked-only, the
external-service skill trees are exempt without exempting the rest of skills/,
the checker's own source is not corpus, and an unused allowance fails."""
import contextlib
import io
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import check_doc_symbols


def run(root: Path, listing: str, file_allow=None, external=frozenset()):
    """main() over `root` with `listing` as the git file list; (exit, output)."""
    listed = subprocess.CompletedProcess(args=[], returncode=0, stdout=listing)
    with patch.object(check_doc_symbols, "ROOT", root), patch.object(
        check_doc_symbols.subprocess, "run", return_value=listed
    ), patch.object(check_doc_symbols, "FILE_ALLOW", file_allow or {}), patch.object(
        check_doc_symbols, "EXTERNAL_SYMBOLS", external
    ), contextlib.redirect_stdout(io.StringIO()) as output:
        return check_doc_symbols.main(), output.getvalue()


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
            listing = "src/lib.rs\ndocs/tracked.md\n"
            self.assertEqual(run(root, listing)[0], 0)
            skill.write_text("Calls `real_symbol` and `ghost_in_skill`.\n")
            status, text = run(root, listing)
            self.assertEqual(status, 1)
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
            listing = "src/lib.rs\n"
            self.assertEqual(run(root, listing)[0], 0)
            # The exemption must not leak to the rest of skills/.
            contracts.write_text("Calls `ghost_in_contracts`.\n")
            status, text = run(root, listing)
            self.assertEqual(status, 1)
            self.assertIn("ghost_in_contracts", text)
            self.assertNotIn("AssetPageDto", text)

    def test_the_checker_source_is_not_corpus(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / "scripts").mkdir()
            (root / "docs").mkdir()
            shutil.copy(check_doc_symbols.__file__, root / "scripts/check_doc_symbols.py")
            (root / "scripts/check_doc_symbols.py").write_text(
                (root / "scripts/check_doc_symbols.py").read_text() + "\n# scoped_elsewhere\n"
            )
            (root / "docs/b.md").write_text("Cites `scoped_elsewhere`.\n")
            status, text = run(root, "scripts/check_doc_symbols.py\ndocs/b.md\n")
            self.assertEqual(status, 1)
            self.assertIn("docs/b.md:1  scoped_elsewhere", text)

    def test_an_unused_allowance_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / "docs").mkdir()
            (root / "docs/a.md").write_text("Cites `needed_name` and `DepSymbol`.\n")
            listing = "docs/a.md\n"
            used = {"docs/a.md": {"needed_name"}}
            self.assertEqual(run(root, listing, used, {"DepSymbol"})[0], 0)
            for file_allow, external in (
                ({**used, "docs/missing.md": {"ghost_name"}}, {"DepSymbol"}),
                ({"docs/a.md": {"needed_name", "ghost_name"}}, {"DepSymbol"}),
                (used, {"DepSymbol", "UnusedDep"}),
            ):
                status, text = run(root, listing, file_allow, external)
                self.assertEqual(status, 1, (file_allow, external))
                self.assertIn("stale allowances: 1", text)


if __name__ == "__main__":
    unittest.main()
