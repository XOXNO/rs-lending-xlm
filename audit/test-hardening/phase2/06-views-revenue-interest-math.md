# Domain 6 — Views + Revenue + Interest + Math (Phase 2 Review)

**Phase:** 2 (independent review)
**Auditor totals (claimed):** broken=1 weak=6 nit=2
**Reviewer totals:** confirmed=7 refuted=1 refined=1 new=2

Files re-read fresh against `audit/test-hardening/prompts/phase1-audit-pragmatic.md`.

---

## Phase 1 entries reviewed

### `revenue_tests.rs::test_claim_revenue_after_liquidation` — auditor: weak [4]

**Disposition:** confirmed

The test (lines 116-145) only snapshots revenue at `before_liq` and after a `30-day` advance, then asserts a single increase. The increase conflates the liquidation fee with 30 days of accrued interest on the remaining position; if the liquidation fee leg silently regressed to zero, the 30-day accrual would still satisfy the assertion. The auditor's patch correctly inserts a `revenue_post_liq` snapshot taken immediately after `liquidate(...)` and before `advance_and_sync(...)`, which isolates the fee. The accumulator-balance assertions added at the tail are also valuable: `test_claim_revenue_routes_through_controller_to_accumulator` (lines 62-109) covers token motion for an interest-only claim but never for a post-liquidation claim, so they probe a distinct code path.

The patch compiles cleanly: `setup_accumulator` is a free function (line 8) but the patch creates the accumulator inline, which is consistent with the routing test's pattern and avoids re-using the helper twice. No conflict with the existing `setup_accumulator` helper because the patch never calls it.

### `revenue_tests.rs::test_revenue_role_required` — auditor: weak [3]

**Disposition:** confirmed

`only_role` macro expansion (`stellar-macros-0.7.1/src/access_control.rs:49`) calls `stellar_access::access_control::ensure_role(...)`, which `panic_with_error!`s `AccessControlError::Unauthorized = 2000` (`vendor/openzeppelin/stellar-access/src/access_control/storage.rs:608`). The role check happens inside the controller contract, so the soroban harness surfaces the panic as `Err(Ok(soroban_sdk::Error))` for a non-`Result`-returning contract function (`claim_revenue` returns `Vec<i128>`, `add_rewards` returns `()`).

The auditor's patch is shape-correct: `result.expect_err("...")` extracts `Result<soroban_sdk::Error, InvokeError>`, the second `.expect("...")` extracts `soroban_sdk::Error`, and `assert_contract_error::<()>(Err(err.into()), 2000)` pins the code. The pre-existing `test_add_rewards_rejects_zero` (lines 213-225) uses the same nested pattern (`Err(Ok(err)) =>`), confirming the harness shape matches the patch.

Note: `admin_config_tests.rs::test_role_enforcement_revenue` (lines 380-395) has the same `is_err()` weakness and explicitly justifies it with a comment claiming "Soroban wraps cross-contract errors at the outer caller boundary." That comment is misleading — the role check is intra-contract, code 2000 surfaces directly. That is a domain 5 issue, not domain 6, so I am not expanding scope here, but flagging it for cross-domain awareness.

### `pool_coverage_tests.rs::test_pool_claim_revenue_burns_supplied_ray_coverage` — auditor: weak [4]

**Disposition:** confirmed

Lines 4-41 register `accumulator` (lines 8-11) but never read its balance, so a regression where the controller pockets or burns the tokens would still pass. `revenue_tests::test_claim_revenue_routes_through_controller_to_accumulator` covers ETH; this test is the only USDC + 1-year + SpotOnly route. The auditor's patch (snapshot pool/accumulator, claim, assert pool delta == accumulator delta == claimed) is correct and the diff also drops the unused `eth_preset` import properly.

### `pool_coverage_tests.rs::test_pool_claim_revenue_proportional_burn_when_reserves_low` — auditor: weak [4]

**Disposition:** confirmed

This is the only test that drives `claim_revenue` into the proportional-burn branch where `claimed == res_raw < rev`. None of the other revenue tests exercise this path with a token-flow assertion. The auditor's patch (lines 76-91 of the diff) correctly snapshots balances after the reserve drain but before the claim, and asserts `pool_before - pool_after == claimed` and `acc_after - acc_before == claimed`. The cap assertion `claimed == res_raw` already exists at line 83.

### `math_rates_tests.rs::test_min_max_equal` — auditor: broken [2, 3]

**Disposition:** refined

The auditor's diagnosis is correct: lines 156-162 are tautologies (`assert_eq!(5, 5)` etc.), no production code is exercised, and rubric items 2 (action under test) and 3 (post-state asserted) are violated. However, the auditor's prescription says "no `min`/`max` helper exists in `common::fp_core` worth re-testing here" and proposes deleting the test outright.

That is wrong on the existence claim. `common/src/fp.rs:153-167` defines `Wad::min` and `Wad::max` as production functions:

```rust
pub fn min(self, other: Wad) -> Wad {
    if self.0 < other.0 { self } else { other }
}
pub fn max(self, other: Wad) -> Wad {
    if self.0 > other.0 { self } else { other }
}
```

No `Ray::min`/`Ray::max` exists, but `Wad` has both. The better refined patch keeps the slot in the math test file but covers the actual production fns instead of deleting the test name (which is a regression marker readable from CI history). Delete is acceptable; covering `Wad::min`/`Wad::max` is strictly better.

**Refined patch (replace tautology with real Wad min/max coverage):**

```diff
--- a/test-harness/tests/math_rates_tests.rs
+++ b/test-harness/tests/math_rates_tests.rs
@@ -1,9 +1,10 @@
 extern crate std;
 
 use common::constants::{MILLISECONDS_PER_YEAR, RAY, WAD};
-use common::fp::Ray;
+use common::fp::{Ray, Wad};
 use common::fp_core::{
     div_by_int_half_up, mul_div_half_up, mul_div_half_up_signed, rescale_half_up,
 };
 use common::rates::*;
 use soroban_sdk::Env;
@@ -150,15 +151,21 @@ fn test_div_by_int_half_up() {
 // ---------------------------------------------------------------------------
 // 11. test_min_max_equal
 // ---------------------------------------------------------------------------
 
 #[test]
-fn test_min_max_equal() {
-    assert_eq!(5, 5);
-    assert_eq!(5, 5);
-    assert_eq!(-3, -3);
-    assert_eq!(-3, -3);
+fn test_wad_min_max() {
+    let a = Wad::from_raw(5 * WAD);
+    let b = Wad::from_raw(7 * WAD);
+    let c = Wad::from_raw(-3 * WAD);
+
+    assert_eq!(a.min(b), a, "min picks the smaller side");
+    assert_eq!(a.max(b), b, "max picks the larger side");
+    assert_eq!(a.min(a), a, "min is reflexive on equal values");
+    assert_eq!(a.max(a), a, "max is reflexive on equal values");
+    assert_eq!(c.min(a), c, "min handles negatives");
+    assert_eq!(c.max(a), a, "max handles negatives");
 }
 
 // ===========================================================================
 // Rates edge cases
 // ===========================================================================
```

Note: `Wad::from_raw` is the constructor at `common/src/fp.rs:128`. The patch assumes it exists; quick verification confirms it.

If the team prefers the simpler delete-only path, the auditor's original patch is still acceptable — but it leaves a real production fn (`Wad::min`/`Wad::max`) unexercised. Flagging as `refined` rather than `confirmed` because the auditor's rationale ("no helper worth testing") is factually wrong.

### `math_rates_tests.rs::test_borrow_rate_capped_at_max` — auditor: nit [5]

**Disposition:** refuted

**Reviewer note:** The auditor argues the test "advertises 'capped at max' but actually exercises Ray * 90 / 100 (90% utilization, region 3)" and proposes renaming to `test_borrow_rate_clamped_in_region_three`. The argument inverts the relationship between region 3 and the cap.

At util=90%, the region-3 analytical rate is `base + slope1 + slope2 + (util - opt) * slope3 / (1 - opt) = 1% + 4% + 10% + (90% - 80%) * 300% / 20% = 15% + 150% = 165%` annual. `max_borrow_rate_ray = 100%`, so the assertion `rate ≈ max_rate / MS_PER_YEAR` is testing exactly the cap kicking in — not the region-3 formula (which would be `165% / MS_PER_YEAR`, ~1 ulp off). The current name is correct: the cap engages, and that is what the assertion verifies. Renaming to "clamped in region three" mis-describes the assertion (it is the *cap*, not region 3, that the assertion pins).

`test_borrow_rate_full_utilization` (lines 236-244) covers util=100% which also engages the cap — that is the only redundant pair, but it does not justify renaming this one. No change recommended.

### `utils_tests.rs::test_validate_healthy_fails` — auditor: weak [3]

**Disposition:** confirmed

Line 153 (`result.is_err()`) does not pin the contract code. `try_withdraw` returns `Result<(), soroban_sdk::Error>` (verified at `test-harness/src/user.rs:367-386`), which is the exact shape `assert_contract_error` expects. The unhealthy withdraw fails inside `validate_health_factor` with `CollateralError::InsufficientCollateral = 100` (auditor reference checks out — see `test-harness/src/assert.rs:37`). The patch imports `assert_contract_error, errors` from the harness root (`test-harness/src/lib.rs:20` re-exports both) and applies cleanly.

### `utils_tests.rs::test_pool_borrow_rate_increases_with_borrows` — auditor: nit [5]

**Disposition:** confirmed

Line 202 reads `// 8. test_pool_utilization_increases_with_borrows` but the function on line 206 is `test_pool_borrow_rate_increases_with_borrows`. The function name accurately reflects the assertion at line 224 (`pool_borrow_rate`, not utilization). Pure comment drift; auditor's patch realigning the section header is correct and risk-free.

### `utils_tests.rs::test_borrow_exceeds_ltv_fails` — auditor: weak [3]

**Disposition:** confirmed

Line 244 (`result.is_err()`) does not pin the code. The borrow path triggers `CollateralError::InsufficientCollateral = 100` at the post-borrow health validation step. `try_borrow` returns `Result<(), soroban_sdk::Error>` (`test-harness/src/user.rs:275-294`), so `assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL)` works directly. Auditor patch is correct.

---

## All "Severity: none" entries — spot-checked

I re-read each file end-to-end. The "none"-rated tests in `views_tests.rs`, `interest_tests.rs`, `interest_rigorous_tests.rs`, `rewards_rigorous_tests.rs`, `pool_revenue_edge_tests.rs`, `revenue_tests.rs`, and the math/rates portions of `math_rates_tests.rs`/`utils_tests.rs` survive review. The rigorous interest/rewards files in particular are well-targeted (compound formula, accounting identity, reserve-factor split, scaled-amount × index = actual, 3-region rate curve, single-vs-multi sync, solvency invariant, proportional rewards). The view tests assert specific numbers (~$12k collateral, ~$2.6k debt, exact bps configs) rather than mere existence. `pool_revenue_edge_tests.rs` correctly exercises the two pool branches it advertises (NoSuppliersToReward post-withdrawal, claim-with-zero-reserves else branch).

Two issues missed by the auditor are added below as `new` entries.

---

## New findings

### NEW-1 — `views_tests.rs::test_can_be_liquidated_boundary` is mislabeled

**Severity:** nit
**Rubric items failed:** [5]

Lines 105-122 advertise a "boundary" test but the inline comment on line 114 reads `HF = (10000 * 0.80) / 3000 = 2.67: clearly healthy.` That is far from any boundary — HF=2.67 is comfortably in the safe zone. The test functions identically to a vanilla healthy-account check. The companion `test_can_be_liquidated_just_below` (lines 129-149) IS a boundary test (HF=0.67 just below 1.0). The "boundary" test should either be renamed to reflect what it actually exercises (a clearly-healthy case) or be tightened to push HF close to 1.0 from above (e.g., HF=1.05) so the `!can_be_liquidated` assertion guards a real boundary, not an obviously-safe configuration.

**Patch (rename — minimal-diff option):**

```diff
--- a/test-harness/tests/views_tests.rs
+++ b/test-harness/tests/views_tests.rs
@@ -100,11 +100,11 @@ fn test_borrow_amount_for_missing_token() {
 // ---------------------------------------------------------------------------
-// 5. test_can_be_liquidated_boundary
+// 5. test_can_be_liquidated_healthy
 // ---------------------------------------------------------------------------
 
 #[test]
-fn test_can_be_liquidated_boundary() {
+fn test_can_be_liquidated_healthy() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
         .build();
 
     // Supply 10k USDC, borrow conservatively so HF stays above 1.0.
```

(If the team prefers tightening the scenario instead, the alternative is to drop the borrow to ~$7900 ETH so HF lands ~1.01 from above; that genuinely tests the boundary semantics.)

### NEW-2 — `admin_config_tests.rs::test_role_enforcement_revenue` weakness already noted in cross-cutting summary

**Severity:** weak (already domain 5; flagged here only for cross-cutting awareness)

`admin_config_tests.rs:380-395` has the same `is_err()` pattern the auditor flagged in `test_revenue_role_required`, with a comment claiming Soroban wraps cross-contract errors. That comment is misleading (the role check is intra-contract via `only_role`, error code 2000 propagates directly), and the same `assert_contract_error::<()>(Err(err.into()), 2000)` pattern that fixes domain 6 would also fix that test. Domain 5 owns this finding; calling it out so the cross-cutting summary in domain 5's phase 2 review captures it.

---

## Cross-cutting note

The auditor's overall framing in the phase-1 cross-cutting section is accurate: three `is_err()` weaknesses, two pool-coverage tests missing token-flow asserts, one revenue-after-liquidation test that conflates fee with accrual, and the broken `test_min_max_equal`. The miss on `Wad::min`/`Wad::max` is a one-line oversight inside the broken-test prescription, not a directional error. The proposed rename of `test_borrow_rate_capped_at_max` is the only finding I disagree with substantively, because the current name describes the assertion better than the proposed replacement.
