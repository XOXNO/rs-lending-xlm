#!/usr/bin/env python3
"""Check that local markdown links resolve within the tracked repository.

Skips http(s), mailto and pure-anchor links. Strips a trailing `#L123` anchor
before testing the path, so source links may point at a line. Absolute paths
and untracked targets fail even when they exist locally.

Usage: python3 scripts/check_doc_links.py
Exit status is 1 when a link is broken, so CI can gate on it.
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")


def main() -> int:
    listed = subprocess.run(
        ["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout.rstrip("\0").split("\0")
    tracked = {ROOT / rel for rel in listed if rel}
    available = tracked | {
        ROOT / parent for path in tracked for parent in path.relative_to(ROOT).parents
    }

    broken = []
    for md in sorted(path for path in tracked if path.suffix == ".md"):
        rel = md.relative_to(ROOT)
        for lineno, line in enumerate(md.read_text(errors="replace").split("\n"), 1):
            for m in LINK.finditer(line):
                target = m.group(1).split("#")[0].strip()
                if not target or target.startswith(("http://", "https://", "mailto:")):
                    continue
                resolved = (md.parent / target).resolve()
                if Path(target).is_absolute() or resolved not in available or not resolved.exists():
                    broken.append((rel, lineno, target))

    for rel, lineno, target in broken:
        print(f"  {rel}:{lineno} -> {target}")
    print(f"broken links: {len(broken)}")
    return 1 if broken else 0


if __name__ == "__main__":
    raise SystemExit(main())
