# Stellar Lending Protocol — Fuzz Tests

Three layers of fuzzing:

1. **Function-level** (this crate, libFuzzer): pure math primitives in `common`.
   Fast (3k–55k exec/s); catches off-by-one, rounding, and overflow bugs.
2. **Contract-level libFuzzer** (this crate, `flow_*.rs` targets): full protocol
   flows via `LendingTest` under coverage-guided mutation. Slower (~5–40 exec/s
   on a dev laptop). Requires `--sanitizer=thread -Zbuild-std` on macOS (see
   below).
3. **Contract-level proptest** (in `test-harness/tests/fuzz_*.rs`): parallel
   property tests via `cargo test`. Easier to run in CI; includes richer
   multi-op sequences the coverage-guided layer doesn't model yet.

### macOS libFuzzer workaround (`--sanitizer=thread`)

The default libFuzzer build (no sanitizer) fails to link on macOS aarch64
because `stellar-access 0.7.x` declares `crate-type = ["lib", "cdylib"]` and the
dylib half doesn't resolve sancov symbols. The Stellar docs recommend building
with TSAN, which sidesteps the cdylib link path. We also need `-Zbuild-std` to
rebuild `std`/`core`/`alloc` with the same sanitizer ABI; otherwise dependencies
emit "mixing `-Zsanitizer` will cause an ABI mismatch" errors.

Setup:

```bash
rustup component add rust-src --toolchain nightly
```

Build + run:

```bash
cd stellar/fuzz
cargo +nightly fuzz build --sanitizer=thread -Zbuild-std
cargo +nightly fuzz run flow_supply_borrow_liquidate \
    --sanitizer=thread -Zbuild-std -- -max_total_time=60
```

On Linux the default (`-sanitizer=none`) build works. TSAN is macOS-specific.

---

## Function-level (libFuzzer)

### Prerequisites

```bash
rustup install nightly
cargo install cargo-fuzz  # once
```

### Running

From `stellar/fuzz/`:

```bash
cargo +nightly fuzz run fp_math -- -max_total_time=30 -sanitizer=none

# All function-level targets via Makefile
cd .. && make fuzz              # 60s each (default)
make fuzz FUZZ_TIME=3600        # 1 hour per target (nightly)

# Triage a crash
cargo +nightly fuzz fmt fp_math artifacts/fp_math/crash-<hash>
```

### Targets

| Target | Function under test | Invariants |
|---|---|---|
| `fp_math` | `mul_div_half_up` / `div_by_int_half_up` / `rescale_half_up` (unified: `kind % 3` dispatch) | per-arm: commutativity+identity+half-up (MulDiv); sign+error bound+f64 diff (DivByInt); roundtrip+sign preservation+away-from-zero (Rescale) |
| `rates_and_index` | pipeline: `calculate_borrow_rate` → `compound_interest` → `calculate_supplier_rewards` | rate: non-negative, max-cap, monotonicity. compound: identity, monotonicity in delta, `≥ 1+r·t` Taylor floor. §5 interest split: `rewards + fee == accrued` (exact), `fee ≈ reserve_factor/BPS × accrued` (half-up), `fee == 0` iff reserve_factor=0 |

### Contract-level libFuzzer targets

Run with `--sanitizer=thread -Zbuild-std` on macOS (see workaround above).

| Target | Flow exercised | Key assertion |
|---|---|---|
| `flow_e2e` | libFuzzer-mutated `Vec<Op>` across three markets (USDC/ETH/XLM) and two borrowers. Ops: Supply, Borrow, Withdraw, Repay, Liquidate, FlashLoan (good/bad receiver), OracleJitter, AdvanceAndSync, ClaimRevenue, CleanBadDebt. | **On success**: HF≥1 for the op-affected user after risk-increasing ops (Borrow/Withdraw); HF>0 otherwise; reserves≥0 across all assets. Bad flash-loan receiver always returns `Err`. **On failure**: reserves + both users' raw supply/borrow balances unchanged (cache-Drop atomicity; subsumes the retired `flow_cache_atomicity` proptest property). |
| `flow_strategy` | libFuzzer-mutated `Vec<Op>` of strategy entrypoints (`multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`) plus `AdvanceAndSync` accrual nudges. Aggregator pre-funded per iteration; ALICE bootstrapped with a baseline position so swap/repay ops reach their target. | HF≥1 after risk-increasing ops (Multiply/SwapDebt/SwapCollateral), HF>0 after repay; reserves≥0 across all assets. **NEW-01 regression — router allowance must be zero** after every successful strategy op (a non-zero residual approval to the aggregator is the high-severity audit finding this target regresses). Cache atomicity on failure. |

Retired targets (2026-04): `flow_supply_borrow_liquidate`, `flow_flash_loan`,
`flow_oracle_tolerance`, and `flow_multi_op` — all subsumed by `flow_e2e`'s
op-sequence fuzzer. Retired proptest harnesses (2026-04):
`fuzz_supply_borrow_liquidate`, `fuzz_oracle_tolerance`, `fuzz_cache_atomicity`,
and `fuzz_isolation_emode_xor` — covered by `flow_e2e`'s Op sequences + the
`isolation_tests.rs` / `emode_tests.rs` unit suites. Their shrunk regression
inputs were all reachable via the bootstrap+Op sequences in `pack_flow_e2e`
(see `fuzz/src/bin/seed_corpus.rs`). Earlier rounds retired
`flow_supply_borrow_tsan_smoke` (link-check only — subsumed by any
contract-level target building under TSAN).

### Input-domain conventions

- Non-negative only for math without signed variants (the protocol's `Ray`/`Wad`/`Bps` are always ≥ 0).
- Magnitudes ≤ 10^27 (RAY domain), ≤ 10^20 when dividing by `BPS`.
- Decimal precisions bounded to `[0, 27]`.
- Values above those bounds produce `MathOverflow` panics — **designed protocol behavior**, not bugs.

---

## Contract-level (proptest)

In `stellar/test-harness/tests/fuzz_*.rs`. Each property test spins up a fresh
`LendingTest` per case and asserts protocol invariants.

### Running

```bash
# All 5 contract-level tests, 256 cases each (default)
make proptest

# Nightly / dedicated-hardware run with 10_000 cases per test (~30 min on M1)
make proptest PROPTEST_CASES=10000

# Single test with custom count
make proptest-one TEST=fuzz_supply_borrow_liquidate PROPTEST_CASES=100000

# Reproduce a failure: proptest saves seeds in test-harness/tests/fuzz_*.proptest-regressions
PROPTEST_CASES=1 cargo test --release -p test-harness --test fuzz_supply_borrow_liquidate
```

### Targets

| Test file | Invariants |
|---|---|
| `fuzz_multi_asset_solvency` | 5–15 random ops across 3 assets / 2 users; all global invariants hold after every step |
| `fuzz_conservation` | Accounting conservation — reserves + borrowed ≥ supplied; Σuser_borrow ≈ pool_borrowed; Σuser_supply + revenue ≈ pool_supplied; reserves ≥ 0 strictly (per step, 5–15 random ops) |
| `fuzz_auth_matrix` | Every privileged controller endpoint (only_owner / only_role) rejects unauthenticated callers; KEEPER role cannot call REVENUE/ORACLE endpoints. Regression gate for audit bug C-01 (`edit_e_mode_category` missing `#[only_owner]`) |
| `fuzz_ttl_keepalive` | `keepalive_accounts` / `keepalive_shared_state` / `keepalive_pools` actually extend the TTL of every expected `ControllerKey` (AccountMeta, SupplyPosition, Market, IsolatedDebt, pool instance). Includes an M-14 regression property: no orphan `SupplyPosition` remains after a full withdraw |
| `fuzz_budget_metering` | Runs with Soroban's *default* budget + resource limits (via `LendingTestBuilder::with_budget_enabled()`). `keepalive_accounts` batches (1-50) and `multiply` at realistic leverage either succeed or fail with a clean budget error -- never with an opaque panic |
| `fuzz_strategy_flashloan` | Strategy (leverage) + flash-loan happy path. `multiply` keeps HF ≥ 1 WAD and zeroes the router allowance (NEW-01 regression). `swap_collateral` uses the actual withdrawal delta (M-11) and rejects `amount_out_min == 0` (M-10). Flash-loan round-trip success property is `#[ignore]` pending SDK-level auth for nested SAC mint (see below) |
| `fuzz_liquidation_differential` | **Differential** harness. Snapshots a random underwater position, runs the full liquidation chain through both the production i128 half-up pipeline and an exact `num_rational::BigRational` reference (`test-harness/src/reference/liquidation.rs`). Asserts that total debt reduction, collateral seizure, and per-asset seizure agree within ≤ 10 ulp absolute in USD-WAD space or 1e-9 relative for large values. Catches accumulated precision drift across the 6-stage chain (HF → bonus → ideal repayment → seizure → protocol fee) that unit tests can't enumerate. |

### Explicit auth trees

A few harnesses must authorize *nested* contract-to-contract calls that
`env.mock_all_auths()` cannot reach — notably the good flash-loan receiver's
nested `token.mint()` inside `execute_flash_loan`. These harnesses opt out of
the blanket auth mock via `LendingTest::new().without_auto_auth()` and build
per-call `MockAuth` trees through the helpers in `test-harness/src/auth.rs`
(`flash_loan_args`, `multiply_args`, `swap_collateral_args`, etc.).

Soroban's recording mode currently cannot authorize the Stellar Asset
Contract admin's `mint` sub-invoke four frames deep (caller → controller →
pool → receiver → SAC). The `prop_flash_loan_success_repayment` property
therefore stays `#[ignore]`-gated with a `real finding` marker. Once the SDK
surface stabilizes, flipping the ignore attribute arms the regression check.

### Differential testing vs `num_rational::BigRational`

`fuzz_liquidation_differential` is the only proptest in the table that runs the
protocol side-by-side with an independent reference implementation. The
reference lives in `test-harness/src/reference/liquidation.rs` and mirrors the
liquidation math (HF, dynamic bonus, ideal-repayment solver, proportional
seizure, protocol fee split) using exact-arithmetic `BigRational`. Production
uses i128 with half-up rounding, so each stage drifts by ≤ 0.5 ulp from the
exact value. Over the ~6-stage chain that's ≤ 3 ulp worst case; the harness
allows 10 ulp absolute in USD-WAD space and ≤ 1e-9 relative for large
reductions. Any sustained drift beyond those bounds is a finding to
investigate.

Scope follows the plan's boundary: liquidation arithmetic only. Bad-debt
socialization (pool-level `supply_index` writes) and rate accrual / compound
interest are **not** differentiated here — they would require full pool-state
emulation in rational space.

### What we do NOT assert (on purpose)

- "Liquidation always improves HF" — false for heavily underwater positions (`HF < 1 + bonus`), where partial liquidations mathematically degrade HF even though bad debt exposure shrinks. The real invariant is `HF > 0` and total debt decreases.
- "Borrow succeeds when LTV is within bounds" — validation can reject for many correct reasons (caps, stale prices, tolerance). We only assert that if it succeeds, HF ≥ 1.

---

## Running on dedicated hardware (Hetzner)

Recommended campaign:

```bash
# Function-level: all 5 targets × 1 hour each (sequential)
make fuzz FUZZ_TIME=3600

# Contract-level: 100k cases per test × 5 tests
make proptest PROPTEST_CASES=100000

# Or run both concurrently on a many-core box
make fuzz FUZZ_TIME=3600 &
make proptest PROPTEST_CASES=100000 &
wait
```

Expected runtime on a Hetzner AX41 (AMD Ryzen 5 5600, 6 cores):
- Function-level × 1h each: ~5 hours wall time
- Contract-level 100k cases: ~4 hours total
- Both together: ~5 hours wall time (different cores)

### Environment tips

- `proptest` saves triggering seeds in `*.proptest-regressions` files — commit these as permanent regression tests.
- `cargo-fuzz` saves `artifacts/<target>/crash-<hash>` — minimize with `cargo fuzz tmin` before committing.
- Both layers run single-threaded (`--test-threads=1` for proptest, libFuzzer is inherently per-process). Parallelize across targets, not within one.

---

## Corpus and regressions

The function-level corpus is gitignored. libFuzzer persists interesting inputs
once a campaign is running, but starting from an empty corpus wastes the first
~10k iterations rediscovering trivial inputs.

**Seed from snapshots before every campaign:**

```bash
make fuzz-seed-corpus
```

This runs `fuzz/src/bin/seed_corpus.rs`, which walks every
`*/test_snapshots/**/*.json`, extracts `MarketParams` / `MarketState` /
position numeric fields from `ledger_entries.contract_data`, and packs them
into the byte layout each fuzz target's `Arbitrary` input struct expects.
Output lands in `fuzz/corpus/<target>/<sha256-prefix>`. Re-running is
idempotent (hash-keyed filenames; duplicates skip). Typical yield on a clean
checkout: ~18k seed files across 11 targets from ~1.4k snapshots.

Empirically, `fp_math` starts a seeded campaign well ahead of an empty
baseline because the packer emits seeds across all three arms (MulDiv /
DivByInt / Rescale) from the same extracted numeric pool — libFuzzer
cross-pollinates bytes between arms during mutation.

To add a regression from a historical bug: drop the minimized `crash-*` file
into `corpus/<target>/` (any location works — libFuzzer reads the whole dir).

Contract-level regressions live in `test-harness/tests/*.proptest-regressions`
(auto-created by proptest on failure). **Commit these files** — they act as
permanent regression tests that re-run on every property-test invocation.

---

## Coverage (fast: corpus replay only)

`cargo fuzz coverage` builds each target with profile instrumentation and
replays the existing corpus once — it does **not** run active fuzzing, so
wall-clock time is dominated by the build, not the replay. HTML reports land
in `target/coverage/fuzz/<target>/index.html`.

Prereqs (one-time):

```bash
rustup component add llvm-tools-preview --toolchain nightly
# optional, prettier function names:
cargo install rustfilt
```

Run:

```bash
# Fast: function-level only (fp_math, rates_and_index)
make fuzz-coverage

# Grow the corpus first with a short fuzz run (30s per target), then measure
make fuzz-coverage FUZZ_COV_TIME=30

# Include contract-level targets (macOS: TSAN build on first run)
make fuzz-coverage-all

# Single target
make fuzz-coverage-one TARGET=flow_e2e FUZZ_COV_TIME=60
```

Filter: harness code (`fuzz/fuzz_targets`, `fuzz/src`), test-harness tests,
stdlib, and `.cargo/registry` deps are excluded from reports — what's left is
the protocol surface (`controller/`, `pool/`, `common/`, `test-harness/src/`).

Use the coverage output to identify underexercised op paths (e.g. whether
`flow_e2e`'s `CleanBadDebt` or `OracleJitter` branches are hit), then either
expand the corpus via `make fuzz-seed-corpus` + a longer campaign, or add
dictionary entries for magic values that block the mutator.

---

## Static UB checking via Miri

Miri is the MIR interpreter for Rust. It detects undefined behavior
(out-of-bounds, invalid alignment, uninitialized reads, invalid provenance,
integer overflow semantics) that escapes both the type system and libFuzzer's
coverage-guided exploration.

**Why:** catches sign-handling edge cases and integer-overflow semantics in the
half-up rounding primitives beyond what libFuzzer's coverage-guided mutation
explores. These functions had sign bugs historically (H-01, H-02) — the two
most security-critical pure-math entry points.

**Scope:** only the pure-i128 subset of `common/src/fp_core.rs`:
`rescale_half_up` and `div_by_int_half_up`. Everything else in `common/`,
`pool/`, and `controller/` routes through Soroban's `I256` host object, which
is backed by FFI into `soroban-env-host` and cannot run under Miri's
interpreter. The filter passed to `cargo miri test` restricts the run to the 8
`#[test]` functions in `fp_core::tests` that don't touch `Env`.

**Prerequisite (one-time):**

```bash
rustup +nightly component add miri rust-src
```

**Run:**

```bash
make miri-common
```

Cold-cache first run takes ~30s on top of the sysroot build (which itself runs
once per toolchain version). Subsequent runs take ~1–2s.

**When to run:** before releases, or after any change to
`common/src/fp_core.rs`. Also gated as a required check on PRs via
`.github/workflows/stellar-fuzz.yml` (`miri-common` job).
