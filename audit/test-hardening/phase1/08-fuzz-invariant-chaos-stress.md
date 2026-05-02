# Domain 08 — Fuzz / Proptest / Invariant / Chaos / Stress

**Phase:** 1
**Files in scope:**
- `test-harness/tests/fuzz_auth_matrix.rs`
- `test-harness/tests/fuzz_budget_metering.rs`
- `test-harness/tests/fuzz_conservation.rs`
- `test-harness/tests/fuzz_liquidation_differential.rs`
- `test-harness/tests/fuzz_multi_asset_solvency.rs`
- `test-harness/tests/fuzz_strategy_flashloan.rs`
- `test-harness/tests/fuzz_ttl_keepalive.rs`
- `test-harness/tests/invariant_tests.rs`
- `test-harness/tests/chaos_simulation_tests.rs`
- `test-harness/tests/stress_simulation_tests.rs`
- `test-harness/tests/bench_liquidate_max_positions.rs`

**Totals:** broken=2 weak=10 nit=2

---

## Notes on F3 / F4

- All 7 proptest test files have a sibling `.proptest-regressions` file checked in (`git ls-files` confirms each is tracked). F3 passes for every proptest test in scope.
- None of the proptest configs disable shrinking or set a fixed `seed`. Failure cases listed in the regression files (e.g. `fuzz_liquidation_differential` has 3 stored shrinks at lines 7-9) replay deterministically because shrinking is left at the default. F4 passes everywhere.
- The non-proptest tests (`invariant_tests.rs`, `chaos_simulation_tests.rs`, `stress_simulation_tests.rs`, `bench_liquidate_max_positions.rs`) are scenario-based, not generator-based, so F2/F3/F4 do not apply. Only F1 + F5 are scored on those.

---

## Per-test entries

### `fuzz_auth_matrix.rs::prop_owner_only_endpoints_reject_unauthed`

**Severity:** none

### `fuzz_auth_matrix.rs::prop_wrong_role_rejected`

**Severity:** weak
**Rubric items failed:** [2]
**Why:** Generator at line 329 is `_seed in any::<u64>()` and the seed is then **discarded** (the `_` prefix is honoured — nothing in the body branches on it). The test runs the same fixed scenario `cases × shrinks` times. The proptest config block at lines 113-114 / 327 wraps both tests, so both share `cases: 64`. There is no domain coverage in the second test — it is effectively a regular `#[test]` masquerading as a proptest. Either parameterize the role being tested (KEEPER/REVENUE/ORACLE × wrong-target endpoint) using the seed, or convert this to a plain `#[test]`.

**Patch (suggested):**
```diff
--- before
+++ after
@@ test-harness/tests/fuzz_auth_matrix.rs:327 @@
-    #[test]
-    fn prop_wrong_role_rejected(
-        _seed in any::<u64>(),
-    ) {
+    #[test]
+    fn prop_wrong_role_rejected(
+        // 0 = KEEPER->REVENUE, 1 = KEEPER->ORACLE, 2 = REVENUE->ORACLE,
+        // 3 = REVENUE->KEEPER, 4 = ORACLE->KEEPER, 5 = ORACLE->REVENUE.
+        case_idx in 0u8..6,
+    ) {
```
…and switch on `case_idx` to grant the wrong role and call the wrong target endpoint, instead of the hardcoded KEEPER pair.

---

### `fuzz_budget_metering.rs::prop_keepalive_batch_stays_in_budget`

**Severity:** weak
**Rubric items failed:** [2, 5]
**Why:** Generator at line 50 caps `num_accounts` at 50, but the production cap on positions per account is 32 (`controller/src/config.rs:219` `max_supply_positions > 32` rejected) and the controller bootstrap default is 10 (`controller/src/access.rs:82-85`). The accounts created here have **no positions** at all (line 63 says `create_account generates synthetic IDs and only bumps the AccountNonce`). The most expensive `keepalive_accounts` path in production iterates `meta.supply_assets` × `meta.borrow_assets` per account, so 50 empty IDs is far cheaper than 10 fully-loaded accounts and does not exercise the budget envelope. Also `_assets_per_account` (line 52) is generated and never consumed (line 53 prefixes it with `_`). F5: no `///` doc comment names the invariant — the file-level docs do, but the proptest block itself only has line comments at 39-42.

**Patch (suggested):**
```diff
--- before
+++ after
@@ test-harness/tests/fuzz_budget_metering.rs:48 @@
-    #[test]
-    fn prop_keepalive_batch_stays_in_budget(
-        num_accounts in 1usize..=50,
-        // Keep the overall input small to avoid multi-minute proptest blowouts.
-        _assets_per_account in 1usize..=5,
-    ) {
+    /// Invariant: keepalive_accounts on accounts holding the production
+    /// position-limit ceiling either fits Soroban's default tx budget or
+    /// surfaces a budget/limit error. See controller/src/config.rs:219
+    /// (`max_supply_positions <= 32`) and controller/src/access.rs:82-85
+    /// (default 10/10 at construct).
+    #[test]
+    fn prop_keepalive_batch_stays_in_budget(
+        // Production cap is 32 supply + 32 borrow per account, default 10.
+        // Test bracket spans the realistic operator range.
+        num_accounts in 1usize..=10,
+        assets_per_account in 1usize..=4, // matches MAX_SUPPLY_POSITIONS = 4
+    ) {
```
…and use `assets_per_account` to actually open that many supply positions per account before calling `keepalive_accounts`, so the per-id work matches the worst-case production load.

---

### `fuzz_budget_metering.rs::prop_strategy_under_budget`

**Severity:** weak
**Rubric items failed:** [2]
**Why:** Generator at lines 120-122 — `supply_u in 100..10_000`, `leverage_bps in 10_000..30_000` — is fine on the leverage axis but the supply range is a tiny slice of the realistic envelope. The protocol caps positional value at `supply_cap` (i128), and operator dashboards routinely see millions in collateral. Pinning `supply_u` to 100..10k means the strategy path runs at the cheapest settlement size, and a regression that increases per-token swap cost would still fit. Widen to span the realistic range. The leverage upper bound of 3x is also conservative — the contract math allows leverage up to (1 / (1 - LTV)) ≈ 4x at LTV 75%; a `leverage_bps` ceiling of `40_000` would still be on the success path but would maximize swap-amount cost.

**Patch (suggested):**
```diff
--- before
+++ after
@@ test-harness/tests/fuzz_budget_metering.rs:118 @@
-    #[test]
-    fn prop_strategy_under_budget(
-        supply_u in 100u32..10_000,
-        // leverage: 1.0x .. 3.0x, encoded as basis points of supply.
-        leverage_bps in 10_000u32..30_000,
-    ) {
+    /// Invariant: multiply (leverage) within feasible LTV either fits
+    /// the default Soroban budget or fails with a budget error. Bounds
+    /// chosen to span operator-realistic deposit sizes and the full
+    /// leverage envelope reachable at LTV = 7500 (1 / (1 - 0.75) ≈ 4x).
+    #[test]
+    fn prop_strategy_under_budget(
+        supply_u in 100u32..1_000_000,
+        leverage_bps in 10_000u32..40_000, // 1x .. 4x (max feasible at LTV 7500)
+    ) {
```

---

### `fuzz_conservation.rs::prop_accounting_conservation`

**Severity:** none

(F1 ✓ four named conservation laws asserted at lines 188-235, citing INVARIANTS.md sections; F2 ✓ amounts 1..20_000 are well-matched to seeded 50k baseline + advance windows; F3 ✓ regression file has shrunk failing case at line 7; F4 ✓ default proptest config; F5 ✓ doc block at lines 1-29 names every law.)

---

### `fuzz_liquidation_differential.rs::prop_liquidation_matches_bigrational_reference`

**Severity:** none

(F1 ✓ asserts production matches BigRational reference within ulp/relative bounds at lines 219-279; F2 ✓ supply 1k..500k USDC, borrow fraction 100..9000 BPS, price-rise 5000..15000 BPS, all calibrated to the LTV/threshold geometry; F3 ✓ 3 stored regressions; F4 ✓; F5 ✓ extensive doc block lines 1-43.)

---

### `fuzz_multi_asset_solvency.rs::prop_solvency_across_op_sequences`

**Severity:** weak
**Rubric items failed:** [2, 5]
**Why:** F2 — Op-strategy (lines 42-86) has no weights — `prop_oneof![]` flat-distributes across 5 ops. Borrow at line 57 bounds `1..100` (which is then × 0.01 at line 149, i.e. 0.01..1.0 tokens), Withdraw at line 77 ranges 1..10_000 BPS. The Codex review note copied into `fuzz_conservation.rs:28-29` calls out **this exact file** for being borrow-heavy with few successful operations. The fuzzer only seeds 50k USDC for both users (lines 136-137) — there is no ETH/WBTC supply, so any borrow against ETH/WBTC fails. After the rebalance done in `fuzz_conservation.rs` (weighted `prop_oneof!`), this file still uses the old flat distribution. F5: doc block at lines 1-8 says only "assert per-step solvency" — does not cite which INVARIANTS.md section the assertions cover. Also note the comment block at lines 88-97 explicitly says HF >= 1 is **not** asserted, which removes one of the central invariants the file's doc block advertises.

**Patch (suggested):**
```diff
--- before
+++ after
@@ test-harness/tests/fuzz_multi_asset_solvency.rs:42 @@
-fn op_strategy() -> impl Strategy<Value = Op> {
-    prop_oneof![
-        (
-            prop_oneof![Just(ALICE), Just(BOB)],
-            prop_oneof![Just("USDC"), Just("ETH"), Just("WBTC")],
-            1u32..10_000u32
-        )
-            .prop_map(|(u, a, amt)| Op::Supply { user: u, asset: a, amt }),
-        (
-            prop_oneof![Just(ALICE), Just(BOB)],
-            prop_oneof![Just("USDC"), Just("ETH"), Just("WBTC")],
-            1u32..100u32
-        )
-            .prop_map(|(u, a, amt)| Op::Borrow { user: u, asset: a, amt }),
+/// Op weights mirror fuzz_conservation.rs: tilt toward supply + repay so
+/// borrowable liquidity remains and most ops succeed.
+fn op_strategy() -> impl Strategy<Value = Op> {
+    prop_oneof![
+        4 => (
+            prop_oneof![Just(ALICE), Just(BOB)],
+            prop_oneof![Just("USDC"), Just("ETH"), Just("WBTC")],
+            1u32..10_000u32,
+        ).prop_map(|(u, a, amt)| Op::Supply { user: u, asset: a, amt }),
+        2 => (
+            prop_oneof![Just(ALICE), Just(BOB)],
+            prop_oneof![Just("USDC"), Just("ETH"), Just("WBTC")],
+            1u32..100u32,
+        ).prop_map(|(u, a, amt)| Op::Borrow { user: u, asset: a, amt }),
```
…and pre-seed the other two assets at the top of the test alongside USDC so borrows can actually settle.

---

### `fuzz_strategy_flashloan.rs::prop_flash_loan_success_repayment`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** The test is `#[ignore]` at line 158 with the explanation that Soroban's recording-mode `mock_all_auths` cannot authorize the SAC admin mint inside the receiver. Because it never executes in CI, every assertion (lines 190-203) is dead code. F1 fails: an ignored proptest exercises no invariant. The note at lines 154-156 says the file stays alive for "regression surface" — but ignored tests do not regress; they sit out of the run. Either restructure so the property runs without the SAC mint (e.g. supply pre-funded liquidity to the receiver and have it `transfer` instead of `mint`), or delete the test and document the gap in `audit/REMEDIATION_PLAN.md`. Keeping it `#[ignore]` deceives the test count.

**Patch (suggested):**
```diff
--- before
+++ after
@@ test-harness/tests/fuzz_strategy_flashloan.rs:157 @@
-    #[test]
-    #[ignore = "real finding: Soroban recording-mode mock_all_auths cannot \
-                authorize nested SAC admin mint inside a flash-loan receiver; \
-                see bugs.md context and flash_loan_tests.rs test_flash_loan_success"]
-    fn prop_flash_loan_success_repayment(
-        amount_units in 100u32..100_000u32,
-    ) {
+    /// Invariant: a flash-loan round-trip leaves reserves grown by exactly
+    /// `flashloan_fee_bps × amount`, the reentrancy guard cleared, and
+    /// emits a successful return. Uses a pre-funded receiver that
+    /// `transfer`s the fee back instead of minting, sidestepping the SAC
+    /// admin auth gap in recording-mode mock_all_auths.
+    #[test]
+    fn prop_flash_loan_success_repayment(
+        amount_units in 100u32..100_000u32,
+    ) {
```
…and rewrite `deploy_flash_loan_receiver` (or use an alternate variant) to be pre-funded with USDC at construction so the fee return is a `transfer` from the receiver's own balance, not a SAC mint.

---

### `fuzz_strategy_flashloan.rs::prop_multiply_leverage_hf_safe`

**Severity:** none

(F1 ✓ asserts NEW-01 zero-allowance + HF >= 1 + clean error rollback at lines 257-275; F2 ✓ debt 1..10 ETH and 1.5x..5x leverage are realistic; F5 ✓ block-level doc lines 207-221.)

---

### `fuzz_strategy_flashloan.rs::prop_strategy_swap_collateral_balance_delta`

**Severity:** weak
**Rubric items failed:** [2]
**Why:** Generator at line 292 — `min_out_valid in any::<bool>()` — flips a coin between the M-10 trigger (min_out = 0) and the happy path. Half the generated cases test M-10 and half test the balance-delta consistency check. With `cases: 16`, that is 8 effective cases per branch — **shallow coverage** for two distinct invariants. Either split into two proptest functions (one for M-10, one for M-11) so each runs the full `cases` budget, or weight the generator toward the M-11 path with a small `min_out_valid` skew (e.g. 80% valid / 20% invalid) and bump `cases` to 32+. Also `withdraw_frac_bps in 100..5000` (line 291) caps at 50% — the M-11 regression is most likely to fire on near-full-withdraw paths where rounding deltas are sharpest, so 9000 BPS upper bound would test more.

**Patch (suggested):**
```diff
--- before
+++ after
@@ test-harness/tests/fuzz_strategy_flashloan.rs:289 @@
-    #[test]
-    fn prop_strategy_swap_collateral_balance_delta(
-        withdraw_frac_bps in 100u32..5_000u32, // 1% -- 50% withdrawal
-        min_out_valid in any::<bool>(),
-    ) {
+    #[test]
+    fn prop_strategy_swap_collateral_balance_delta(
+        // 1% .. 90% withdrawal — M-11 regression probes rounding at the
+        // tail; do not cap at 50%.
+        withdraw_frac_bps in 100u32..9_000u32,
+        // Skew toward valid swaps: M-10 trigger (min_out = 0) only needs a
+        // handful of cases; M-11 balance-delta needs deep coverage.
+        min_out_valid in prop_oneof![4 => Just(true), 1 => Just(false)],
+    ) {
```

---

### `fuzz_ttl_keepalive.rs::prop_keepalive_accounts_bumps_positions`

**Severity:** weak
**Rubric items failed:** [2]
**Why:** Generator at line 76 — `num_accounts in 1usize..=5` — caps at 5 accounts, but the proptest also indexes `USERS` (line 34) which has 5 entries, so the cap is **the array length, not the protocol envelope**. Production keepers batch hundreds of accounts; a regression that scales super-linearly in `num_accounts` is invisible at N <= 5. `asset_mix in vec![0..3, 1..=3]` (line 77) caps total assets at 3 (the array length), matching the 3 markets configured at line 60-65 — fine. Widen the user pool. The harness already supports synthetic user names via `t.supply(user, ...)` accepting any `&str`.

**Patch (suggested):**
```diff
--- before
+++ after
@@ test-harness/tests/fuzz_ttl_keepalive.rs:34 @@
-const USERS: &[&str] = &["alice", "bob", "carol", "dave", "eve"];
+const USERS: &[&str] = &[
+    "alice", "bob", "carol", "dave", "eve",
+    "u06", "u07", "u08", "u09", "u10",
+    "u11", "u12", "u13", "u14", "u15",
+    "u16", "u17", "u18", "u19", "u20",
+];
@@ test-harness/tests/fuzz_ttl_keepalive.rs:74 @@
-    #[test]
-    fn prop_keepalive_accounts_bumps_positions(
-        num_accounts in 1usize..=5,
-        asset_mix in prop::collection::vec(0usize..3, 1..=3),
-    ) {
+    #[test]
+    fn prop_keepalive_accounts_bumps_positions(
+        // Span operator-realistic batch sizes. Cost-model regression in
+        // bump_account that scales super-linearly is only visible at >5.
+        num_accounts in 1usize..=20,
+        asset_mix in prop::collection::vec(0usize..3, 1..=3),
+    ) {
```

---

### `fuzz_ttl_keepalive.rs::prop_keepalive_shared_bumps_markets`

**Severity:** none

### `fuzz_ttl_keepalive.rs::prop_keepalive_pools_forwards`

**Severity:** weak
**Rubric items failed:** [1]
**Why:** Assertion at line 215-218 is `post_ttl >= *pre_ttl`. After `keepalive_pools` the pool's instance TTL must be **at least `TTL_BUMP_INSTANCE`** ledgers in the future (`common/src/constants.rs:63` = `ONE_DAY_LEDGERS * 180`). The current assertion accepts a no-op bump (post == pre). If the controller silently fails to forward `keepalive_pools` and the pre-existing TTL from deploy time happens to be exactly preserved, the test passes. The check should be `post_ttl >= TTL_BUMP_INSTANCE.saturating_sub(1)` — same pattern Property 1 uses for `min_ttl` (line 111).

**Patch (suggested):**
```diff
--- before
+++ after
@@ test-harness/tests/fuzz_ttl_keepalive.rs:213 @@
-        for (name, pre_ttl, pool) in &pre {
-            let post_ttl = pool_instance_ttl(&t, pool);
-            prop_assert!(
-                post_ttl >= *pre_ttl,
-                "pool {} instance TTL regressed: {} -> {}", name, pre_ttl, post_ttl
-            );
-        }
+        let min_ttl = common::constants::TTL_BUMP_INSTANCE.saturating_sub(1);
+        for (name, pre_ttl, pool) in &pre {
+            let post_ttl = pool_instance_ttl(&t, pool);
+            prop_assert!(
+                post_ttl >= min_ttl,
+                "pool {} instance TTL not bumped to TTL_BUMP_INSTANCE: \
+                 pre={} post={} min={}",
+                name, pre_ttl, post_ttl, min_ttl
+            );
+        }
```

---

### `fuzz_ttl_keepalive.rs::prop_account_orphan_positions_not_stuck`

**Severity:** weak
**Rubric items failed:** [2]
**Why:** Generator at lines 240-242 — `supply_amt 10..1_000`, `num_partials 0..=3`, `partial_bps vec(1000..9000, 0..=3)`. `num_partials` and `partial_bps.len()` are **independent**: the loop at line 254 uses `partial_bps.get(i).copied().unwrap_or(5000)`, so when `num_partials = 3` and `partial_bps = []`, every partial uses the default 5000 BPS — exact same scenario every time. Either tie `partial_bps.len() = num_partials` (use `prop::collection::vec(1000u16..9000, num_partials..=num_partials)` via `.prop_flat_map`), or drop `num_partials` and just take `partial_bps.len()` as the count. M-14 regression coverage is currently under-fuzzed because the default-fallback collapses the parameter space.

**Patch (suggested):**
```diff
--- before
+++ after
@@ test-harness/tests/fuzz_ttl_keepalive.rs:238 @@
-    #[test]
-    fn prop_account_orphan_positions_not_stuck(
-        supply_amt in 10u32..1_000,
-        num_partials in 0u32..=3,
-        partial_bps in prop::collection::vec(1000u16..9000, 0..=3),
-    ) {
+    #[test]
+    fn prop_account_orphan_positions_not_stuck(
+        supply_amt in 10u32..1_000,
+        // Tie partial count to vector length so every iteration uses a
+        // generated bps value, not the silent 5000 fallback.
+        partial_bps in prop::collection::vec(1000u16..9000, 0..=5),
+    ) {
+        let num_partials = partial_bps.len() as u32;
```

---

### `invariant_tests.rs::test_hf_above_one_after_every_borrow`

**Severity:** none

### `invariant_tests.rs::test_hf_above_one_after_every_withdraw`

**Severity:** none

### `invariant_tests.rs::test_hf_below_one_required_for_liquidation`

**Severity:** none

### `invariant_tests.rs::test_ltv_less_than_threshold_always`

**Severity:** none

### `invariant_tests.rs::test_supply_index_monotonically_increasing`

**Severity:** none

### `invariant_tests.rs::test_borrow_index_monotonically_increasing`

**Severity:** none

### `invariant_tests.rs::test_position_limits_enforced`

**Severity:** none

### `invariant_tests.rs::test_isolation_and_emode_mutually_exclusive`

**Severity:** none

### `invariant_tests.rs::test_total_supply_matches_pool_balance`

**Severity:** nit
**Rubric items failed:** [5]
**Why:** F1 borderline — the assertion at lines 280-285 is `(total_user_supply - 80_000.0).abs() < 10.0`, which is fine, and the pool-balance assertion at line 290-296 is `pool_balance >= total_user_supply`, named in the inline comment as "the invariant". But the test name `test_total_supply_matches_pool_balance` implies an **equality** invariant, while the assertion is an **inequality** that is trivially satisfied because the pool was seeded with 1M baseline. A regression that drains `pool_balance` to exactly `total_user_supply - $1` would be caught, but a regression that **inflates** it (phantom liquidity) would not. F5: the doc comment at line 263 just numbers the test — does not state which property. Add a `///` block specifying that this guards INVARIANTS.md "Pool Solvency Identity".

**Patch (suggested):**
```diff
--- before
+++ after
@@ test-harness/tests/invariant_tests.rs:262 @@
-// ---------------------------------------------------------------------------
-// 9. test_total_supply_matches_pool_balance
-// ---------------------------------------------------------------------------
-
-#[test]
-fn test_total_supply_matches_pool_balance() {
+// ---------------------------------------------------------------------------
+// 9. test_total_supply_matches_pool_balance
+// ---------------------------------------------------------------------------
+
+/// INVARIANTS.md Pool Solvency Identity: pool token balance >= sum of
+/// user supply balances. Inequality (not equality) because pool may
+/// hold seed liquidity / donations on top of user deposits.
+#[test]
+fn test_total_supply_matches_pool_balance() {
```

---

### `invariant_tests.rs::test_full_lifecycle_supply_borrow_repay_withdraw`

**Severity:** none

---

### `chaos_simulation_tests.rs::test_chaos_multi_user_random_operations`

**Severity:** weak
**Rubric items failed:** [1]
**Why:** "Random" is misleading — the LCG at lines 16-37 is seeded with a fixed `Rng::new(42)` (line 58), so this test runs **one fixed scenario**, not a randomized one. The HF check at line 156 `hf >= 1.0 || hf == f64::MAX || hf > 1e18` is also too loose: `hf > 1e18` accepts any positive float including INF, and the disjunction makes the test pass trivially for any account with no debt. Either: (a) parameterize the seed by reading from an env var (`std::env::var("CHAOS_SEED")`) so CI can sweep, or (b) acknowledge this is a deterministic regression scenario and rename it. The borrow_successes check at line 139 only requires >= 3 of 12 — a weak bar.

**Patch (suggested):**
```diff
--- before
+++ after
@@ test-harness/tests/chaos_simulation_tests.rs:43 @@
-#[test]
-fn test_chaos_multi_user_random_operations() {
+/// Deterministic chaos regression: 15 users, fixed seed-42 scenario over
+/// 8 weeks. Asserts (1) every borrowing account stays HF >= 1 OR has no
+/// debt, (2) supply/borrow indexes rose above 1.0 RAY for every market,
+/// (3) protocol revenue stays >= 0. NOT a randomized test — the LCG is
+/// seeded with the constant 42 so the scenario replays identically.
+#[test]
+fn test_chaos_multi_user_random_operations() {
@@ test-harness/tests/chaos_simulation_tests.rs:155 @@
-                assert!(
-                    hf >= 1.0 || hf == f64::MAX || hf > 1e18,
-                    "user {} HF should be >= 1.0, got {}",
-                    user,
-                    hf
-                );
+                // f64::INFINITY iff the user has no debt — that is fine.
+                // Otherwise must be >= 1.0.
+                let healthy = hf == f64::INFINITY || hf >= 1.0;
+                assert!(healthy, "user {} HF should be >= 1.0, got {}", user, hf);
```

---

### `chaos_simulation_tests.rs::test_chaos_bank_run_full_exit`

**Severity:** none

### `chaos_simulation_tests.rs::test_chaos_sustained_high_utilization`

**Severity:** none

### `chaos_simulation_tests.rs::test_chaos_price_oscillation_no_wrongful_liquidation`

**Severity:** none

### `chaos_simulation_tests.rs::test_chaos_multi_market_accounting`

**Severity:** none

### `chaos_simulation_tests.rs::test_chaos_keeper_revenue_lifecycle`

**Severity:** none

---

### `stress_simulation_tests.rs::test_multi_user_lending_cycle`

**Severity:** none

### `stress_simulation_tests.rs::test_full_exit_solvency`

**Severity:** none

### `stress_simulation_tests.rs::test_cascading_liquidations_stability`

**Severity:** none

### `stress_simulation_tests.rs::test_interest_accrual_consistency`

**Severity:** none

### `stress_simulation_tests.rs::test_position_limit_stress`

**Severity:** nit
**Rubric items failed:** [5]
**Why:** F1 asserts `assert_borrow_count`/`assert_supply_count` after each operation (lines 414, 421, 428, 432) and `assert_healthy` at line 422 — fine. But the test does not actually stress the limit: `with_position_limits(3, 3)` (line 406) caps at 3 yet the file has 3 markets configured (lines 403-405), so opening all 3 does not hit a *cap* — it hits the *full set*. To stress the limit, the test should either lower the limit below the asset count and assert the over-limit operation rejects, or add a 4th market and confirm a 4th supply gets rejected. As written, the test only confirms "up to N positions can be opened where N is also the number of available markets". F5: the section header at line 397 says "Position limit stress" but the test never tests rejection at the cap — only successful repay-then-reopen.

**Patch (suggested):**
```diff
--- before
+++ after
@@ test-harness/tests/stress_simulation_tests.rs:400 @@
-#[test]
-fn test_position_limit_stress() {
-    let mut t = LendingTest::new()
-        .with_market(usdc_preset())
-        .with_market(eth_preset())
-        .with_market(wbtc_preset())
-        .with_position_limits(3, 3)
-        .build();
+/// Position-limit stress: with 4 markets and a cap of 3, the 4th supply
+/// must be rejected with POSITION_LIMIT_EXCEEDED. After a partial repay,
+/// the cap window must reopen.
+#[test]
+fn test_position_limit_stress() {
+    let mut t = LendingTest::new()
+        .with_market(usdc_preset())
+        .with_market(usdt_stable_preset())
+        .with_market(eth_preset())
+        .with_market(wbtc_preset())
+        .with_position_limits(3, 3)
+        .build();
```
…and add a fourth supply attempt that asserts `POSITION_LIMIT_EXCEEDED` (mirror of `invariant_tests.rs::test_position_limits_enforced` at lines 199-218).

---

### `stress_simulation_tests.rs::test_keeper_index_freshness_matters`

**Severity:** none

---

### `bench_liquidate_max_positions.rs::bench_liquidate_5_supply_5_borrow_within_default_budget`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** This is *not* a fuzz/proptest — it is a single-scenario bench. The "invariant" is: liquidate either succeeds or surfaces a budget error (lines 127-144). But the only enforcement is that the panic message contains "budget" / "exceeded" / "limit" / "cpu" / "memory" / "entries" / "size" (lines 67-72). That string match is also satisfied by panic messages like `"index out of bounds"` (contains "limit") or `"size mismatch"` (contains "size"). A genuine arithmetic-overflow panic in liquidation that says `"size of i128 overflow"` would be **silently classified as acceptable**. The classifier is too permissive. Tighten to require an explicit budget-error type (`HostError::Budget(ExceededLimit)` or the controller's named budget error code) rather than substring matching; or assert the specific Soroban host-error variant via `payload.downcast_ref::<HostError>()`.

Additional concern: the file states (lines 26-28) that for 32/32 (the actual contract cap) the harness needs more presets. As-is, the bench tests 5/5, which is `5/32 = 16%` of the realistic ceiling. The TODO is not gated on by a tracking issue.

**Patch (suggested):**
```diff
--- before
+++ after
@@ test-harness/tests/bench_liquidate_max_positions.rs:65 @@
-    let low = msg.to_lowercase();
-    let is_budget = low.contains("budget")
-        || low.contains("exceeded")
-        || low.contains("limit")
-        || low.contains("cpu")
-        || low.contains("memory")
-        || low.contains("entries")
-        || low.contains("size");
+    // Tighten substring match: previous "limit" / "size" matches were
+    // satisfied by "index out of bounds" and arithmetic-overflow text.
+    // Require Soroban host-budget keywords AND exclude common false
+    // positives (overflow, out of bounds, panicked).
+    let low = msg.to_lowercase();
+    let is_overflow = low.contains("overflow") || low.contains("out of bounds");
+    let is_budget = !is_overflow
+        && (low.contains("budget exceeded")
+            || low.contains("exceededlimit")
+            || low.contains("cpu instruction")
+            || low.contains("memory limit")
+            || low.contains("read entries")
+            || low.contains("write entries")
+            || low.contains("tx size"));
```
And for the 5/5 → 32/32 gap, file a tracking issue and add an `#[ignore]`d sibling test that compiles the 32/32 setup — at least the build path stays exercised when new preset assets land.
