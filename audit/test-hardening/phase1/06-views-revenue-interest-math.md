# Domain 6 — Views + Revenue + Interest + Math

**Phase:** 1
**Files in scope:**
- `test-harness/tests/views_tests.rs`
- `test-harness/tests/revenue_tests.rs`
- `test-harness/tests/interest_tests.rs`
- `test-harness/tests/interest_rigorous_tests.rs`
- `test-harness/tests/rewards_rigorous_tests.rs`
- `test-harness/tests/pool_revenue_edge_tests.rs`
- `test-harness/tests/pool_coverage_tests.rs`
- `test-harness/tests/math_rates_tests.rs`
- `test-harness/tests/utils_tests.rs`

**Totals:** broken=1 weak=6 nit=2

---

## `views_tests.rs`

### `views_tests.rs::test_total_collateral_usd_multi_asset`

**Severity:** none

### `views_tests.rs::test_total_borrow_usd_multi_asset`

**Severity:** none

### `views_tests.rs::test_collateral_amount_for_missing_token`

**Severity:** none

### `views_tests.rs::test_borrow_amount_for_missing_token`

**Severity:** none

### `views_tests.rs::test_can_be_liquidated_boundary`

**Severity:** none

### `views_tests.rs::test_can_be_liquidated_just_below`

**Severity:** none

### `views_tests.rs::test_get_all_markets_multiple`

**Severity:** none

### `views_tests.rs::test_get_all_markets_single`

**Severity:** none

### `views_tests.rs::test_get_account_owner_correct`

**Severity:** none

### `views_tests.rs::test_get_emode_category_view`

**Severity:** none

### `views_tests.rs::test_get_isolated_debt_tracks_borrows`

**Severity:** none

### `views_tests.rs::test_get_position_limits_default`

**Severity:** none

### `views_tests.rs::test_get_position_limits_custom`

**Severity:** none

### `views_tests.rs::test_liquidation_estimations_basic`

**Severity:** none

### `views_tests.rs::test_get_market_index_view`

**Severity:** none

### `views_tests.rs::test_get_active_accounts_multiple`

**Severity:** none

### `views_tests.rs::test_get_asset_config_view`

**Severity:** none

### `views_tests.rs::test_pool_address_view`

**Severity:** none

### `views_tests.rs::test_collateral_amount_for_token_happy`

**Severity:** none

### `views_tests.rs::test_borrow_amount_for_token_happy`

**Severity:** none

### `views_tests.rs::test_liquidation_collateral_available_happy`

**Severity:** none

### `views_tests.rs::test_ltv_collateral_in_usd_happy`

**Severity:** none

---

## `revenue_tests.rs`

### `revenue_tests.rs::test_claim_revenue_after_interest`

**Severity:** none

### `revenue_tests.rs::test_claim_revenue_routes_through_controller_to_accumulator`

**Severity:** none

### `revenue_tests.rs::test_claim_revenue_after_liquidation`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** The test asserts that pool revenue increases across a liquidation + 30-day accrual window (lines 138-144), but never verifies token movement. Liquidation pays the protocol the liquidation fee in the borrowed asset, and `claim_revenue` would move tokens from pool to accumulator; neither flow is asserted. The current shape conflates "interest accrued during 30 days post-liquidation" with "liquidation generated fees", because both feed the same scaled-revenue counter. Adding a fee-only delta (snapshot revenue immediately before and after liquidation, prior to advancing time) and a token-balance delta on the accumulator would tighten this to a real liquidation-fee assertion.

**Patch (suggested):**
```diff
--- before
+++ after
@@ -116,6 +116,12 @@
 fn test_claim_revenue_after_liquidation() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
         .build();

     // Alice supplies and borrows near the limit.
     t.supply(ALICE, "USDC", 10_000.0);
     t.borrow(ALICE, "ETH", 3.0); // ~$6000 debt

     let revenue_before_liq = t.snapshot_revenue("ETH");

     // Drop USDC to trigger liquidation.
     t.set_price("USDC", usd_cents(50));
     t.assert_liquidatable(ALICE);

-    // Liquidate: generates fees.
+    // Liquidate: must increase scaled revenue immediately from the
+    // protocol-fee leg of the seizure (independent of subsequent accrual).
     t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
+    let revenue_post_liq = t.snapshot_revenue("ETH");
+    assert!(
+        revenue_post_liq > revenue_before_liq,
+        "liquidation should bump revenue by the liquidation fee: before={}, after={}",
+        revenue_before_liq,
+        revenue_post_liq
+    );

     // Advance time so interest accrues on remaining positions.
     t.advance_and_sync(days(30));

     let revenue_after_liq = t.snapshot_revenue("ETH");
     assert!(
-        revenue_after_liq > revenue_before_liq,
+        revenue_after_liq > revenue_post_liq,
         "revenue should increase after liquidation: before={}, after={}",
-        revenue_before_liq,
+        revenue_post_liq,
         revenue_after_liq
     );
+
+    // Verify the claim moves tokens from pool to accumulator.
+    let accumulator = t
+        .env
+        .register(test_harness::mock_reflector::MockReflector, ());
+    t.ctrl_client().set_accumulator(&accumulator);
+    let asset = t.resolve_market("ETH").asset.clone();
+    let tok = soroban_sdk::token::Client::new(&t.env, &asset);
+    let acc_before = tok.balance(&accumulator);
+    let claimed = t.claim_revenue("ETH");
+    let acc_after = tok.balance(&accumulator);
+    assert!(claimed > 0, "expected non-zero claim; got {}", claimed);
+    assert_eq!(
+        acc_after - acc_before,
+        claimed,
+        "accumulator must receive the full claimed amount"
+    );
 }
```

### `revenue_tests.rs::test_claim_revenue_zero_when_no_activity`

**Severity:** none

### `revenue_tests.rs::test_add_rewards_increases_supply_index`

**Severity:** none

### `revenue_tests.rs::test_add_rewards_rejects_zero`

**Severity:** none

### `revenue_tests.rs::test_revenue_role_required`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Lines 244-256 assert only `result.is_err()` for both `try_claim_revenue` and `try_add_rewards`. Any panic — wrong asset, malformed payload, contract-internal bug — would also satisfy the assertion, so the test passes whether or not access control is the cause of the rejection. The protocol uses OpenZeppelin `AccessControlError::Unauthorized = 2000` for missing-role panics; pinning the code via the existing `assert_contract_error` helper turns this into a real RBAC test. The test does not exercise `#[should_panic]`, so it is not "broken" per the rubric, but rubric item 3 (post-state asserted = the specific rejection reason) is not met.

**Patch (suggested):**
```diff
--- before
+++ after
@@ -1,7 +1,7 @@
 extern crate std;

 use test_harness::{
-    days, errors, eth_preset, usd_cents, usdc_preset, LendingTest, ALICE, BOB, LIQUIDATOR,
+    assert_contract_error, days, errors, eth_preset, usd_cents, usdc_preset, LendingTest, ALICE, BOB, LIQUIDATOR,
 };

@@ -233,6 +233,8 @@
 fn test_revenue_role_required() {
     let mut t = LendingTest::new().with_market(usdc_preset()).build();

     // Create Bob without the REVENUE role.
     let bob_addr = t.get_or_create_user(BOB);

     let ctrl = t.ctrl_client();
     let asset = t.resolve_market("USDC").asset.clone();

-    // Bob tries claim_revenue.
+    // Bob tries claim_revenue: must hit AccessControl Unauthorized (2000).
     let assets = soroban_sdk::vec![&t.env, asset.clone()];
     let result = ctrl.try_claim_revenue(&bob_addr, &assets);
-    assert!(
-        result.is_err(),
-        "non-revenue user should not be able to claim revenue"
-    );
+    let err = result.expect_err("non-revenue user must not be able to claim revenue");
+    let err = err.expect("expected contract error from try_claim_revenue");
+    assert_contract_error::<()>(Err(err.into()), 2000);

-    // Bob tries add_rewards.
+    // Bob tries add_rewards: must also hit Unauthorized (2000).
     let rewards = soroban_sdk::vec![&t.env, (asset, 100i128)];
     let result = ctrl.try_add_rewards(&bob_addr, &rewards);
-    assert!(
-        result.is_err(),
-        "non-revenue user should not be able to add rewards"
-    );
+    let err = result.expect_err("non-revenue user must not be able to add rewards");
+    let err = err.expect("expected contract error from try_add_rewards");
+    assert_contract_error::<()>(Err(err.into()), 2000);
 }
```

---

## `interest_tests.rs`

### `interest_tests.rs::test_interest_accrues_on_borrow`

**Severity:** none

### `interest_tests.rs::test_interest_accrues_on_supply`

**Severity:** none

### `interest_tests.rs::test_interest_rate_increases_with_utilization`

**Severity:** none

### `interest_tests.rs::test_compound_interest_over_multiple_periods`

**Severity:** none

### `interest_tests.rs::test_interest_zero_when_no_borrows`

**Severity:** none

### `interest_tests.rs::test_reserve_factor_splits_interest`

**Severity:** none

### `interest_tests.rs::test_advance_time_without_sync_stale`

**Severity:** none

### `interest_tests.rs::test_advance_and_sync_specific_markets`

**Severity:** none

---

## `interest_rigorous_tests.rs`

### `interest_rigorous_tests.rs::test_borrow_index_matches_compound_formula`

**Severity:** none

### `interest_rigorous_tests.rs::test_supply_index_reflects_interest_minus_reserve_factor`

**Severity:** none

### `interest_rigorous_tests.rs::test_interest_accounting_identity`

**Severity:** none

### `interest_rigorous_tests.rs::test_reserve_factor_exact_split`

**Severity:** none

### `interest_rigorous_tests.rs::test_scaled_amount_times_index_equals_actual`

**Severity:** none

### `interest_rigorous_tests.rs::test_rate_curve_three_regions`

**Severity:** none

### `interest_rigorous_tests.rs::test_single_vs_multi_sync_taylor_accuracy`

**Severity:** none

### `interest_rigorous_tests.rs::test_supply_index_unchanged_without_borrows`

**Severity:** none

### `interest_rigorous_tests.rs::test_multiple_suppliers_share_proportionally`

**Severity:** none

### `interest_rigorous_tests.rs::test_interest_grows_with_time_checkpoints`

**Severity:** none

### `interest_rigorous_tests.rs::test_pool_solvency_invariant`

**Severity:** none

### `interest_rigorous_tests.rs::test_index_values_accessible_and_rational`

**Severity:** none

---

## `rewards_rigorous_tests.rs`

### `rewards_rigorous_tests.rs::test_add_rewards_index_increase_matches_formula`

**Severity:** none

### `rewards_rigorous_tests.rs::test_add_rewards_distributed_proportionally`

**Severity:** none

### `rewards_rigorous_tests.rs::test_add_rewards_does_not_affect_borrow_index`

**Severity:** none

### `rewards_rigorous_tests.rs::test_add_rewards_compounds_over_multiple_calls`

**Severity:** none

### `rewards_rigorous_tests.rs::test_add_rewards_rejects_when_no_supply`

**Severity:** none

### `rewards_rigorous_tests.rs::test_rewards_plus_interest_compound`

**Severity:** none

### `rewards_rigorous_tests.rs::test_large_rewards_accounting_stable`

**Severity:** none

### `rewards_rigorous_tests.rs::test_four_suppliers_exact_proportional_split`

**Severity:** none

### `rewards_rigorous_tests.rs::test_rewards_after_interest_proportional`

**Severity:** none

---

## `pool_revenue_edge_tests.rs`

### `pool_revenue_edge_tests.rs::test_add_rewards_rejects_after_full_withdrawal`

**Severity:** none

### `pool_revenue_edge_tests.rs::test_claim_revenue_else_branch_when_reserves_fully_drained`

**Severity:** none

---

## `pool_coverage_tests.rs`

### `pool_coverage_tests.rs::test_pool_claim_revenue_burns_supplied_ray_coverage`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Lines 4-41 assert that revenue exists, the claim returns positive, and the snapshot revenue clears to zero, but never verify token movement to the accumulator. The accumulator was registered at line 8-11 but its balance is never read, so the test would still pass if the controller silently dropped tokens or kept them on its own balance. Since "claim_revenue burns" is exactly what this test name promises, asserting accumulator+pool balance deltas equal `claimed` is the load-bearing check.

**Patch (suggested):**
```diff
--- before
+++ after
@@ -1,4 +1,4 @@
-use test_harness::{eth_preset, usdc_preset, LendingTest, ALICE, BOB};
+use test_harness::{usdc_preset, LendingTest, ALICE, BOB};

 #[test]
 fn test_pool_claim_revenue_burns_supplied_ray_coverage() {
@@ -29,13 +29,28 @@
     // Check that revenue exists (snapshot_revenue reads the pool's internal
     // state).
     let rev = t.snapshot_revenue("USDC");
     assert!(rev > 0, "Expected some revenue after 1 year");

-    // 5. Claim revenue. This must hit pool/src/lib.rs:401.
+    // 5. Claim revenue. Snapshot pool + accumulator balances to verify the
+    // tokens flow pool -> controller (transient) -> accumulator.
+    let asset = t.resolve_market("USDC").asset.clone();
+    let pool_addr = t.resolve_market("USDC").pool.clone();
+    let tok = soroban_sdk::token::Client::new(&t.env, &asset);
+    let pool_before = tok.balance(&pool_addr);
+    let acc_before = tok.balance(&accumulator);
+
     let claimed = t.claim_revenue("USDC");
     assert!(claimed > 0, "Should have claimed some revenue");

+    let pool_after = tok.balance(&pool_addr);
+    let acc_after = tok.balance(&accumulator);
+    assert_eq!(
+        pool_before - pool_after,
+        claimed,
+        "pool must release exactly the claimed amount"
+    );
+    assert_eq!(
+        acc_after - acc_before,
+        claimed,
+        "accumulator must receive the full claimed amount"
+    );
+
     // Verify the pool burned the revenue.
     let rev_after = t.snapshot_revenue("USDC");
     assert_eq!(rev_after, 0);
 }
```

### `pool_coverage_tests.rs::test_pool_claim_revenue_proportional_burn_when_reserves_low`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Lines 44-88 verify the cap (`claimed == res_raw`) and that revenue stays positive after a partial burn, but again do not verify any token motion. Asserting that the accumulator received exactly `claimed` and the pool released exactly `claimed` makes the coverage test an end-to-end correctness test instead of a "ran the line" coverage stub.

**Patch (suggested):**
```diff
--- before
+++ after
@@ -69,11 +69,28 @@
     // Reserves are now near 0. Ensure accrued revenue exceeds reserves.
     let rev = t.snapshot_revenue("USDC");
     let res_raw = t.pool_client("USDC").reserves();
     assert!(
         rev > res_raw,
         "Revenue {} must be > reserves {} to hit proportional burn",
         rev,
         res_raw
     );

+    // Snapshot balances so we can verify the partial transfer.
+    let asset = t.resolve_market("USDC").asset.clone();
+    let pool_addr = t.resolve_market("USDC").pool.clone();
+    let tok = soroban_sdk::token::Client::new(&t.env, &asset);
+    let pool_before = tok.balance(&pool_addr);
+    let acc_before = tok.balance(&accumulator);
+
     let claimed = t.claim_revenue("USDC");

     // For coverage, this only needs to run.
     assert!(claimed > 0);
     assert_eq!(claimed, res_raw); // Capped at reserves.

+    let pool_after = tok.balance(&pool_addr);
+    let acc_after = tok.balance(&accumulator);
+    assert_eq!(
+        pool_before - pool_after,
+        claimed,
+        "pool must release exactly `claimed` (capped at reserves)"
+    );
+    assert_eq!(
+        acc_after - acc_before,
+        claimed,
+        "accumulator must receive exactly `claimed`"
+    );
+
     // Verify the proportional burn reduced but did not clear the revenue.
     let rev_remaining = t.snapshot_revenue("USDC");
     assert!(rev_remaining > 0);
 }
```

---

## `math_rates_tests.rs`

### `math_rates_tests.rs::test_rescale_same_decimals`

**Severity:** none

### `math_rates_tests.rs::test_rescale_upscale`

**Severity:** none

### `math_rates_tests.rs::test_rescale_downscale_half_up`

**Severity:** none

### `math_rates_tests.rs::test_mul_div_half_up_zero`

**Severity:** none

### `math_rates_tests.rs::test_mul_div_half_up_precision_boundary`

**Severity:** none

### `math_rates_tests.rs::test_div_half_up_exact`

**Severity:** none

### `math_rates_tests.rs::test_div_half_up_rounds_up`

**Severity:** none

### `math_rates_tests.rs::test_mul_half_up_signed_negative`

**Severity:** none

### `math_rates_tests.rs::test_div_half_up_signed_negative`

**Severity:** none

### `math_rates_tests.rs::test_div_by_int_half_up`

**Severity:** none

### `math_rates_tests.rs::test_min_max_equal`

**Severity:** broken
**Rubric items failed:** [2, 3]
**Why:** Lines 156-162 are tautologies: `assert_eq!(5, 5)` and `assert_eq!(-3, -3)` exercise no production code, no `min`/`max` function is called, and the test would pass even if `cmp::min` and `cmp::max` were both `panic!()`. There is no action under test (item 2 fails — the panic origin would never be the function under test, because no function is under test) and no post-state being asserted (item 3). The test is also misnamed: nothing is asserted to be "equal" except literals already known equal at compile time. Since the rubric flags `#[should_panic]` without `expected = ...` as broken for being non-functional, an asserts-nothing test is the same hazard. Recommend deleting outright (no `min`/`max` helper exists in `common::fp_core` worth re-testing here) — or replacing with real coverage of `Ray::min`/`Ray::max` if that is the intent.

**Patch (suggested):**
```diff
--- before
+++ after
@@ -152,15 +152,3 @@
-// ---------------------------------------------------------------------------
-// 11. test_min_max_equal
-// ---------------------------------------------------------------------------
-
-#[test]
-fn test_min_max_equal() {
-    assert_eq!(5, 5);
-    assert_eq!(5, 5);
-    assert_eq!(-3, -3);
-    assert_eq!(-3, -3);
-}
-
 // ===========================================================================
 // Rates edge cases
 // ===========================================================================
```

### `math_rates_tests.rs::test_borrow_rate_zero_utilization`

**Severity:** none

### `math_rates_tests.rs::test_borrow_rate_at_mid_utilization`

**Severity:** none

### `math_rates_tests.rs::test_borrow_rate_at_optimal_utilization`

**Severity:** none

### `math_rates_tests.rs::test_borrow_rate_full_utilization`

**Severity:** none

### `math_rates_tests.rs::test_borrow_rate_capped_at_max`

**Severity:** nit
**Rubric items failed:** [5]
**Why:** Lines 248-258 advertise "capped at max" but actually exercise `Ray * 90 / 100` (90% utilization, region 3). The assertion compares the resulting per-ms rate to `max_borrow_rate_ray / MILLISECONDS_PER_YEAR` and tolerates 1 ulp, which only passes because at 90% util the analytical rate (`1 + 4 + 10 + (90-80)*300/(100-80) = 165%`) is already capped down to `max_borrow_rate_ray = 100%`. The name suggests the test verifies the cap kicks in; the inputs verify the cap kicks in *plus* the full region-3 formula. Renaming to `test_borrow_rate_clamped_in_region_three` (or covering the cap directly with util close to 1.0) describes what is actually checked. Pure rename, no behavior change.

**Patch (suggested):**
```diff
--- before
+++ after
@@ -246,11 +246,11 @@
 // ---------------------------------------------------------------------------
-// 16. test_borrow_rate_capped_at_max
+// 16. test_borrow_rate_clamped_in_region_three
 // ---------------------------------------------------------------------------

 #[test]
-fn test_borrow_rate_capped_at_max() {
+fn test_borrow_rate_clamped_in_region_three() {
     let env = Env::default();
     let params = make_test_params();

     let rate = calculate_borrow_rate(&env, Ray::from_raw(RAY * 90 / 100), &params);
     let max_rate = div_by_int_half_up(RAY, MILLISECONDS_PER_YEAR as i128);
     assert!((rate.raw() - max_rate).abs() <= 1);
 }
```

### `math_rates_tests.rs::test_deposit_rate_zero_utilization`

**Severity:** none

### `math_rates_tests.rs::test_deposit_rate_with_reserve_factor`

**Severity:** none

### `math_rates_tests.rs::test_compound_interest_zero_delta`

**Severity:** none

### `math_rates_tests.rs::test_compound_interest_one_year`

**Severity:** none

### `math_rates_tests.rs::test_utilization_zero_supply`

**Severity:** none

### `math_rates_tests.rs::test_utilization_over_one`

**Severity:** none

### `math_rates_tests.rs::test_supply_index_update_zero_rewards`

**Severity:** none

### `math_rates_tests.rs::test_supply_index_update_with_rewards`

**Severity:** none

### `math_rates_tests.rs::test_borrow_index_update`

**Severity:** none

### `math_rates_tests.rs::test_supplier_rewards_split`

**Severity:** none

### `math_rates_tests.rs::test_scaled_to_original_basic`

**Severity:** none

### `math_rates_tests.rs::test_compound_interest_small_rate`

**Severity:** none

---

## `utils_tests.rs`

### `utils_tests.rs::test_isolated_debt_non_isolated_account`

**Severity:** none

### `utils_tests.rs::test_isolated_debt_dust_erasure`

**Severity:** none

### `utils_tests.rs::test_isolated_debt_over_repay_clamps`

**Severity:** none

### `utils_tests.rs::test_validate_healthy_passes`

**Severity:** none

### `utils_tests.rs::test_validate_healthy_fails`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Lines 134-156 assert `result.is_err()` for the failing withdraw (line 153) but never pin the contract error. Withdrawing from an unhealthy account hits `validate_health_factor` in `controller/src/validation.rs:82` which panics with `CollateralError::InsufficientCollateral = 100`. Without that pin, a future regression that returns a *different* error (e.g., `HealthFactorTooLow`, `AmountMustBePositive` from a typo, an oracle staleness panic) would also pass. Use the existing `assert_contract_error` helper.

**Patch (suggested):**
```diff
--- before
+++ after
@@ -1,7 +1,7 @@
 extern crate std;

 use common::constants::WAD;

-use test_harness::{eth_preset, usd, usd_cents, usdc_preset, LendingTest, ALICE};
+use test_harness::{assert_contract_error, errors, eth_preset, usd, usd_cents, usdc_preset, LendingTest, ALICE};
@@ -148,12 +148,9 @@
     let hf = t.health_factor(ALICE);
     assert!(hf < 1.0, "HF should be < 1.0 after price drop, got {}", hf);

-    // Attempting to withdraw must fail due to low HF.
+    // Attempting to withdraw must fail with InsufficientCollateral (100).
     let result = t.try_withdraw(ALICE, "USDC", 1.0);
-    assert!(
-        result.is_err(),
-        "withdraw should fail when HF is below threshold"
-    );
+    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
 }
```

### `utils_tests.rs::test_health_factor_no_debt_is_max`

**Severity:** none

### `utils_tests.rs::test_health_factor_changes_with_price`

**Severity:** none

### `utils_tests.rs::test_pool_borrow_rate_increases_with_borrows`

**Severity:** nit
**Rubric items failed:** [5]
**Why:** Section header at line 202 reads "8. test_pool_utilization_increases_with_borrows" but the function on line 206 is named `test_pool_borrow_rate_increases_with_borrows`. The function name is correct (rate is what's measured at line 224), but the comment misleads readers searching by section. Pure documentation drift; pick one and align. `grep` confirms no other test references the comment text, so realigning the comment is safe.

**Patch (suggested):**
```diff
--- before
+++ after
@@ -200,7 +200,7 @@
 // ---------------------------------------------------------------------------
-// 8. test_pool_utilization_increases_with_borrows
+// 8. test_pool_borrow_rate_increases_with_borrows
 // ---------------------------------------------------------------------------

 #[test]
 fn test_pool_borrow_rate_increases_with_borrows() {
```

### `utils_tests.rs::test_borrow_exceeds_ltv_fails`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Lines 232-245 assert only `result.is_err()` after attempting an LTV-exceeding borrow. The borrow should fail with `CollateralError::InsufficientCollateral = 100` (see `controller/src/positions/borrow.rs:429`). A bare `is_err()` would still pass if the borrow failed because of a pricing bug, an account-not-found regression, or any other contract panic. Pinning the code via `assert_contract_error` makes the test guard the LTV check specifically.

**Patch (suggested):**
```diff
--- before
+++ after
@@ -232,11 +232,12 @@
 #[test]
 fn test_borrow_exceeds_ltv_fails() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
         .build();

     // Supply $10k USDC, LTV=75% => max borrow = $7500.
     t.supply(ALICE, "USDC", 10_000.0);

-    // Borrow 4 ETH = $8000 > $7500.
+    // Borrow 4 ETH = $8000 > $7500: must trip InsufficientCollateral (100).
     let result = t.try_borrow(ALICE, "ETH", 4.0);
-    assert!(result.is_err(), "borrow exceeding LTV should fail");
+    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
 }
```

### `utils_tests.rs::test_total_debt_zero_after_full_repay`

**Severity:** none

---

## Cross-cutting patterns

The domain is in good shape overall — 91/100 tests pass the rubric cleanly, including every entry in `interest_rigorous_tests.rs`, `rewards_rigorous_tests.rs`, `pool_revenue_edge_tests.rs`, `views_tests.rs`, and `interest_tests.rs`. The systemic gaps cluster in two places. First, three tests fall back on `assert!(result.is_err())` instead of pinning the contract error code, even though the harness already exposes `assert_contract_error` and an `errors` module covering every domain panic — `revenue_tests::test_revenue_role_required`, `utils_tests::test_validate_healthy_fails`, and `utils_tests::test_borrow_exceeds_ltv_fails` are all easily upgraded by importing the helper. Second, the two `pool_coverage_tests` and `revenue_tests::test_claim_revenue_after_liquidation` exercise revenue flows but never assert token-balance deltas at the pool/accumulator boundary, leaving silent-token-drop regressions invisible. The single broken test, `math_rates_tests::test_min_max_equal`, is a literal tautology that asserts nothing about production code and should be deleted (or rewritten against `Ray::min`/`Ray::max` if those exist). Two nits are pure naming drift (a stale comment header in `utils_tests.rs` and a misleading function name for region-3 clamping in `math_rates_tests.rs`).
