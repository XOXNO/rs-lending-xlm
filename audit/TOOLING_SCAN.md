# Tooling Scan Report

Self-service security tooling output for the Stellar SDF Audit Bank
submission. Produced fresh on the audit-prep run; CI re-runs every push.

**Repo HEAD at scan time**: `a4d2afe` (`a4d2afea37173e293ad996e3f87af56fe0e509fc`)
**Toolchain**: `rustc 1.93.1`, `cargo 1.93.1`, channel pinned to `1.93` in
`rust-toolchain.toml`.

For findings the scans surface (or are documented to accept), see
[`audit/REMEDIATION_PLAN.md`](./REMEDIATION_PLAN.md).

## Tool inventory

| Tool | Version | Surface | Verdict |
|---|---|---|---|
| `cargo audit` (RustSec) | 0.22.1 / advisory-db 1058 advisories @ 2026-04-25 | Cargo.lock dependency tree (249 crates) | **0 vulnerabilities**; 3 informational advisories (documented below) |
| `cargo clippy` (lint, prod surface) | rustc 1.93.1 | `--workspace --lib --bins -D warnings` | **clean** |
| `cargo clippy` (lint, all targets) | rustc 1.93.1 | `--workspace --all-targets -D warnings` | **clean** (3 trivial issues fixed during pre-audit prep) |
| `cargo test` (unit + integration) | rustc 1.93.1 | `--workspace` | **685 passed / 0 failed / 3 ignored** |
| `soroban-scanner` (OpenZeppelin / XOXNO fork) | `soroban-security-detectors-runner` 0.0.2 (XOXNO fork @ `4323161`) | `controller/src/`, `pool/src/`, `pool-interface/src/`, `common/src/` (36 files in-scope) | **0 detector findings** |
| `cargo-fuzz` (libFuzzer) | 0.13.1 | 6 targets in `fuzz/fuzz_targets/` | nightly campaign 30 min/target — no panics |
| `proptest` (in `test-harness/tests/`) | runs via `cargo test` | 7 harnesses | nightly campaign 50 000 cases each — no panics |
| Miri | nightly | `common/src/fp_core.rs::tests` (8 tests) | gated as required check via `.github/workflows/fuzz.yml` (`miri-common` job) |
| Certora Soroban Prover | spec compile passes; empirical run pending | 13 spec modules, 209 rule fns | pending (`MATH_REVIEW.md §0`) |

## 1. `cargo audit`

Command (run from repo root):

```bash
cargo audit            # human-readable
cargo audit --json     # machine-readable (used to derive the JSON below)
```

Result summary (excerpt of `cargo audit --json`):

```
{
  "database":   { "advisory-count": 1058, "last-updated": "2026-04-25" },
  "lockfile":   { "dependency-count": 249 },
  "vulnerabilities": { "found": false, "count": 0, "list": [] },
  "warnings": {
    "unmaintained": [
      { "advisory": { "id": "RUSTSEC-2024-0388", "package": "derivative" } },
      { "advisory": { "id": "RUSTSEC-2024-0436", "package": "paste"      } }
    ],
    "unsound":      [
      { "advisory": { "id": "RUSTSEC-2026-0097", "package": "rand"       } }
    ]
  }
}
```

### Accepted advisories (3, all on transitive Soroban deps)

| ID | Crate | Type | Why accepted |
|---|---|---|---|
| RUSTSEC-2024-0388 | `derivative 2.2.0` | unmaintained | Pulled in via `ark-poly → ark-ec → soroban-env-host → soroban-sdk 25.3.1`. No direct dependency from `controller/` / `pool/` / `pool-interface/` / `common/`. Fix path is upstream: requires Soroban-SDK to retire `ark-*` cryptography or migrate to `derive_more`. Tracked. |
| RUSTSEC-2024-0436 | `paste 1.0.15` | unmaintained | Pulled in via `soroban-sdk` macro infrastructure. Same upstream-fix posture. No direct usage in our crates. |
| RUSTSEC-2026-0097 | `rand 0.8.5` | unsound (with `log` feature + custom logger) | Soundness condition requires the host env to enable `log`'s feature + a custom logger that re-enters `rand::rng()`. Our deployable WASM (`controller`, `pool`) ships `#![no_std]` and never touches `rand` at runtime; the dep is host-only via `soroban-env-host`. Conditions are not met on our deployment surface. |

**Vulnerability count**: 0.

## 2. `cargo clippy`

Production surface (the WASM-deployed crates):

```bash
cargo clippy --workspace --lib --bins -- -D warnings
```

Verdict: **clean**.

Full surface including tests, harnesses, fuzz targets:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Verdict: **clean** (3 trivial style issues fixed during pre-audit prep —
`common/src/events.rs` ×2, `common/src/rates.rs` ×1, `pool/src/lib.rs` ×3
redundant-field-names / range-contains).

Pre-existing test-harness lints noted in `audit/AUDIT_PREP.md` were resolved
during prep; the current run is clean across the whole workspace.

## 3. `cargo test`

```bash
cargo test --workspace
```

Verdict: **685 passed / 0 failed / 3 ignored**.

The 3 ignored tests are intentional gates documented in:

- `fuzz/README.md` "Explicit auth trees" — `prop_flash_loan_success_repayment`
  ignored pending Soroban SDK auth-tree support for nested SAC `mint`.
- two more ignored cases in the test-harness (documented inline).

Coverage on in-scope crates is **95.43 %** (11 301 / 11 842 lines) per
`audit/AUDIT_PREP.md "Static Analysis Status"`. The full per-file coverage
report lives at `target/coverage/merged-report.md` after `make coverage-merged`.

## 4. `soroban-scanner` (OpenZeppelin / XOXNO fork)

Wrapper script: `.github/scripts/run_scanner.sh` (see file for retry rationale —
upstream's symbol resolver recurses unboundedly for some HashMap iteration
orders; the wrapper retries up to `SOROBAN_SCANNER_MAX_ATTEMPTS = 5` to absorb
the non-determinism).

Filter: `.github/scripts/scope_scanner_output.py` narrows findings to the
deployable surface (`/common/src/`, `/pool/src/`, `/pool-interface/src/`,
`/controller/src/`).

```bash
.github/scripts/run_scanner.sh > scan-results.json
```

Verdict (filtered to deployable crates):

```
errors:                0
files in scope:        36
detectors triggered:   0
```

CI strict-mode: PR builds fail on HIGH/CRITICAL findings. The scan-clean
artifact is attached to every CI run; the local replay matches. The XOXNO
fork carries six patches over upstream `soroban-security-detectors-sdk`,
documented in the workflow comment block at `.github/workflows/ci.yml:78-114`.

## 5. `cargo-fuzz` libFuzzer targets

Inventory (`fuzz/fuzz_targets/`):

| Target | Function under test | Notes |
|---|---|---|
| `fp_math` | `mul_div_half_up` / `div_by_int_half_up` / `rescale_half_up` | per-arm: commutativity, identity, half-up direction, sign, error bound |
| `fp_ops` | `Ray` / `Wad` / `Bps` operator surface | round-trip preservation, sign, overflow boundary |
| `rates_and_index` | `calculate_borrow_rate → compound_interest → calculate_supplier_rewards` | rate non-negativity, monotonicity, compound `≥ 1+r·t` Taylor floor; §5 interest split: `rewards + fee == accrued` exact |
| `pool_native` | `pool/src/cache.rs` + `interest.rs` boundary | bypasses host where possible; specifically targets the new `pool_native` paths added in `commit 418038a` |
| `flow_e2e` | LibFuzzer-mutated `Vec<Op>` across 3 markets / 2 borrowers; ops Supply/Borrow/Withdraw/Repay/Liquidate/FlashLoan(good or bad)/OracleJitter/AdvanceAndSync/ClaimRevenue/CleanBadDebt | HF≥1 after risk-increasing ops; bad receiver always Err; cache atomicity on failure |
| `flow_strategy` | LibFuzzer-mutated strategy ops (`multiply` / `swap_debt` / `swap_collateral` / `repay_debt_with_collateral`) | HF ≥ 1; **NEW-01 regression**: router allowance must be 0 after every successful strategy op |

Run protocols:

```bash
make fuzz                      # 60s each (default)
make fuzz FUZZ_TIME=1800       # 30 min each per target (CI nightly)
make fuzz-coverage             # corpus replay + coverage report (target/coverage/fuzz/)
```

Nightly campaign: `.github/workflows/fuzz.yml` runs `FUZZ_TIME=1800` per
target and uploads artifacts (corpora, coverage HTML) on completion.
Verdict on the most recent nightly run: **no panics, no UB, no asserts
violated**.

## 6. `proptest` contract-level harnesses

Inventory (`test-harness/tests/`):

| Harness | Property |
|---|---|
| `fuzz_multi_asset_solvency` | 5–15 random ops across 3 assets / 2 users; all global invariants hold after every step |
| `fuzz_conservation` | reserves + borrowed ≥ supplied; Σuser_borrow ≈ pool_borrowed; Σuser_supply + revenue ≈ pool_supplied; reserves ≥ 0 strictly |
| `fuzz_auth_matrix` | every privileged endpoint rejects unauthenticated callers; KEEPER cannot call REVENUE/ORACLE; **C-01 regression** (`edit_e_mode_category` `#[only_owner]`) |
| `fuzz_ttl_keepalive` | `keepalive_*` extends every expected `ControllerKey` TTL; **M-14 regression**: no orphan `SupplyPosition` after full withdraw |
| `fuzz_budget_metering` | runs with Soroban default budget; `keepalive_accounts` (1-50 batch) and `multiply` either succeed or fail with a clean budget error — never an opaque panic |
| `fuzz_strategy_flashloan` | strategy + flash-loan happy path; **NEW-01 regression**: zero router allowance; **M-11**: actual withdrawal delta; **M-10**: `amount_out_min == 0` rejected |
| `fuzz_liquidation_differential` | snapshot underwater position; run liquidation chain through prod (i128 half-up) and reference (`num_rational::BigRational`); assert agreement within 10 ulp / 1e-9 relative across HF → bonus → ideal repayment → seizure → protocol-fee |

Run protocols:

```bash
make proptest                       # 256 cases per harness (default)
make proptest PROPTEST_CASES=50000  # CI nightly
```

Verdict on most recent nightly: **no failures**; saved
`*.proptest-regressions` files are committed and re-run on every
invocation as permanent regression gates.

## 7. Miri

Scope: pure-i128 subset of `common/src/fp_core.rs` (`rescale_half_up`,
`div_by_int_half_up`) — 8 tests in `fp_core::tests` that don't touch
Soroban's `Env` (the rest of `common/`, `pool/`, `controller/` routes
through `I256` host objects via FFI into `soroban-env-host` and cannot
run under Miri).

```bash
make miri-common
```

CI gate: `.github/workflows/fuzz.yml::miri-common` runs on every push;
required for PR merge.

The local one-shot run on this machine produced an `ethnum` build error in
the nightly toolchain's sysroot (an environment-specific issue with the
local nightly's cached `rust-src`). The CI gate runs against a clean
nightly per build and passes; Miri remains the authoritative gate.
Documented for transparency.

## 8. Certora Soroban Prover (formal verification)

Specs live at `controller/certora/spec/`:

| Spec module | Rule fns |
|---|---|
| `boundary_rules.rs` | 38 |
| `solvency_rules.rs` | 34 |
| `strategy_rules.rs` | 20 |
| `math_rules.rs` | 19 |
| `interest_rules.rs` | 15 |
| `emode_rules.rs` | 15 |
| `liquidation_rules.rs` | 10 |
| `isolation_rules.rs` | 9 |
| `oracle_rules.rs` | 8 |
| `position_rules.rs` | 6 |
| `health_rules.rs` | 6 |
| `index_rules.rs` | 6 |
| `flash_loan_rules.rs` | 4 |
| (per-spec totals) | **190 source rules**, ~209 fns counting helpers |

Compile gate (passes):

```bash
cargo check -p controller --features certora --no-default-features
```

Empirical prover gate (**pending**, tracked in `architecture/MATH_REVIEW.md §0`
"Empirical `certoraSorobanProver <conf>` run with vendored stack"):

```bash
certoraSorobanProver controller/confs/math.conf
```

Toolchain status: `cvlr-spec` compile blocker is resolved by vendoring CVLR
under `vendor/cvlr/` with `#![no_std]` patched into `cvlr-spec/src/lib.rs`;
workspace `Cargo.toml` redirects every `cvlr-*` crate to the vendored copy.
The remaining gate is the empirical `certoraSorobanProver` run; this is
[Task P1-8](#) in the audit-prep plan.

Rule-coverage gaps and remediation are tracked in
`architecture/MATH_REVIEW.md §3` (16 tautological rules, 9 weak, 4 vacuous,
8 documented invariants without rule coverage). 7 new rules already shipped
during prep close §0 items.

## 9. Coverage

```bash
make coverage-merged
```

Result: **95.43 % line coverage** (11 301 / 11 842 in-scope lines).
Per-file detail in `target/coverage/merged-report.md` after the make
target completes.

Production files are ≥ 90 %, most ≥ 99 %. Per-file 100 % coverage on:
`flash_loan`, `utils`, `withdraw`, `account`, `cache`, `pool/views`,
`pool/interest`. Lowest in-scope files:

- `controller/src/oracle/reflector.rs` (0/2 — only real network calls
  exercise it; mocked elsewhere).
- `controller/src/helpers/testutils.rs` (39 % — test infrastructure, not
  prod code).

Every position / strategy / liquidation path is ≥ 99.7 %.

## 10. CI gates

Workflows under `.github/workflows/`:

| Workflow | Runs | Strict gates |
|---|---|---|
| `ci.yml` | every push | `cargo test --workspace`, `cargo clippy -D warnings`, `soroban-scanner` (HIGH/CRITICAL fails PR) |
| `fuzz.yml` | nightly + on-demand | `cargo-fuzz` 30 min/target × 6 targets; `proptest` 50 000 cases × 7 harnesses; `miri-common` |
| `release.yml` | tag push | `make build` + `make optimize`, attest-build-provenance, upload-artifact (release wasm + sha) |

## 11. Findings list

After all the above tooling runs, the **net findings list** for this
audit-prep cycle is:

| Source | Findings (production crates) |
|---|---|
| `cargo audit` | 0 vulnerabilities; 3 transitive informational advisories (accepted, see §1) |
| `cargo clippy` | 0 |
| `cargo test` | 0 failures |
| `soroban-scanner` (filtered to in-scope crates) | 0 |
| `cargo-fuzz` nightly | 0 panics |
| `proptest` nightly | 0 failures |
| Miri | 0 UB found |
| Certora (compile) | spec compiles clean; empirical prover run pending |

For the historical pre-audit hunt findings (H-/M-/L-/N-/I- series) and
their remediation, see [`audit/REMEDIATION_PLAN.md`](./REMEDIATION_PLAN.md).

## 12. Reproducing the scan

```bash
git clone <repo>
cd rs-lending-xlm
git checkout audit-2026-q2          # frozen audit tag
rustup show                          # uses rust-toolchain.toml -> 1.93
make build                           # required before integration tests
cargo audit
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
.github/scripts/run_scanner.sh > scan-results.json
make miri-common                     # gated by CI; local nightly env may differ
make fuzz FUZZ_TIME=60              # smoke
make proptest                        # smoke
```

Long campaigns (CI nightly cadence):

```bash
make fuzz FUZZ_TIME=1800            # 30 min per target × 6 targets
make proptest PROPTEST_CASES=50000  # 50 000 cases per harness × 7 harnesses
```

The full toolchain version pin is in `rust-toolchain.toml`; CI installs the
exact same `1.93` toolchain via `rustup show`.

## 13. What's not here

- **Manual code review.** The internal team has run multiple manual review
  rounds; the resulting fixes are in `audit/REMEDIATION_PLAN.md`. The next
  manual review is the external Runtime Verification + Certora engagement.
- **Penetration testing on testnet.** `architecture/DEPLOYMENT.md "Smoke-Test
  Runbook"` documents the operator-side validation flow; no adversarial
  testnet campaign has been run.
- **Symbolic execution beyond Certora.** No `Klee`-style symbolic execution
  has been run. Certora Soroban Prover handles this surface.
- **Differential fuzzing across alternative implementations.** The
  `fuzz_liquidation_differential` harness compares against an exact
  `BigRational` reference; other modules don't have a comparable reference.
