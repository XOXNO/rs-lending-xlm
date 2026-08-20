#!/usr/bin/env python3
"""Check that Rust-looking identifiers in markdown exist in the source tree.

Reads every tracked *.md, pulls out backticked identifiers that look like Rust
symbols (snake_case functions, CamelCase types, SCREAMING constants), and
reports the ones that appear nowhere in the sources. A hit here is either a
stale name the code has renamed or dropped, or a name that never existed.

The corpus is the repo's own sources plus the places docs legitimately cite
names from: ops/config files (env vars, Prometheus alert rules), Rust file
stems (test-binary and module names), and the registry sources of the few
dependencies whose internals the docs describe.

Usage: python3 scripts/check_doc_symbols.py [--quiet]
Exit status is 1 when unknown symbols remain, so CI can gate on it.
"""
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SKIP_DIRS = ("target/", "vendor/", ".git/")

# Non-Rust files that legitimately define names the docs cite: env vars live in
# Dockerfiles and Compose files, Prometheus alert names in ops/alerts.yml.
EXTRA_GLOBS = ("*.toml", "*.json", "*.sh", "*.py", "*.yml", "*.yaml",
               "Makefile", "Dockerfile")

# Dependencies whose internal items the docs name directly. Their sources are
# not in this repo, so the checker reads them from the Cargo registry:
# stellar-* are the OpenZeppelin Stellar contracts the protocol builds on
# (OZ token/timelock constants and entrypoints), mx-keyvault is the keeper's
# Azure credential loader (env-var contract documented in its README).
REGISTRY_CRATES = ("stellar-tokens", "stellar-governance", "stellar-access",
                   "stellar-contract-utils", "mx-keyvault")

# Words that look like symbols but are prose, tooling, or external API names.
ALLOW = {
    # external tooling / language keywords that appear in backticks
    "cargo", "clippy", "rustc", "wasm", "make", "grep", "sed", "jq", "curl",
    "docker", "python3", "bash", "sh", "git", "soroban", "stellar",
    # generic prose in backticks
    "true", "false", "None", "Some", "Ok", "Err", "Self", "Vec", "Option",
    "Result", "String", "Address", "Env", "Val", "Bytes", "BytesN", "Symbol",
    "Map", "u32", "u64", "i128", "u128", "i64", "bool", "usize",
    # status labels of docs/reference/invariants.md, not code identifiers
    "ENFORCED",
}

# Per-file allowances. Each entry is a name the file cites deliberately even
# though no such item exists in any source we can see; the comment says why.
FILE_ALLOW = {
    # Aave V4's own constants, quoted in the "Aave" column of the comparison
    # to contrast with our mechanisms. They belong to another codebase.
    "docs/explanation/aave-v4-audit-comparison.md": {
        "DUST_LIQUIDATION_THRESHOLD",
        "MAX_USER_RESERVES_LIMIT",
        "MAX_ALLOWED_SPOKE_CAP",
    },
    # Symbolic variable names the formula prose defines for itself right next
    # to the pseudo-code that uses them; they are math notation, not items.
    # (Skipping fenced blocks would not help: these are flagged in the prose
    # sentence that defines them, not inside the block.)
    "docs/reference/formulas.md": {
        "actual_borrowed", "actual_supplied", "milliseconds_per_year",
        "remaining_value",
    },
    # Cited as a name that deliberately does NOT exist ("`prices_status` is
    # not an entrypoint"); the real helper, fetch_prices_status, is checked.
    "services/lending-exporter/README.md": {"prices_status"},
    # Exported constant and frozen action-string table of @xoxno/sdk-js. The
    # file states these live in that repo and are not verifiable here.
    "skills/indexing-lending-events/SKILL.md": {
        "STELLAR_LENDING_TOPICS",
        "liq_repay", "liq_seize", "param_upd", "sw_debt_r", "sw_col_wd",
        "rp_col_wd", "rp_col_r", "close_wd",
    },
    # Fragments used to illustrate the test-naming convention
    # (test_<entry>_<condition>_<expected>), not whole test names.
    "tests/test-harness/tests/README.md": {
        "exceeding_ltv", "stale_twap_history", "creates_position",
    },
}


def in_skipped_dir(rel: str) -> bool:
    return any(rel.startswith(d) or f"/{d}" in rel for d in SKIP_DIRS)


def registry_roots():
    """Source dirs of the dependencies whose internals the docs cite."""
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    for crate in REGISTRY_CRATES:
        # Version-agnostic: any checked-out version defines the same names.
        found = sorted((cargo_home / "registry" / "src").glob(f"*/{crate}-*"))
        if not found:
            print(f"warning: {crate} sources not found under {cargo_home}; "
                  "its names may be reported as unknown", file=sys.stderr)
        yield from found


def sources() -> str:
    parts = []
    stems = []
    for p in ROOT.rglob("*.rs"):
        if in_skipped_dir(str(p.relative_to(ROOT))):
            continue
        parts.append(p.read_text(errors="replace"))
        # File stems name test binaries and modules (`smoke_test`, `curve`).
        stems.append(p.stem)
    for pat in EXTRA_GLOBS:
        for p in ROOT.rglob(pat):
            if in_skipped_dir(str(p.relative_to(ROOT))):
                continue
            parts.append(p.read_text(errors="replace"))
    for root in registry_roots():
        for p in root.rglob("*.rs"):
            parts.append(p.read_text(errors="replace"))
    parts.append(" ".join(stems))
    return "\n".join(parts)


def markdown_files():
    out = subprocess.run(
        ["git", "ls-files", "*.md"], cwd=ROOT, capture_output=True, text=True
    )
    for line in out.stdout.split("\n"):
        if line.strip():
            yield ROOT / line.strip()


def candidates(text: str):
    """Backticked tokens that look like Rust identifiers, not prose."""
    for m in re.finditer(r"`([A-Za-z_][A-Za-z0-9_]*)`", text):
        name = m.group(1)
        if name in ALLOW or len(name) < 4:
            continue
        snake = "_" in name and name.islower()
        camel = re.fullmatch(r"[A-Z][a-z0-9]+(?:[A-Z][a-z0-9]*)+", name)
        screaming = re.fullmatch(r"[A-Z][A-Z0-9_]{3,}", name)
        if snake or camel or screaming:
            yield m, name


def main() -> int:
    quiet = "--quiet" in sys.argv
    known = set(re.findall(r"\b([A-Za-z_][A-Za-z0-9_]*)\b", sources()))

    unknown = []
    for md in markdown_files():
        rel = str(md.relative_to(ROOT))
        allowed = FILE_ALLOW.get(rel, frozenset())
        text = md.read_text(errors="replace")
        for m, name in candidates(text):
            if name in known or name in allowed:
                continue
            line = text.count("\n", 0, m.start()) + 1
            unknown.append((rel, line, name))

    if unknown and not quiet:
        print("Symbols cited in markdown but absent from the source tree:")
        for path, line, name in unknown:
            print(f"  {path}:{line}  {name}")
    print(f"unknown symbols: {len(unknown)}")
    return 1 if unknown else 0


if __name__ == "__main__":
    raise SystemExit(main())
