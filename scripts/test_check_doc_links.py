"""Checks that a link resolves only to a tracked, existing, relative path in the
repo, and that a markdown #anchor names a heading or explicit anchor there."""
import contextlib
import io
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import check_doc_links


def run(root: Path, listing: str):
    """main() over `root` with `listing` as the git file list; (exit, output)."""
    listed = subprocess.CompletedProcess(args=[], returncode=0, stdout=listing)
    with patch.object(check_doc_links, "ROOT", root), patch.object(
        check_doc_links.subprocess, "run", return_value=listed
    ), contextlib.redirect_stdout(io.StringIO()) as output:
        return check_doc_links.main(), output.getvalue()


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
                "# Top\n"
                "[source](src/lib.rs#L1) [directory](src/) [space](with space.txt)\n"
                "[web](https://example.com) [mail](mailto:a@example.com) [anchor](#top)\n"
            )
            listing = "README.md\0src/lib.rs\0with space.txt\0missing.rs\0"
            self.assertEqual(run(root, listing)[0], 0)
            readme.write_text(
                readme.read_text()
                + f"[absolute]({root / 'src/lib.rs'})\n"
                + "[untracked](local.log)\n[missing](missing.rs)\n[escape](../)\n"
            )
            status, output = run(root, listing)
            self.assertEqual(status, 1)
            self.assertIn("broken links: 4", output)

    def test_markdown_anchors_must_name_a_heading_or_explicit_anchor(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / "src").mkdir()
            (root / "src/lib.rs").touch()
            (root / "other.md").write_text(
                "# Real Heading\n"
                "## Real Heading\n"
                "### INV-AUTH-01 — Pool `owner`, [linked](index.md) & more!\n"
                '<a id="adr-0001"></a>\n'
                "```\n# not a heading\n```\n"
            )
            index = root / "index.md"
            good = (
                "# Local\n"
                "[a](other.md#real-heading) [b](other.md#real-heading-1) "
                "[c](other.md#inv-auth-01--pool-owner-linked--more) "
                "[d](other.md#adr-0001) [e](#local) [f](src/lib.rs#L1)\n"
            )
            index.write_text(good)
            listing = "index.md\0other.md\0src/lib.rs\0"
            self.assertEqual(run(root, listing), (0, "broken links: 0\n"))
            index.write_text(
                good + "[x](other.md#missing) [y](#missing) [z](other.md#not-a-heading)\n"
            )
            status, output = run(root, listing)
            self.assertEqual(status, 1)
            self.assertIn("index.md:3 -> other.md#missing", output)
            self.assertIn("index.md:3 -> #missing", output)
            self.assertIn("index.md:3 -> other.md#not-a-heading", output)
            self.assertIn("broken links: 3", output)


if __name__ == "__main__":
    unittest.main()
