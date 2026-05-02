# Domain 08 — Fuzz / Proptest / Invariant / Chaos / Stress (Phase 2 Review)

**Phase:** 2 (independent review)
**Reviewing:** `audit/test-hardening/phase1/08-fuzz-invariant-chaos-stress.md`
**Files re-read:** all 11 source files in scope, plus `common/src/constants.rs`,
`controller/src/access.rs`, `controller/src/config.rs`, `pool/src/lib.rs`,
`test-harness/src/user.rs`, `test-harness/src/view.rs`,
`test-harness/src/helpers.rs`.

**Totals:** confirmed=8 refuted=1 refined=4 new=1

---

## Per-entry verdicts

### `fuzz_auth_matrix.rs::prop_owner_only_endpoints_reject_unauthed`

**Phase 1 severity:** none
**Disposition:** confirmed

(No finding to validate.)

---

### `fuzz_auth_matrix.rs::prop_wrong_role_rejected`

**Phase 1 severity:** weak (F2)
**Disposition:** confirmed

Verified: line 329 generator `_seed in any::<u64>()` is silently consumed
(`let _ = (max_supply, max_borrow);` analogue does not exist here — the seed
is never re-bound or branched on at lines 335-393). The body executes the
exact same KEEPER → REVENUE and KEEPER → ORACLE pair every iteration. The
proptest config at line 114 (`cases: 64`) wraps both tests, so this runs the
same scenario 64 × shrinks times. The auditor's proposed `case_idx in 0u8..6`
parameterization is a sound F2 fix grounded in the role matrix from
`controller/src/access.rs:9-11` (`KEEPER`, `REVENUE`, `ORACLE`).

---

### `fuzz_budget_metering.rs::prop_keepalive_batch_stays_in_budget`

**Phase 1 severity:** weak (F2, F5)
**Disposition:** refined

Verified the F2 critique. Generator at line 50 caps `num_accounts` at 50 with
no per-account positions (lines 60-66 only call `t.create_account`, which per
`test-harness/src/user.rs:13-18` and lines 83-107 creates an `AccountMeta`
entry with no SupplyPosition/BorrowPosition rows — confirming the auditor's
"no positions at all" claim, slight wording nit: it does set AccountMeta, not
*only* the AccountNonce). Line 52's `_assets_per_account` is generated and
discarded. F5: the proptest block has line-comment docs (39-42) but no `///`
on the test fn.

**Reviewer note:** the auditor's proposed bound `num_accounts in 1usize..=10,
assets_per_account in 1usize..=4` is grounded in the right constants
(`controller/src/access.rs:82-85` defaults 10/10,
`common/src/constants.rs:74-76` `MAX_SUPPLY_POSITIONS = 4`,
`controller/src/config.rs:219-220` enforces ≤ 32 ceiling) — but the bench
should ideally also probe up toward 32 to catch super-linear regressions on
the production cap. Refining the patch to add a "stress" sibling case at the
controller cap.

**Patch (refined):**
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
+    /// Invariant: keepalive_accounts on a realistic operator batch either
+    /// fits Soroban's default tx budget or surfaces a budget/limit error.
+    /// Bounds:
+    /// - `num_accounts` 1..=10 mirrors the controller's bootstrap default
+    ///   (`controller/src/access.rs:82-85` PositionLimits 10/10).
+    /// - `assets_per_account` 1..=4 matches `common/src/constants.rs:74-76`
+    ///   `MAX_SUPPLY_POSITIONS = 4`, the live default per-account ceiling.
+    /// `controller/src/config.rs:219-220` enforces an absolute hard ceiling
+    /// of 32; widen the bound to that once the harness adds enough preset
+    /// markets to populate 32 supply rows per account.
+    #[test]
+    fn prop_keepalive_batch_stays_in_budget(
+        num_accounts in 1usize..=10,
+        assets_per_account in 1usize..=4,
+    ) {
+        let assets = ["USDC", "ETH", "WBTC"];
```
…then in the body, supply `assets_per_account.min(assets.len())` markets per
created user before invoking `keepalive_accounts`, so the per-id storage
walk actually iterates the supply set.

---

### `fuzz_budget_metering.rs::prop_strategy_under_budget`

**Phase 1 severity:** weak (F2)
**Disposition:** confirmed

Verified `supply_u in 100u32..10_000` (line 120) and `leverage_bps in
10_000..30_000` (line 122). Auditor's reasoning about the leverage ceiling is
correct: at LTV 7500 BPS the geometric series reaches `1 / (1 - 0.75) ≈ 4x`,
so `40_000` is the realistic upper bound. The proposed wider supply range
(100..1_000_000) is a sound F2 fix for the cost model regression goal.

---

### `fuzz_conservation.rs::prop_accounting_conservation`

**Phase 1 severity:** none
**Disposition:** confirmed

Verified all four laws assert at lines 188-235; weighted `prop_oneof!` at
lines 79-96 actually rebalances supply/borrow per the doc block. F1-F5 all
satisfied.

---

### `fuzz_liquidation_differential.rs::prop_liquidation_matches_bigrational_reference`

**Phase 1 severity:** none
**Disposition:** confirmed

Verified F1-F5. Regression file has 3 stored shrinks at lines 7-9.

---

### `fuzz_multi_asset_solvency.rs::prop_solvency_across_op_sequences`

**Phase 1 severity:** weak (F2, F5)
**Disposition:** refined

Verified op-strategy at lines 42-86 has flat distribution, borrow at line 57
ranges `1..100u32` (× 0.01 = 0.01..1.0 tokens at line 149). Lines 88-97
explicitly disclaim HF-≥-1 enforcement. The auditor is right that the file
docs (lines 1-8) advertise solvency invariants without HF, so the doc block
and assertions match — the F1 framing is fine. F2 is the real issue:
borrow-heavy + ETH/WBTC unseeded means most borrows fail.

**Reviewer note:** verified that lines 136-137 only seed USDC for both users;
ETH and WBTC supply pools have no liquidity until users randomly hit the
Supply branch. The auditor's proposed weighted `prop_oneof!` matches
`fuzz_conservation.rs::op_strategy()` (lines 75-97) — that file is the
working pattern. The auditor also calls for pre-seeding ETH and WBTC; this
is necessary even with the weighted distribution because the first borrow
of a non-USDC asset against USDC-only pool reserves still fails before any
counter-party Supply op fires.

Refining the patch to also seed all three assets, mirroring
`fuzz_conservation.rs:148-151`:

**Patch (refined):**
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
+/// Op weights mirror fuzz_conservation.rs:75-97: tilt toward supply + repay
+/// so borrowable liquidity remains and most ops succeed. Without weights
+/// the flat 5-way distribution produces ~20% supplies and ~20% borrows
+/// against single-asset seed liquidity, so most borrow ops short-circuit
+/// and the harness yields little coverage of solvency drift.
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
+            1u16..10_000u16,
+        ).prop_map(|(u, a, f)| Op::Repay { user: u, asset: a, frac_bps: f }),
+        1 => (
+            prop_oneof![Just(ALICE), Just(BOB)],
+            prop_oneof![Just("USDC"), Just("ETH"), Just("WBTC")],
+            1u32..100u32,
+        ).prop_map(|(u, a, amt)| Op::Borrow { user: u, asset: a, amt }),
+        1 => (
+            prop_oneof![Just(ALICE), Just(BOB)],
+            prop_oneof![Just("USDC"), Just("ETH"), Just("WBTC")],
+            1u16..10_000u16,
+        ).prop_map(|(u, a, f)| Op::Withdraw { user: u, asset: a, frac_bps: f }),
+        2 => (60u32..(7 * 24 * 3600)).prop_map(|s| Op::Advance { secs: s }),
+    ]
+}
@@ test-harness/tests/fuzz_multi_asset_solvency.rs:135 @@
-        // Prime both users with collateral so borrows can succeed.
-        t.supply(ALICE, "USDC", 50_000.0);
-        t.supply(BOB, "USDC", 50_000.0);
+        // Pre-seed every market so a borrow op against ETH or WBTC has
+        // pool liquidity to draw from from the first step. Mirrors
+        // fuzz_conservation.rs:148-151.
+        t.supply(ALICE, "USDC", 50_000.0);
+        t.supply(BOB, "USDC", 50_000.0);
+        t.supply(ALICE, "ETH", 20.0);
+        t.supply(BOB, "WBTC", 1.0);
```

---

### `fuzz_strategy_flashloan.rs::prop_flash_loan_success_repayment`

**Phase 1 severity:** broken (F1)
**Disposition:** confirmed

Verified `#[ignore = "real finding: ..."]` at lines 158-160. The test is
guaranteed not to run in `cargo test` without `--ignored`. The assertions at
lines 190-203 are dead code as written. F1 fails: an ignored proptest is not
testing any invariant. The auditor's call to either restructure the receiver
to use `transfer` from a pre-funded balance (sidestepping the SAC admin mint
auth gap) or delete the test outright is sound.

---

### `fuzz_strategy_flashloan.rs::prop_multiply_leverage_hf_safe`

**Phase 1 severity:** none
**Disposition:** confirmed

(F1, F2, F5 verified at the cited lines.)

---

### `fuzz_strategy_flashloan.rs::prop_strategy_swap_collateral_balance_delta`

**Phase 1 severity:** weak (F2)
**Disposition:** confirmed

Verified line 292 `min_out_valid in any::<bool>()` — coin flip between M-10
trigger and M-11 happy path. With `cases: 16` (line 138), only ~8 cases per
branch. Line 291 `withdraw_frac_bps in 100..5_000` caps at 50%. Auditor's
critique is sound: M-11 rounding stress concentrates at near-full-withdraw
paths where collateral-supply scaled reads round closer to zero. The
proposed weighted `prop_oneof![4 => Just(true), 1 => Just(false)]` and
widened `100..9_000` is a clean F2 fix.

---

### `fuzz_ttl_keepalive.rs::prop_keepalive_accounts_bumps_positions`

**Phase 1 severity:** weak (F2)
**Disposition:** confirmed

Verified line 76 `num_accounts in 1usize..=5` is bound by `USERS` (line 34,
5 entries), not by any protocol envelope. The auditor's call to expand the
USERS array and widen `num_accounts` to 1..=20 is sound — the test exercises
batched keeper operation, and `t.supply(user, ...)` (per
`test-harness/src/user.rs`) accepts any `&str` so synthetic user names
("u06"..) work.

---

### `fuzz_ttl_keepalive.rs::prop_keepalive_shared_bumps_markets`

**Phase 1 severity:** none
**Disposition:** confirmed

---

### `fuzz_ttl_keepalive.rs::prop_keepalive_pools_forwards`

**Phase 1 severity:** weak (F1)
**Disposition:** refined

Verified the assertion at lines 215-218 is `post_ttl >= *pre_ttl`, which
accepts a no-op (post == pre). `pool/src/lib.rs:693-698` shows
`pool::keepalive` calls `extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE)`,
which only extends when current TTL falls below `TTL_THRESHOLD_INSTANCE`
(120 days per `common/src/constants.rs:62`). After extension, the new TTL
is at least `TTL_BUMP_INSTANCE` (180 days). Property 1
(`prop_keepalive_accounts_bumps_positions`) at line 111 already uses the
stricter `post_ttl >= TTL_BUMP_USER.saturating_sub(1)` pattern.

**Reviewer note:** the auditor's proposed strict check
`post_ttl >= TTL_BUMP_INSTANCE.saturating_sub(1)` is semantically right but
could trigger a false positive if Soroban's default deploy TTL for instance
storage starts above `TTL_BUMP_INSTANCE` (in which case keepalive is a
no-op AND post_ttl > TTL_BUMP_INSTANCE, so the strict check still passes).
That's not the issue — what is: the strict check WILL fail if pre_ttl is
below `TTL_BUMP_INSTANCE` (the common case after some ledger advance) AND
keepalive silently does not run. That's exactly the regression class to
catch. The strict check's only edge case is equivalent: pre = post = some
value below `TTL_BUMP_INSTANCE - 1` — both `pre >= post` AND the strict
check fail, so the regression surfaces. Patch confirmed sound.

The patch as written uses `common::constants::TTL_BUMP_INSTANCE` — but the
file already imports `TTL_BUMP_SHARED, TTL_BUMP_USER` at line 27, so the
import line should add `TTL_BUMP_INSTANCE` to that use. Refined patch:

**Patch (refined):**
```diff
--- before
+++ after
@@ test-harness/tests/fuzz_ttl_keepalive.rs:27 @@
-use common::constants::{TTL_BUMP_SHARED, TTL_BUMP_USER};
+use common::constants::{TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_BUMP_USER};
@@ test-harness/tests/fuzz_ttl_keepalive.rs:213 @@
-        for (name, pre_ttl, pool) in &pre {
-            let post_ttl = pool_instance_ttl(&t, pool);
-            prop_assert!(
-                post_ttl >= *pre_ttl,
-                "pool {} instance TTL regressed: {} -> {}", name, pre_ttl, post_ttl
-            );
-        }
+        // Mirror property 1 (line 111): require post_ttl to clear the
+        // TTL_BUMP_INSTANCE floor (180 days, common/src/constants.rs:63),
+        // not just "did not regress" — a silent no-op forwarder would pass
+        // the weaker check trivially.
+        let min_ttl = TTL_BUMP_INSTANCE.saturating_sub(1);
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

**Phase 1 severity:** weak (F2)
**Disposition:** confirmed

Verified lines 240-242 generators are independent: `num_partials in 0u32..=3`,
`partial_bps in vec(1000..9000, 0..=3)`. Line 255 fallback `unwrap_or(5000)`
is reached whenever `num_partials > partial_bps.len()`. Auditor's proposed
fix (drop `num_partials`, derive count from `partial_bps.len()`) collapses
that joint-distribution gap. Sound F2 fix.

---

### `invariant_tests.rs` (10 tests)

Items 1-8, 10: **none**, confirmed.

#### `invariant_tests.rs::test_total_supply_matches_pool_balance`

**Phase 1 severity:** nit (F5)
**Disposition:** confirmed

Verified the assertion at lines 290-296 is the inequality `pool_balance >=
total_user_supply`, which is correct for the post-bootstrap protocol model
(seed liquidity floods the pool). The test name promises equality. The
auditor's F5 fix (add a `///` block citing INVARIANTS.md "Pool Solvency
Identity") is appropriate.

---

### `chaos_simulation_tests.rs::test_chaos_multi_user_random_operations`

**Phase 1 severity:** weak (F1)
**Disposition:** refuted (in part) / refined

The auditor flagged two issues: (a) the LCG seed is fixed (line 58) and
(b) the HF check at line 156 `hf >= 1.0 || hf == f64::MAX || hf > 1e18`
is too loose.

**Reviewer note (refute on F1, accept on naming/F5):** the test does assert
named invariants:
1. HF ≥ 1.0 for every borrowing account (line 156).
2. Supply and borrow indexes ≥ 1.0 RAY for every market (lines 175, 180).
3. Protocol revenue ≥ 0 for every market (line 190).

These are clear, named invariants tied to the protocol's safety model, so
F1 is satisfied. The fixed seed is a separate concern: the test is
deterministic but it is a *deterministic regression scenario*, not random.
The Phase-1 framing as F1-broken is incorrect; the right framing is F5
(name does not match what it does — "random_operations" implies seed
sweep).

The HF expression `hf >= 1.0 || hf == f64::MAX || hf > 1e18` is genuinely
loose, but the auditor's proposed fix `hf == f64::INFINITY || hf >= 1.0` is
**incorrect** for this codebase. Verified via
`test-harness/src/view.rs:28-32` and `test-harness/src/helpers.rs:69-71`:
`health_factor` for a no-debt account returns
`wad_to_f64(i128::MAX) = i128::MAX as f64 / WAD as f64 ≈ 1.7e20` — a
finite f64, **not** `f64::INFINITY`. The check `hf == f64::INFINITY`
would reject the no-debt happy path; `hf > 1e18` is a fingerprint of
exactly that synthetic value. Replacing the disjunction with the
INFINITY check is a regression, not a fix.

A correct refinement: rename the test, add a `///` doc block stating the
seed is fixed and the test is a regression scenario, and tighten the HF
expression to drop the redundant `f64::MAX` term (impossible per the
encoding) while keeping the `1e18` discriminator.

**Patch (refined):**
```diff
--- before
+++ after
@@ test-harness/tests/chaos_simulation_tests.rs:42 @@
-#[test]
-fn test_chaos_multi_user_random_operations() {
+/// Deterministic chaos regression: 15 users, fixed-seed (42) LCG-driven
+/// scenario over 8 weeks with one ETH price oscillation. Assertions:
+///  (1) every borrowing account stays HF >= 1 OR has no debt (no-debt HF
+///      surfaces as `i128::MAX / WAD` per
+///      test-harness/src/view.rs:28-32 + helpers.rs:69-71, hence the
+///      `> 1e18` discriminator below).
+///  (2) supply and borrow indexes >= 1.0 RAY for every market.
+///  (3) protocol revenue >= 0 for every market.
+/// The "random" in the name is a misnomer kept for git history -- this is
+/// a deterministic regression scenario, not a randomized fuzz.
+#[test]
+fn test_chaos_multi_user_random_operations() {
@@ test-harness/tests/chaos_simulation_tests.rs:155 @@
-                assert!(
-                    hf >= 1.0 || hf == f64::MAX || hf > 1e18,
-                    "user {} HF should be >= 1.0, got {}",
-                    user,
-                    hf
-                );
+                // No-debt accounts surface as health_factor_raw = i128::MAX,
+                // which divides to ~1.7e20 in f64 (test-harness helpers:
+                // wad_to_f64). Use `> 1e18` as the no-debt fingerprint;
+                // f64::MAX and f64::INFINITY never appear on this path.
+                let healthy = hf > 1e18 || hf >= 1.0;
+                assert!(healthy, "user {} HF should be >= 1.0, got {}", user, hf);
```

---

### `chaos_simulation_tests.rs::test_chaos_bank_run_full_exit`, `test_chaos_sustained_high_utilization`, `test_chaos_price_oscillation_no_wrongful_liquidation`, `test_chaos_multi_market_accounting`, `test_chaos_keeper_revenue_lifecycle`

**Phase 1 severity:** none
**Disposition:** confirmed

---

### `stress_simulation_tests.rs::test_multi_user_lending_cycle`, `test_full_exit_solvency`, `test_cascading_liquidations_stability`, `test_interest_accrual_consistency`, `test_keeper_index_freshness_matters`

**Phase 1 severity:** none
**Disposition:** confirmed

---

### `stress_simulation_tests.rs::test_position_limit_stress`

**Phase 1 severity:** nit (F5)
**Disposition:** confirmed

Verified line 406 `with_position_limits(3, 3)` matches the 3 markets at
lines 403-405 (USDC, ETH, WBTC). The test exercises the "open / repay /
reopen" round trip but never tests rejection at the cap, since the cap
equals the asset count. `invariant_tests.rs::test_position_limits_enforced`
(lines 199-218) is the working pattern: 4 markets with limit 2/2, 3rd
supply asserts `POSITION_LIMIT_EXCEEDED`. The auditor's proposed addition
of a 4th market and a rejection assertion is sound.

---

### `bench_liquidate_max_positions.rs::bench_liquidate_5_supply_5_borrow_within_default_budget`

**Phase 1 severity:** broken (F1)
**Disposition:** confirmed

Verified the substring classifier at lines 65-72 accepts panic messages
containing any of "budget", "exceeded", "limit", "cpu", "memory", "entries",
"size". Auditor is right that this is too permissive: arithmetic-overflow
panics (`"size of i128 overflow"`) and bounds-check panics (`"index out of
bounds"` contains "limit") slip through and get silently classified as
acceptable budget exhaustion. The auditor's proposed exclusion of
`overflow` / `out of bounds` plus tighter positive substrings (e.g.
`budget exceeded`, `cpu instruction`, `memory limit`, `read entries`,
`write entries`, `tx size`) is sound.

The 5/5 → 32/32 gap is also genuine. As-is, the bench tests 5/32 = 15.6%
of the absolute ceiling. The Phase-1 recommendation to file a tracking
issue and add an `#[ignore]`d 32/32 sibling is reasonable, with the caveat
that an `#[ignore]`d test that is never wired up has the same broken-F1
problem as `prop_flash_loan_success_repayment` — the patch should make
sure the ignored test compiles AND has a clear path to un-ignore (e.g.,
guarded behind a feature flag that the harness enables once enough preset
markets land).

---

## New findings (not in Phase 1)

### NEW-1: `fuzz_strategy_flashloan.rs::ShortAggregator` is dead code

**Severity:** weak (F1)

Lines 71-104 define a `ShortAggregator` contract that returns 1% less than
`amount_out_min`. Lines 351-354 acknowledge it via
`#[allow(dead_code)] fn _unused_short_aggregator_hook()` — explicitly
silencing the dead-code warning. The doc block at lines 64-69 advertises
`ShortAggregator` as the M-09 / M-11 regression probe ("the caller reads
the *actual* balance delta rather than trusting `amount_out_min`"), but
no proptest in this file (or anywhere else in `test-harness/tests/`)
instantiates it.

This is a documentation-vs-implementation gap. The file claims to cover
M-09 (saturating_sub-hides-aggregator-underpay) but the property test
machinery for that finding is wired off. M-09 has zero proptest coverage
in this domain.

**Patch (suggested):** instantiate `ShortAggregator` in a new property test
that swaps it in for the default `MockAggregator` and asserts the strategy
swap path either (a) detects the 1% shortfall via a balance-delta error or
(b) silently accepts a partial swap (the M-09 regression). Or, if the
finding has been fixed by `swap_collateral`'s post-balance check, delete
the `ShortAggregator` and the doc claim.

---

## Summary

The auditor's Phase 1 report is largely well-grounded. Eight findings are
confirmed at the cited lines with the correct rubric mapping. Four
"refined" entries needed tightening of either the proposed fix (TTL import
cleanup, conservation pre-seeding) or the rubric framing (chaos test was
F5/naming, not F1). One refute: the chaos HF check fix as written
(`hf == f64::INFINITY`) would break the test against a `1.7e20` no-debt
value the harness actually returns. One new finding: `ShortAggregator` is
dead code, leaving M-09 without proptest coverage.

The constants the auditor referenced check out:
- `controller/src/config.rs:219-220` does enforce `> 32` rejection.
- `controller/src/access.rs:82-85` does default to 10/10.
- `common/src/constants.rs:74-76` does set `MAX_SUPPLY_POSITIONS = 4`.
- `common/src/constants.rs:63` does set `TTL_BUMP_INSTANCE = ONE_DAY_LEDGERS * 180`.

All 7 proptest files have tracked `.proptest-regressions` siblings (F3
clean), and none of the proptest configs disable shrinking or pin a
fixed seed (F4 clean).
