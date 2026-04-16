# Supply Chain Risk Report

---

## Metadata

- **Scan Date**: 2026-04-16
- **Project**: rs-lending-xlm (Stellar/Soroban Rust lending protocol)
- **Frozen Commit**: 5ee115c
- **Repositories Scanned**: 4 in-scope crates (controller, pool, pool-interface, common) + workspace
- **Total Dependencies**: 249 transitive crates resolved (host build); ~30 in `wasm32v1-none` on-chain build graph
- **cargo-audit**: 3 advisories (all warnings, no vulnerabilities)

---

## Executive Summary

The lending protocol's direct supply-chain surface is small and well-controlled. Only **four** crates.io dependencies enter the on-chain `wasm32v1-none` build (`soroban-sdk`, `stellar-access`, `stellar-macros`, `stellar-contract-utils`); each is pinned to an exact registry release. The remaining ~245 transitive crates only appear in native test/host builds (`testutils`) and never touch deployed bytecode. The vendored CVLR copy under `vendor/cvlr/` is **opt-in behind the `certora` feature** and is not compiled into the deployed contract under any production path.

### Soroban-SDK pinning

`soroban-sdk = "25.3.1"` in `Cargo.toml` is a caret requirement. Lockfile resolves to exactly `25.3.1` with sha256 `4502f2e018f238a4c5d3212d7d20ea6abcdc6e58babd63b642b693739db30fd1`. Upgrade exposure: `cargo update -p soroban-sdk` could lift to the next patch silently. Recommend tightening to `=25.3.1` for the audit-frozen build, since the wasm cdylib hash is part of the on-chain identity.

### OpenZeppelin Stellar contracts

`stellar-access`, `stellar-macros`, `stellar-contract-utils` all at `0.7.0`, all from crates.io with checksums recorded. Source: `OpenZeppelin/stellar-contracts` (~79 stars, MIT, 9 open issues, last push 2026-04-10 — actively maintained, organization-owned). Same caret-pinning concern applies — recommend `=0.7.0`. No known CVEs against these crates. Note: relatively young (the OZ Stellar effort began Dec 2024) and small star count means the codebase has comparatively few external eyes; it is the highest-leverage trusted dep aside from the SDK itself.

### Vendored CVLR — diff vs upstream

Diffed `vendor/cvlr/` against upstream tag `cvlr-spec-v0.6.1` (commit `0b33500`). Exactly **two textual changes**, both `no_std` adapters with **no semantic effect**:

1. `vendor/cvlr/cvlr-spec/src/lib.rs:1` — added `#![no_std]` (matches the workspace `Cargo.toml` comment).
2. `vendor/cvlr/cvlr-log/src/core.rs:242` — replaced `std::file!()`/`std::line!()` with `::core::file!()`/`::core::line!()`. These are the same compiler-builtin macros re-exported under both `std` and `core`.

**Conclusion**: no spec-bypass risk introduced by the vendor patches. CVLR is also gated behind `--features certora` and is not present in any default build artifact. The Cargo.toml workspace comment accurately documents the patch.

### Ark crypto crates

`ark-bls12-381`, `ark-bn254`, `ark-ec`, `ark-ff`, `ark-poly`, `ark-serialize`, `ark-std` are pulled by `soroban-env-host 25.0.1` (host VM only). `cargo tree -p controller --target wasm32v1-none` shows zero `ark-*` in the on-chain dep graph. `Grep` on `controller/src/`, `pool/src/`, `common/src/` for `ark_`, `bls12`, `bn254` returns no matches. Contract code therefore cannot directly invoke any ark API; only the host-provided crypto host functions in soroban-env-host can do so on-chain. Not exploitable from contract surface.

### Build-time / proc-macro deps

Resolved proc-macros: `proc-macro2`, `quote`, `syn`, `serde_derive`, `derive_arbitrary`, `bytes-lit`, `num-derive`, `soroban-sdk-macros`, `soroban-env-macros`, `soroban-builtin-sdk-macros`, `stellar-macros`, `cvlr-derive`, `cvlr-macros`, `cvlr-soroban-macros`, `cvlr-soroban-derive`, `visibility`, `ctor`. All pinned through Cargo.lock with checksums. The `dtolnay`-maintained chain (`proc-macro2`/`quote`/`syn`/`paste`) and `serde-rs` are the de-facto Rust ecosystem standards; build-time risk is the universal Rust baseline — not specific to this project. Notable: **CVLR proc-macros load from a non-pinned git repo** (see Cargo.toml `[workspace.dependencies]` entries for `cvlr` and `cvlr-soroban` git URLs without `rev`/`tag`). Today's lockfile freezes `cvlr-soroban` to commit `d2d516a`, but a `cargo update` would float to whatever the upstream branch tip becomes. The on-chain build never enables `--features certora`, so a malicious upstream change cannot reach deployed bytecode — but it could compromise a Certora-machine workstation. Recommend pinning `rev = "d2d516a..."` in the workspace manifest.

### `cargo audit` advisory call

| Advisory | Crate | Reachable from contract? | Verdict |
|---|---|---|---|
| RUSTSEC-2024-0388 (`derivative` unmaintained) | `ark-ec`/`ark-ff`/`ark-poly` | No — host-only | **Safe to accept** |
| RUSTSEC-2024-0436 (`paste` unmaintained) | `wasmi_core` (host VM) and `ark-ff` | No — host-only | **Safe to accept** |
| RUSTSEC-2026-0097 (`rand` 0.8.5 unsound with custom logger) | `soroban-env-host` and `soroban-sdk` testutils | No | **Safe to accept** |

The rand advisory requires a `log` custom logger that calls `rand::rng()` from inside its `log()` impl, plus trace-level logging and a reseed. The protocol installs **no custom log handler**, the `wasm32v1-none` build does not link `rand` at all (verified via `cargo tree`), and even on native test builds the only consumers (`soroban-env-host`, `soroban-sdk` testutils) generate randomness deterministically for snapshot ledgers — not via the affected `thread_rng` reseed path through a logger. The unsound code path is **not reachable** in this project.

### Counts by Risk Factor

| Risk Factor | Dependencies | Total |
|-------------|--------------|-------|
| Floating git pin (no rev/branch) | cvlr, cvlr-soroban (workspace `[workspace.dependencies]`) | 2 |
| Caret-pinned trusted dep (could float on `cargo update`) | soroban-sdk, stellar-access, stellar-macros, stellar-contract-utils | 4 |
| Unmaintained transitive (host-only) | derivative, paste | 2 |
| Unsound transitive (not reachable) | rand 0.8.5 | 1 |
| Young / small-audience trusted dep | stellar-access, stellar-macros, stellar-contract-utils (OZ Stellar contracts, ~79 stars) | 3 |
| Vendored with patch | cvlr (vendor/cvlr/) | 1 |
| **Total flagged** | — | **8** (with overlap) |

### High-Risk Dependencies

| Dependency | Risk Factors | Notes | Suggested Alternative |
|---|---|---|---|
| `cvlr` (git, vendored + patched) | Floating git pin in `[patch]`; vendored copy with 2 textual diffs | Patches confirmed minimal (`#![no_std]` + `core::file!()` swap), zero semantic change. Behind `--features certora`, not in deployed wasm. **Recommend pinning `rev = ...` in workspace.** | **Keep `cvlr` but pin to a commit hash.** Upstream is the only formal-verification toolchain that supports CVL-for-Rust; no functional alternative exists. |
| `cvlr-soroban` (git) | Floating git pin (no rev/branch) | Same as above — `--features certora` only. Lockfile pins `d2d516a` today, but `cargo update` could float. | **Pin `rev = "d2d516a5b27d1926608aa2ce06544dc81c09b435"` in `Cargo.toml`.** |
| `stellar-access` / `stellar-macros` / `stellar-contract-utils` (OZ) | Caret-pinned, young codebase (Dec 2024), only ~79 stars on parent repo, comparatively few external auditors yet | These are *active code* in the deployed contract (auth gates, ownable, pausable, upgradeability). The trust is well-placed (OpenZeppelin is the org), but the stellar-specific port is the newest piece and has the smallest review surface in the dependency tree. | **No replacement** — OZ Stellar is the canonical option. **Mitigation**: pin to `=0.7.0` and add a manual review of the access-control + upgradeable surface to the audit scope (currently listed as "trusted, out of scope" in `audit/SCOPE.md`). |
| `soroban-sdk` (caret `25.3.1`) | Caret-pinned; the cdylib hash is part of on-chain identity, so a transparent `cargo update` to 25.3.x+1 between freeze and deploy would change deployed bytecode | Apache-2.0, organization-owned (Stellar Development Foundation), 183 stars, active (push 2026-04-16), 74 open issues — healthy maintenance posture. | **Tighten to `=25.3.1`** in workspace `Cargo.toml`. |
| `derivative` (transitive, ark-*) | Unmaintained (RUSTSEC-2024-0388) | Host-only, not in `wasm32v1-none` graph. Single-maintainer abandoned. | **No action needed**; cannot be replaced without forking ark-*. |
| `paste` (transitive, wasmi_core + ark-ff) | Unmaintained (RUSTSEC-2024-0436) | Host-only. dtolnay deprecated it; replacement is `pastey`. | **No action needed**; soroban-env-host upstream owns the migration. |
| `rand` 0.8.5 (transitive) | Unsound with custom logger (RUSTSEC-2026-0097) | Required preconditions (custom `log` logger calling `rand::rng()`, trace logging, reseed) do not exist in this project. Not in wasm graph. | **No action needed**; advisory is genuinely irrelevant here. |

## Suggested Alternatives

- For floating git pins on CVLR: pin via `rev = "<sha>"` to the commits the lockfile currently records (`cvlr-soroban` at `d2d516a5b27d1926608aa2ce06544dc81c09b435`). For the patched `cvlr`, the existing path-vendoring approach is appropriate; document the upstream commit it tracks in a `vendor/cvlr/UPSTREAM` file or as a comment in `vendor/cvlr/Cargo.toml`.
- For the four crates.io trusted deps (`soroban-sdk`, `stellar-access`, `stellar-macros`, `stellar-contract-utils`): change caret requirements to `=` requirements during the audit window so `cargo update` cannot silently change deployed bytecode between scope-freeze and deploy.
- The 3 cargo-audit warnings are accurately characterized as accepted in `audit/AUDIT_PREP.md` — no change needed.

## Report Generated By

Supply Chain Risk Auditor Skill
Generated: 2026-04-16
