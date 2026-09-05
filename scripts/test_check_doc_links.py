"""Regression check for local files masking broken links in a clean checkout."""
import contextlib
import io
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import check_doc_links


class DocLinksTest(unittest.TestCase):
    def test_only_portable_tracked_targets_resolve(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / "src").mkdir()
            (root / "src/lib.rs").touch()
            (root / "local.log").touch()
            (root / "with space.txt").touch()
            readme = root / "README.md"
            readme.write_text(
                "[source](src/lib.rs#L1) [directory](src/) [space](with space.txt)\n"
                "[web](https://example.com) [mail](mailto:a@example.com) [anchor](#top)\n"
            )
            listing = subprocess.CompletedProcess(
                args=[], returncode=0,
                stdout="README.md\0src/lib.rs\0with space.txt\0missing.rs\0",
            )
            with patch.object(check_doc_links, "ROOT", root), patch.object(
                check_doc_links.subprocess, "run", return_value=listing
            ), contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(check_doc_links.main(), 0)
                readme.write_text(
                    readme.read_text()
                    + f"[absolute]({root / 'src/lib.rs'})\n"
                    + "[untracked](local.log)\n[missing](missing.rs)\n[escape](../)\n"
                )
                self.assertEqual(check_doc_links.main(), 1)
                self.assertIn("broken links: 4", output.getvalue())


if __name__ == "__main__":
    unittest.main()
