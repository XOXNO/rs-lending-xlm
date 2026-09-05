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
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SKIP_DIRS = ("target/", "vendor/", ".git/")

# Non-Rust files that legitimately define names the docs cite: env vars live in
# Dockerfiles and Compose files, Prometheus alert names in ops/alerts.yml.
EXTRA_SUFFIXES = (".toml", ".json", ".sh", ".py", ".yml", ".yaml")
EXTRA_NAMES = ("Makefile", "Dockerfile")

# Dependencies whose internal items the docs name directly. Their sources are
# not in this repo. They are listed explicitly rather than read out of
# ~/.cargo/registry, so the check gives the same answer on a cold checkout as on
# a warm one: depending on unpacked crate sources made `make docs-check` pass or
# fail according to whether Cargo happened to have fetched them.
#
# Adding a name here asserts that some dependency defines it. Keep each grouped
# under its crate so the claim stays checkable by hand.
EXTERNAL_SYMBOLS = {
    # stellar-tokens (OpenZeppelin non-fungible token): TTL constants, the
    # approval surface, and the enumerable/sequential helpers the position-NFT
    # docs describe.
    "OWNER_EXTEND_AMOUNT", "OWNER_TTL_THRESHOLD", "TOKEN_EXTEND_AMOUNT",
    "TOKEN_TTL_THRESHOLD", "approve_for_all", "is_approved_for_all",
    "ApproveForAll", "get_token_id", "get_owner_token_id", "total_supply",
    "NonFungibleTokenError", "NFTSequentialStorageKey", "TokenIdCounter",
    "next_token_id", "increment_token_id",
    "NFTEnumerableStorageKey", "OwnerTokens", "OwnerTokensIndex",
    "GlobalTokens", "GlobalTokensIndex",
    # stellar-governance (OpenZeppelin timelock): storage keys and predicates
    # the keeper README describes.
    "DONE_LEDGER", "MinDelay", "is_operation_done", "UnexecutedPredecessor",
    # stellar-contract-utils / stellar-access: error codes the test docs map.
    "EnforcedPause", "ExpectedPause",
    # mx-keyvault: the Azure credential env-var contract the keeper README
    # documents.
    "AZURE_TENANT_ID", "AZURE_CLIENT_ID", "AZURE_CLIENT_SECRET",
    "AZURE_IDENTITY_DISABLE_MANAGED_IDENTITY_CREDENTIAL",
}

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
    # Historical audit names from before the controller settlement refactor.
    # The archived analysis describes the original source; current functions
    # are settle_borrow/settle_repay and process_deposit.
    "docs/audit/controller-defense/synthesis/DRAIN-ANALYSIS.md": {
        "settle_debt", "settle_supply",
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
    # Edge and node types of the codebase-memory MCP graph, named while
    # explaining what that graph does and does not model. They are labels in an
    # external index, not items in this source tree.
    "CLAUDE.md": {"CALLS"},
    # Fragments used to illustrate the test-naming convention
    # (test_<entry>_<condition>_<expected>), not whole test names.
    "tests/test-harness/tests/README.md": {
        "exceeding_ltv", "stale_twap_history", "creates_position",
    },
    # Certora prover internals (Kotlin classes, config names, TAC operators),
    # compiler-rt intrinsics and WASM opcodes cited from the open-source prover
    # and the Rust sysroot while explaining how the WASM front-end models i128
    # arithmetic. None of them is an item in this tree.
    "docs/explanation/certora-sunbeam-prover-tuning.md": {
        "__udivti3", "__modti3", "__umodti3", "__ashlti3", "__ashrti3",
        "__lshrti3", "DIVTI3", "MODTI3", "UDIVTI3", "ASHLTI3",
        "fail_with_error", "i256_add", "i256_div", "i256_mul",
        "obj_from_i256_pieces", "overflowing_add", "wrapping_neg",
        "impl_signed_mulo", "widen_mul", "u128_div_rem", "IntModuleImpl",
        "ByteStore", "BitCounts", "shr_s", "shr_u", "div_u",
        "ExpNormalizerIA", "uninterp_bwand", "uninterp_bwor",
        "uninterp_bwashr", "uninterp_bwlshr", "uninterp_bwshl",
        "uninterp_bwxor", "ShiftRightArithmetical", "IntMul", "UninterpMul",
        "UninterpDiv", "UseLIA", "UseBV", "WITH_VERIFIER",
        "BitwiseAxiomGenerator", "test_overflow_add", "test_underflow_add",
        "AllCommonAvailableSolversWithClOptions",
    },
    # The review cites the same compiler-rt intrinsics and the compiler-builtins
    # helper the prover inlines when function names are stripped.
    "docs/explanation/certora-suite-review-2026-09-03.md": {
        "__udivti3", "__modti3", "u128_div_rem", "div_u",
        # Summaries the review found unused; they are being deleted on the
        # same branch, so the names describe what was removed.
        "reserves_summary", "supplied_amount_summary", "borrowed_amount_summary",
        "protocol_revenue_summary", "capital_utilisation_summary",
        "pool_snapshot_summary", "PoolViewsSnapshot", "fresh_monotone_index",
        "token_price_summary", "total_collateral_in_usd_summary",
        "total_borrow_in_usd_summary", "ltv_collateral_in_usd_summary",
    },
}


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
        if in_skipped_dir(rel) or not p.is_file():
            continue
        if p.suffix == ".rs":
            parts.append(p.read_text(errors="replace"))
            # File stems name test binaries and modules (`smoke_test`, `curve`).
            stems.append(p.stem)
        elif p.suffix in EXTRA_SUFFIXES or p.name in EXTRA_NAMES:
            parts.append(p.read_text(errors="replace"))
    parts.append(" ".join(stems))
    # Names owned by dependencies, asserted rather than scanned so the result
    # does not depend on whether Cargo has unpacked the crate sources.
    parts.append(" ".join(EXTERNAL_SYMBOLS))
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
