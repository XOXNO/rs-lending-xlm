#!/usr/bin/env python3
"""Check that local markdown links resolve within the tracked repository.

Skips http(s) and mailto links. Absolute paths and untracked targets fail even
when they exist locally. An `#anchor` on a markdown target, or a pure `#anchor`
link, must name a heading of that file (GitHub slug rules, `-N` for repeats,
fenced blocks skipped) or an explicit `id`/`name` attribute. An anchor on any
other target (for example `#L123` on a source file) is not checked.

Usage: python3 scripts/check_doc_links.py
Exit status is 1 when a link is broken, so CI can gate on it.
"""
import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parent.parent
LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
HEADING = re.compile(r"^ {0,3}#{1,6}\s+(.*?)(?:\s+#+)?\s*$")
FENCE = re.compile(r"^ {0,3}(?:```|~~~)")
EXPLICIT_ANCHOR = re.compile(r"<[^>]*\b(?:id|name)\s*=\s*[\"']([^\"']+)[\"']")


def anchors(md: Path) -> set[str]:
    """Heading slugs and explicit `id`/`name` anchors that `md` defines."""
    found: set[str] = set()
    seen: dict[str, int] = {}
    fenced = False
    for line in md.read_text(errors="replace").split("\n"):
        if FENCE.match(line):
            fenced = not fenced
            continue
        if fenced:
            continue
        found.update(EXPLICIT_ANCHOR.findall(line))
        heading = HEADING.match(line)
        if heading:
            text = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", heading.group(1))
            text = re.sub(r"<[^>]+>", "", text)
            slug = re.sub(r"[^\w\- ]", "", text.lower()).replace(" ", "-")
            repeat = seen.get(slug, 0)
            seen[slug] = repeat + 1
            found.add(f"{slug}-{repeat}" if repeat else slug)
    return found


def main() -> int:
    listed = subprocess.run(
        ["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout.rstrip("\0").split("\0")
    tracked = {ROOT / rel for rel in listed if rel}
    available = tracked | {
        ROOT / parent for path in tracked for parent in path.relative_to(ROOT).parents
    }

    broken = []
    defined: dict[Path, set[str]] = {}
    for md in sorted(path for path in tracked if path.suffix == ".md"):
        rel = md.relative_to(ROOT)
        for lineno, line in enumerate(md.read_text(errors="replace").split("\n"), 1):
            for m in LINK.finditer(line):
                target, _, anchor = m.group(1).strip().partition("#")
                target = target.strip()
                if target.startswith(("http://", "https://", "mailto:")):
                    continue
                resolved = (md.parent / target).resolve() if target else md
                if target and (
                    Path(target).is_absolute() or resolved not in available or not resolved.exists()
                ):
                    broken.append((rel, lineno, target))
                    continue
                if anchor and resolved.suffix == ".md":
                    if resolved not in defined:
                        defined[resolved] = anchors(resolved)
                    if unquote(anchor) not in defined[resolved]:
                        broken.append((rel, lineno, f"{target}#{anchor}"))

    for rel, lineno, target in broken:
        print(f"  {rel}:{lineno} -> {target}")
    print(f"broken links: {len(broken)}")
    return 1 if broken else 0


if __name__ == "__main__":
    raise SystemExit(main())
