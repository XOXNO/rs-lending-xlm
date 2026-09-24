#!/usr/bin/env python3
"""Check that Rust-looking identifiers in markdown exist in the source tree.

Reads every tracked *.md, pulls out backticked identifiers that look like Rust
symbols (snake_case functions, CamelCase types, SCREAMING constants), and
reports the ones that appear nowhere in the sources. A hit here is either a
stale name the code has renamed or dropped, or a name that never existed.

The corpus is the repo's own sources plus the places docs legitimately cite
names from: ops/config files (env vars, Prometheus alert rules), Rust file
stems (test-binary and module names), and `EXTERNAL_SYMBOLS`, a fixed list of
names that dependencies define. This checker and its test are not corpus, so
an allowance is scoped to the file it names. An allowance that no citation
needs fails as stale.

Usage: python3 scripts/check_doc_symbols.py [--quiet]
Exit status is 1 when unknown symbols or stale allowances remain, so CI can
gate on it.
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SKIP_DIRS = ("target/", "vendor/", ".git/")
SELF_FILES = ("scripts/check_doc_symbols.py", "scripts/test_check_doc_symbols.py")
IDENT = re.compile(r"\b([A-Za-z_][A-Za-z0-9_]*)\b")

# Non-Rust files that legitimately define names the docs cite: env vars live in
# Dockerfiles and Compose files, Prometheus alert names in
# services/*/ops/alerts.yml.
EXTRA_SUFFIXES = (".toml", ".json", ".sh", ".py", ".yml", ".yaml")
EXTRA_NAMES = ("Makefile", "Dockerfile")

# Dependencies whose internal items the docs name directly. Their sources are
# not in this repo. They are listed explicitly rather than read from
# ~/.cargo/registry, so the result does not depend on which crates Cargo fetched.
#
# Adding a name here asserts that some dependency defines it. Keep each grouped
# under its crate so the claim stays checkable by hand.
EXTERNAL_SYMBOLS = {
    # stellar-tokens (OpenZeppelin non-fungible token): the approval event, the
    # error enum and the enumerable storage keys the position-NFT docs describe.
    "ApproveForAll", "NonFungibleTokenError",
    "OwnerTokensIndex", "GlobalTokens", "GlobalTokensIndex",
    # stellar-governance (OpenZeppelin timelock): storage keys and predicates
    # the keeper README describes.
    "DONE_LEDGER", "MinDelay", "is_operation_done", "UnexecutedPredecessor",
    # mx-keyvault: the Azure credential env-var contract the keeper README
    # documents.
    "AZURE_IDENTITY_DISABLE_MANAGED_IDENTITY_CREDENTIAL",
}

# Per-file allowances. Each entry is a name the file cites deliberately even
# though no such item exists in any source we can see; the comment says why.
FILE_ALLOW = {
    # A DTO from the external xoxno-api-v2 repository used by the lending math
    # reference; its field semantics are linked to the SDK skill.
    "skills/xoxno-lending/math.md": {"ReserveIrmCurveDto"},
    # Edge and node types of the codebase-memory MCP graph, named while
    # explaining what that graph does and does not model. They are labels in an
    # external index, not items in this source tree.
    "CLAUDE.md": {"CALLS"},
    # Fragments used to illustrate the test-naming convention
    # (test_<entry>_<condition>_<expected>), not whole test names.
    "tests/test-harness/tests/README.md": {
        "exceeding_ltv", "stale_twap_history", "creates_position",
    },
    # A compiler-rt intrinsic the prover models, cited while explaining i128
    # arithmetic in the WASM front-end.
    "certora/README.md": {"__modti3"},
}


# Skill directories that document a service living in another repository: the
# public REST API (xoxno-api-v2) and the swap-aggregator quote server
# (arb-algo / stellar-indexer). Their names -- NestJS DTOs and pipes, quote
# server structs and env vars -- are real, but nothing in this repo defines
# them, so the check cannot resolve them and there is no local code for them to
# go stale against. Scoped to these two trees, not all of skills/: the other
# skills cite this repo's contracts and stay gated. check_doc_links.py still
# covers every file here.
EXTERNAL_DOC_DIRS = (
    "skills/xoxno-lending-data/",
    "skills/xoxno-swap-aggregator/",
)


def in_skipped_dir(rel: str) -> bool:
    return any(rel.startswith(d) or f"/{d}" in rel for d in SKIP_DIRS)



def tracked_files() -> list[Path]:
    """Files git tracks. Generated and ignored files are excluded on purpose:
    a check whose answer depends on whether the test suite has been run yet is
    not reproducible. `tests/**/test_snapshots/*.json` in particular are
    gitignored and only exist after `make test`."""
    out = subprocess.run(
        ["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True
    )
    return [ROOT / line for line in out.stdout.split("\n") if line.strip()]


def sources() -> str:
    parts = []
    stems = []
    for p in tracked_files():
        rel = str(p.relative_to(ROOT))
        if in_skipped_dir(rel) or rel in SELF_FILES or not p.is_file():
            continue
        if p.suffix == ".rs":
            parts.append(p.read_text(errors="replace"))
            # File stems name test binaries and modules (`smoke_test`, `curve`).
            stems.append(p.stem)
        elif p.suffix in EXTRA_SUFFIXES or p.name in EXTRA_NAMES:
            parts.append(p.read_text(errors="replace"))
    parts.append(" ".join(stems))
    return "\n".join(parts)


def markdown_files() -> list[Path]:
    """Tracked *.md everywhere, plus every skills/**/*.md on disk: skills are
    drafted for a while before they are tracked and should be checked from the
    first draft. docs/ and the rest stay tracked-only (see tracked_files)."""
    out = subprocess.run(
        ["git", "ls-files", "*.md"], cwd=ROOT, capture_output=True, text=True
    )
    files = {ROOT / line.strip() for line in out.stdout.split("\n") if line.strip()}
    files.update(
        p for p in (ROOT / "skills").rglob("*.md")
        if p.is_file() and not in_skipped_dir(str(p.relative_to(ROOT)))
    )
    return sorted(files)


def candidates(text: str):
    """Backticked tokens that look like Rust identifiers, not prose."""
    for m in re.finditer(r"`([A-Za-z_][A-Za-z0-9_]*)`", text):
        name = m.group(1)
        if len(name) < 4:
            continue
        snake = "_" in name and name.islower()
        camel = re.fullmatch(r"[A-Z][a-z0-9]+(?:[A-Z][a-z0-9]*)+", name)
        screaming = re.fullmatch(r"[A-Z][A-Z0-9_]{3,}", name)
        if snake or camel or screaming:
            yield m, name


def main() -> int:
    quiet = "--quiet" in sys.argv
    known = set(IDENT.findall(sources()))

    unknown = []
    used: set = set()
    for md in markdown_files():
        rel = str(md.relative_to(ROOT))
        if rel.startswith(EXTERNAL_DOC_DIRS):
            continue
        allowed = FILE_ALLOW.get(rel, frozenset())
        text = md.read_text(errors="replace")
        for m, name in candidates(text):
            if name in known:
                continue
            if name in allowed:
                used.add((rel, name))
            elif name in EXTERNAL_SYMBOLS:
                used.add(name)
            else:
                line = text.count("\n", 0, m.start()) + 1
                unknown.append((rel, line, name))
    stale = [f"EXTERNAL_SYMBOLS: {n}" for n in sorted(EXTERNAL_SYMBOLS) if n not in used] + [
        f"FILE_ALLOW[{rel!r}]: {n}"
        for rel, names in sorted(FILE_ALLOW.items())
        for n in sorted(names)
        if (rel, n) not in used
    ]

    if unknown and not quiet:
        print("Symbols cited in markdown but absent from the source tree:")
        for path, line, name in unknown:
            print(f"  {path}:{line}  {name}")
    if stale:
        print("Allowances no citation needs (delete them):")
        for entry in stale:
            print(f"  {entry}")
        print(f"stale allowances: {len(stale)}")
    print(f"unknown symbols: {len(unknown)}")
    return 1 if unknown or stale else 0


if __name__ == "__main__":
    raise SystemExit(main())
