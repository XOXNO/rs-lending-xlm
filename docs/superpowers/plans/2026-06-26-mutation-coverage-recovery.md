# Mutation-Testing Coverage Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Raise the `make mutants` kill rate on `common` + controller helpers from 61% to ~95%+ by fixing the target's test scope, adding fast unit tests for pure helpers, and suppressing provably-equivalent mutants.

**Architecture:** Three tracks. (C) suppress 6 equivalent math mutants in `.cargo/mutants.toml`. (B) add co-located unit tests for pure/near-pure helpers (`common` constants, validation guards, oracle observation, controller payment aggregation). (A) extend the default `mutants` Makefile target to run `common`, `controller`, and `test-harness` test packages so harness flows kill the `Cache`/storage/wasm-dependent mutants.

**Tech Stack:** Rust, `soroban-sdk` (testutils), `cargo-mutants`, GNU Make. Tests are co-located via `#[cfg(test)] #[path = "..."] mod tests;` with `autotests = false` (private access required).

## Global Constraints

- Verification bar: `cargo check --workspace` and `cargo clippy --workspace -- -D warnings` clean. **Never** use `--all-features` (enables `certora`, breaks linking).
- Test packages compile under the default (non-`certora`) feature set.
- Co-located test wiring pattern is mandatory: a source file at `<dir>/x.rs` adds `#[cfg(test)] #[path = "<rel>/tests/.../x.rs"] mod tests;` and the test file begins with `use super::*;`.
- `rustfmt` directly on touched files (per repo convention); run `cargo fmt -p <pkg>` only if it does not reformat cfg-gated `#[path]` modules.
- Commit style: Conventional Commits, imperative subject ≤72 chars.
- Pool wasm fixture must exist before any harness run: `make build` (writes `target/wasm32v1-none/release/pool.wasm`).

---

### Task 1: Track C — suppress provably-equivalent math mutants

**Files:**
- Modify: `.cargo/mutants.toml` (append to existing `exclude_re` array)

**Interfaces:**
- Produces: a cleaner mutant denominator. No code symbols.

These six mutants cannot be killed by any test: `Wad::min`/`Wad::max` return the same value at equality; the three `rescale_*` functions early-`return a` when `from_decimals == to_decimals`, making `>` vs `>=` unobservable; `mul_div_half_up_signed` truncates to `0` at `product == 0` for both rounding directions.

- [ ] **Step 1: Add the six exclusions to `.cargo/mutants.toml`**

The file's `exclude_re` array currently contains one entry. Replace the array with:

```toml
exclude_re = [
    # `replace .* with Default::default()` on unit-returning fns.
    "replace .* -> \\(\\) with \\(\\)",

    # Provably-equivalent boundary mutants (equality pre-handled or symmetric at 0):
    "replace < with <= in Wad::min",
    "replace > with >= in Wad::max",
    "replace > with >= in rescale_half_up",
    "replace > with >= in rescale_floor",
    "replace > with >= in rescale_ceil",
    "replace < with <= in mul_div_half_up_signed",
]
```

- [ ] **Step 2: Verify the mutants disappear from the candidate list**

Run: `cargo mutants --package common --file 'common/src/math/fp.rs' --file 'common/src/math/fp_core.rs' --list`
Expected: the output does **not** contain `Wad::min`, `Wad::max`, `rescale_half_up`, `rescale_floor`, `rescale_ceil`, or `mul_div_half_up_signed` boundary lines listed above. (Other math mutants may still appear.)

- [ ] **Step 3: Commit**

```bash
git add .cargo/mutants.toml
git commit -m "test(mutants): suppress provably-equivalent fp boundary mutants"
```

---

### Task 2: Track B — pin protocol constants with a value test

**Files:**
- Create: `common/tests/constants.rs`
- Modify: `common/src/constants/mod.rs` (append test wiring)

**Interfaces:**
- Consumes: `RAY`, `WAD`, `BPS`, `MAX_REASONABLE_PRICE_WAD`, `DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD`, `TTL_THRESHOLD_INSTANCE`, `TTL_BUMP_INSTANCE`, `TTL_THRESHOLD_SHARED`, `TTL_BUMP_SHARED`, `TTL_THRESHOLD_USER`, `TTL_BUMP_USER`, `MAX_BORROW_RATE_RAY` — all re-exported from `common::constants`.

- [ ] **Step 1: Write the constants value test**

Create `common/tests/constants.rs`:

```rust
use super::*;

#[test]
fn fixed_point_scales() {
    assert_eq!(RAY, 1_000_000_000_000_000_000_000_000_000);
    assert_eq!(WAD, 1_000_000_000_000_000_000);
    assert_eq!(BPS, 10_000);
}

#[test]
fn derived_usd_bounds() {
    // 1e9 * WAD
    assert_eq!(MAX_REASONABLE_PRICE_WAD, 1_000_000_000_000_000_000_000_000_000);
    // 5 * WAD
    assert_eq!(DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD, 5_000_000_000_000_000_000);
}

#[test]
fn ttl_ledger_counts() {
    // ONE_DAY_LEDGERS (17_280) * day spans.
    assert_eq!(TTL_THRESHOLD_INSTANCE, 518_400); // * 30
    assert_eq!(TTL_BUMP_INSTANCE, 3_110_400); // * 180
    assert_eq!(TTL_THRESHOLD_SHARED, 518_400); // * 30
    assert_eq!(TTL_BUMP_SHARED, 3_110_400); // * 180
    assert_eq!(TTL_THRESHOLD_USER, 518_400); // * 30
    assert_eq!(TTL_BUMP_USER, 2_073_600); // * 120
}

#[test]
fn max_borrow_rate_is_two_ray() {
    // 2 * RAY
    assert_eq!(MAX_BORROW_RATE_RAY, 2_000_000_000_000_000_000_000_000_000);
}
```

- [ ] **Step 2: Wire the test into the crate**

Append to `common/src/constants/mod.rs`:

```rust

#[cfg(test)]
#[path = "../../tests/constants.rs"]
mod tests;
```

- [ ] **Step 3: Run the test, expect pass**

Run: `cargo test -p common constants`
Expected: PASS (4 tests).

- [ ] **Step 4: Verify the constant-arithmetic mutants are now caught**

Run: `cargo mutants --package common --file 'common/src/constants/shared.rs' --file 'common/src/constants/pool.rs'`
Expected: `0 missed` for the `* with +` / `* with /` mutants on the pinned constants (any unpinned residual is listed and triaged in Task 7).

- [ ] **Step 5: Commit**

```bash
git add common/tests/constants.rs common/src/constants/mod.rs
git commit -m "test(common): pin protocol constant values to kill arithmetic mutants"
```

---

### Task 3: Track B — cover the four untested validation guards

**Files:**
- Modify: `common/tests/validation.rs` (append tests + imports)

**Interfaces:**
- Consumes: `require_positive_amount(&Env, i128)`, `require_nonneg_amount(&Env, i128)`, `cap_is_enabled(i128) -> bool`, `require_wasm_receiver(&Env, &Address)` — all from `common::validation` (in scope via existing `use super::*;`).

`common/tests/validation.rs` already exists and starts with `use super::*; use soroban_sdk::Env;`. Append to it.

- [ ] **Step 1: Write the guard tests**

Append to `common/tests/validation.rs`:

```rust
use soroban_sdk::{contract, contractimpl, Address};
use soroban_sdk::testutils::Address as _;

#[contract]
struct WasmReceiver;

#[contractimpl]
impl WasmReceiver {}

#[test]
fn require_positive_accepts_one() {
    let env = Env::default();
    require_positive_amount(&env, 1);
}

#[test]
#[should_panic]
fn require_positive_rejects_zero() {
    let env = Env::default();
    require_positive_amount(&env, 0);
}

#[test]
fn require_nonneg_accepts_zero() {
    let env = Env::default();
    require_nonneg_amount(&env, 0);
}

#[test]
#[should_panic]
fn require_nonneg_rejects_negative() {
    let env = Env::default();
    require_nonneg_amount(&env, -1);
}

#[test]
fn cap_is_enabled_truth_table() {
    assert!(!cap_is_enabled(0));
    assert!(!cap_is_enabled(-1));
    assert!(!cap_is_enabled(i128::MAX));
    assert!(cap_is_enabled(1));
    assert!(cap_is_enabled(1_000_000));
}

#[test]
fn require_wasm_receiver_accepts_contract() {
    let env = Env::default();
    let receiver = env.register(WasmReceiver, ());
    require_wasm_receiver(&env, &receiver);
}

#[test]
#[should_panic]
fn require_wasm_receiver_rejects_account() {
    let env = Env::default();
    let account = Address::generate(&env);
    require_wasm_receiver(&env, &account);
}
```

- [ ] **Step 2: Run the tests, expect pass**

Run: `cargo test -p common validation`
Expected: PASS (existing tests + 8 new).

- [ ] **Step 3: Verify validation mutants are caught**

Run: `cargo mutants --package common --file 'common/src/validation.rs'`
Expected: `0 missed` for `require_positive_amount`, `require_nonneg_amount`, `cap_is_enabled`, `require_wasm_receiver`.

- [ ] **Step 4: Commit**

```bash
git add common/tests/validation.rs
git commit -m "test(common): cover positive/nonneg/cap/wasm-receiver guards"
```

---

### Task 4: Track B — cover oracle observation guards

**Files:**
- Create: `common/tests/oracle/observation.rs`
- Modify: `common/src/oracle/observation.rs` (append test wiring)

**Interfaces:**
- Consumes (in scope via `use super::*;` on the observation module): `is_stale(u64,u64,u64)->bool`, `check_not_future_at(&Env,u64,u64)`, `validate_timestamp(&Env,u64,u64,u64)` (private), `normalize_positive_price(&Env,i128,u32)->i128`, `validate_positive_price_timestamps(&Env,i128,u32,u64,&[u64],u64)->i128`, `u256_to_i128(&Env,&U256)->i128`, `millis_to_seconds(u64)->u64`.

- [ ] **Step 1: Write the observation tests**

Create `common/tests/oracle/observation.rs`:

```rust
use super::*;
use soroban_sdk::{Env, U256};

#[test]
fn is_stale_false_at_exact_max_age() {
    // elapsed == max_stale is fresh (strict `>`).
    assert!(!is_stale(160, 100, 60));
}

#[test]
fn is_stale_true_past_max_age() {
    assert!(is_stale(161, 100, 60));
}

#[test]
fn is_stale_false_when_feed_not_in_past() {
    assert!(!is_stale(100, 100, 60));
    assert!(!is_stale(100, 200, 60));
}

#[test]
fn millis_to_seconds_divides_by_thousand() {
    assert_eq!(millis_to_seconds(1_500), 1);
    assert_eq!(millis_to_seconds(60_000), 60);
}

#[test]
fn normalize_scales_token_to_wad() {
    let env = Env::default();
    // price 1 at 6 decimals -> 1 * 10^(18-6) WAD.
    assert_eq!(normalize_positive_price(&env, 1, 6), 1_000_000_000_000);
}

#[test]
#[should_panic]
fn normalize_rejects_nonpositive() {
    let env = Env::default();
    normalize_positive_price(&env, 0, 6);
}

#[test]
fn u256_to_i128_roundtrips() {
    let env = Env::default();
    let v = U256::from_u128(&env, 12_345);
    assert_eq!(u256_to_i128(&env, &v), 12_345);
}

#[test]
fn validate_timestamp_accepts_fresh() {
    let env = Env::default();
    validate_timestamp(&env, 1_000, 990, 60);
}

#[test]
#[should_panic]
fn validate_timestamp_rejects_stale() {
    let env = Env::default();
    validate_timestamp(&env, 1_000, 800, 60); // elapsed 200 > 60
}

#[test]
#[should_panic]
fn validate_timestamp_rejects_future_skew() {
    let env = Env::default();
    validate_timestamp(&env, 1_000, 1_100, 60); // 100 > MAX_FUTURE_SKEW_SECONDS
}

#[test]
#[should_panic]
fn check_not_future_at_rejects_skew() {
    let env = Env::default();
    check_not_future_at(&env, 1_000, 1_100);
}

#[test]
fn validate_positive_price_timestamps_returns_wad() {
    let env = Env::default();
    let timestamps = [990u64, 995u64];
    let out = validate_positive_price_timestamps(&env, 1, 6, 1_000, &timestamps, 60);
    assert_eq!(out, 1_000_000_000_000);
}
```

- [ ] **Step 2: Wire the test into the observation module**

Append to `common/src/oracle/observation.rs`:

```rust

#[cfg(test)]
#[path = "../../../tests/oracle/observation.rs"]
mod tests;
```

- [ ] **Step 3: Run the tests, expect pass**

Run: `cargo test -p common observation`
Expected: PASS (12 tests).

- [ ] **Step 4: Verify observation mutants are caught**

Run: `cargo mutants --package common --file 'common/src/oracle/observation.rs'`
Expected: a large drop in missed. One known-equivalent residual may remain — `observation.rs:30:14: replace > with >= in is_stale` (at `now == feed_ts`, elapsed is `0`, never `> max_stale`, so unobservable). Record it for Task 7; do not chase it.

- [ ] **Step 5: Commit**

```bash
git add common/tests/oracle/observation.rs common/src/oracle/observation.rs
git commit -m "test(common): cover oracle staleness, skew, and normalization guards"
```

---

### Task 5: Track B — cover controller payment aggregation

**Files:**
- Create: `contracts/controller/tests/helpers/utils.rs`
- Modify: `contracts/controller/src/helpers/utils.rs` (append test wiring)

**Interfaces:**
- Consumes (in scope via `use super::*;` on the utils module): `aggregate_payment_amount(&Env, Option<i128>, i128, bool) -> i128` (private), `push_unique_address(&mut Vec<Address>, Address)`.

This is the first co-located test under `contracts/controller/src/helpers/`. The path from `src/helpers/utils.rs` to `tests/` is `../../tests/...`.

- [ ] **Step 1: Write the aggregation tests**

Create `contracts/controller/tests/helpers/utils.rs`:

```rust
use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Env, Vec};

#[test]
#[should_panic]
fn aggregate_rejects_negative() {
    let env = Env::default();
    aggregate_payment_amount(&env, None, -1, false);
}

#[test]
#[should_panic]
fn aggregate_rejects_zero_when_not_withdraw_all() {
    let env = Env::default();
    aggregate_payment_amount(&env, None, 0, false);
}

#[test]
fn aggregate_zero_is_withdraw_all_sentinel() {
    let env = Env::default();
    assert_eq!(aggregate_payment_amount(&env, None, 0, true), 0);
    assert_eq!(aggregate_payment_amount(&env, Some(0), 5, true), 0);
    assert_eq!(aggregate_payment_amount(&env, None, 5, true), 5);
}

#[test]
fn aggregate_sums_previous_and_amount() {
    let env = Env::default();
    assert_eq!(aggregate_payment_amount(&env, Some(10), 5, false), 15);
    assert_eq!(aggregate_payment_amount(&env, None, 7, false), 7);
    assert_eq!(aggregate_payment_amount(&env, Some(0), 5, false), 5);
}

#[test]
fn push_unique_dedups_preserving_order() {
    let env = Env::default();
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let mut out: Vec<Address> = Vec::new(&env);
    push_unique_address(&mut out, a.clone());
    push_unique_address(&mut out, a.clone());
    push_unique_address(&mut out, b.clone());
    assert_eq!(out.len(), 2);
    assert_eq!(out.get_unchecked(0), a);
    assert_eq!(out.get_unchecked(1), b);
}
```

- [ ] **Step 2: Wire the test into the utils module**

Append to `contracts/controller/src/helpers/utils.rs`:

```rust

#[cfg(test)]
#[path = "../../tests/helpers/utils.rs"]
mod tests;
```

- [ ] **Step 3: Run the tests, expect pass**

Run: `cargo test -p controller utils`
Expected: PASS (5 tests).

- [ ] **Step 4: Verify utils mutants are caught**

Run: `cargo mutants --package controller --file 'contracts/controller/src/helpers/utils.rs'`
Expected: `aggregate_payment_amount` and `push_unique_address` mutants caught. Known-equivalent residual: `utils.rs:81:15: replace < with <= in aggregate_payment_amount` (at `amount == 0` both clauses panic). `transfer_amount` mutants remain (SAC-dependent) — covered by Task 6 (harness). Record residuals for Task 7.

- [ ] **Step 5: Commit**

```bash
git add contracts/controller/tests/helpers/utils.rs contracts/controller/src/helpers/utils.rs
git commit -m "test(controller): cover payment aggregation and address dedup"
```

---

### Task 6: Track A — run the harness against the default mutants target

**Files:**
- Modify: `Makefile` (the `mutants:` target, lines ~433-444)

**Interfaces:**
- Consumes: `MUTANTS_TIMEOUT` (Makefile var, default `120`).
- Produces: a `mutants` target whose test scope includes `common`, `controller`, and `test-harness`.

The controller is registered natively in the harness (`tests/test-harness/src/setup/builder.rs:175`), so controller-source mutations propagate. The harness needs `pool.wasm` present.

- [ ] **Step 1: Add the test-package scope and timeout to the `mutants` target**

In `Makefile`, replace the `cargo mutants ...` command inside the `mutants:` target with:

```makefile
	cargo mutants --package common --package controller \
		--file 'common/src/**/*.rs' \
		--file 'contracts/controller/src/helpers/**/*.rs' \
		--exclude '**/tests/**' \
		--exclude '**/certora/**' \
		--test-package common \
		--test-package controller \
		--test-package test-harness \
		--minimum-test-timeout $(MUTANTS_TIMEOUT) \
		--jobs 1
```

- [ ] **Step 2: Build the pool wasm fixture the harness loads**

Run: `make build`
Expected: completes; `target/wasm32v1-none/release/pool.wasm` exists.

- [ ] **Step 3: Smoke-check the harness is now in scope on a small slice**

Run: `cargo mutants --package controller --file 'contracts/controller/src/helpers/math.rs' --test-package controller --test-package test-harness --minimum-test-timeout 120 --jobs 1`
Expected: the `+= with -=` mutants in `calculate_account_risk_totals_body` are now **caught** (not missed) — proving harness flows propagate controller-source mutations. (Runtime: several minutes; the harness builds once then mutates.)

- [ ] **Step 4: Commit**

```bash
git add Makefile
git commit -m "test(mutants): run common/controller/test-harness against default target"
```

---

### Task 7: Verify — full run, record kill rate, triage residuals

**Files:**
- None (verification + reporting only). Optionally Modify: `.cargo/mutants.toml` if a *new* equivalent mutant is confirmed.

- [ ] **Step 1: Confirm the workspace is clean**

Run: `cargo check --workspace && cargo clippy --workspace -- -D warnings`
Expected: no errors, no warnings.

- [ ] **Step 2: Confirm package tests pass**

Run: `cargo test -p common && cargo test -p controller`
Expected: PASS, no failures.

- [ ] **Step 3: Ensure the pool wasm fixture is current**

Run: `make build`
Expected: completes.

- [ ] **Step 4: Full mutation run**

Run: `make mutants`
Expected: completes (note: substantially longer than the prior 11 min — harness is now in the loop). Capture the final `N mutants tested in …: X missed, Y caught, Z unviable` line.

- [ ] **Step 5: Triage every residual MISSED mutant**

For each remaining `MISSED` line, classify in one sentence: `equivalent` (cannot be killed — e.g. `is_stale:30:14`, `aggregate_payment_amount:81:15`), `accepted` (low-value, e.g. an unpinned constant), or `follow-up` (a genuine gap to file). If any is confirmed `equivalent`, add its precise `path:line:col: replace …` string to `.cargo/mutants.toml` `exclude_re`.

- [ ] **Step 6: Record the outcome in the spec and commit any suppression**

Append a short "Result" note (date, before/after kill rate, residual list) to `docs/superpowers/specs/2026-06-26-mutation-coverage-recovery-design.md`.

```bash
git add docs/superpowers/specs/2026-06-26-mutation-coverage-recovery-design.md .cargo/mutants.toml
git commit -m "docs(mutants): record coverage-recovery result and residual triage"
```

---

## Self-Review

**Spec coverage:**
- Track C (suppress equivalents) → Task 1 (+ Task 7 step 5 for any newly-confirmed equivalent). ✓
- Track C (pin constants) → Task 2. ✓
- Track B (validation guards) → Task 3; (observation) → Task 4; (utils aggregation) → Task 5. ✓
- Track A (Makefile default scope + harness) → Task 6. ✓
- Verification bar + residual enumeration → Task 7. ✓
- Spec's "emode/math/risk_params/account/providers/rates left to Track A" → covered by Task 6's harness scope; Task 6 step 3 proves propagation on `math.rs`. ✓

**Placeholder scan:** No TBD/TODO; every code step shows complete code; every command shows expected output. ✓

**Type consistency:** Wiring paths verified against existing patterns — `common/src/constants/mod.rs` → `../../tests/constants.rs`; `common/src/oracle/observation.rs` → `../../../tests/oracle/observation.rs` (matches sibling `reflector.rs`); `contracts/controller/src/helpers/utils.rs` → `../../tests/helpers/utils.rs` (matches `src/oracle/tolerance.rs` depth). Signatures (`aggregate_payment_amount`, `push_unique_address`, observation fns, validation guards, `EModeSpokeUsageRaw` fields) read from source this session. ✓
