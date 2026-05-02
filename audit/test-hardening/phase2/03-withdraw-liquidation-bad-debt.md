# Domain 3 — Phase 2 Review (Withdraw + Liquidation + Bad Debt)

**Phase:** 2 (independent reviewer of phase-1 audit)
**Reviewed:** `audit/test-hardening/phase1/03-withdraw-liquidation-bad-debt.md`
**Source files re-read:**
- `test-harness/tests/withdraw_tests.rs`
- `test-harness/tests/liquidation_tests.rs`
- `test-harness/tests/liquidation_coverage_tests.rs`
- `test-harness/tests/liquidation_math_tests.rs`
- `test-harness/tests/liquidation_mixed_decimal_tests.rs`
- `test-harness/tests/bad_debt_index_tests.rs`
- `test-harness/tests/lifecycle_regression_tests.rs`
- `test-harness/src/{user,view,assert,liquidation,reference/liquidation,context}.rs`
- `controller/src/positions/liquidation.rs`, `controller/src/utils.rs`, `controller/src/oracle/mod.rs`, `controller/src/router.rs`, `controller/src/config.rs`
- `common/src/errors.rs`, `common/src/constants.rs`

**Totals:** confirmed=24 refuted=0 refined=6 new=2

---

## Phase 1 Findings — Disposition

### `withdraw_tests.rs::test_withdraw_multiple_assets`
**Disposition:** confirmed
**Severity:** weak
The test supplies USDC + ETH and withdraws partials but never asserts the wallet received the withdrawn tokens (rubric item 4). `assert_balance_eq` is the right helper. Patch is correct.

---

### `withdraw_tests.rs::test_withdraw_allowed_without_borrows`
**Disposition:** confirmed
**Severity:** weak
Confirmed: line 130-133 only asserts supply balance, never the wallet delta. `assert_balance_eq(ALICE, "USDC", 10_000.0)` would close the gap. Note: `test_withdraw_full_amount_returned` (lines 213-231) already exercises the same wallet-delta behavior, so this finding is real but partially redundant.

---

### `liquidation_tests.rs::test_liquidation_basic_proportional`
**Disposition:** confirmed
**Severity:** weak
Confirmed: lines 41-60 verify only liquidator-side state. Borrower's debt reduction (`borrow_balance(ALICE, "ETH") < 3.0`) and collateral seizure (`supply_balance(ALICE, "USDC") < 10_000`) are never read. Patch is sound.

---

### `liquidation_tests.rs::test_liquidation_targeted_single_collateral`
**Disposition:** confirmed
**Severity:** weak
Confirmed: lines 67-90 only verify liquidator collateral receipt. The test also lacks a debt-reduction or HF check on the borrower side. Patch addresses both.

---

### `liquidation_tests.rs::test_liquidation_dynamic_bonus_moderate`
**Disposition:** confirmed
**Severity:** weak
Confirmed: lines 132-149 only assert `collateral_received_usd > 2000.0`. Test name promises "dynamic bonus" but no quantitative bonus rate is computed. Lines 135-136 capture `_debt_before` and `_collateral_before` but discard them. The 4-16% bound suggested in the patch matches the formula at HF≈0.67 (gap≈0.343, 2*gap clamp=0.686, bonus = 500 + 1000*0.686 ≈ 1186 BPS = 11.86%) — well within the proposed bounds.

---

### `liquidation_tests.rs::test_liquidation_dynamic_bonus_deep_underwater`
**Disposition:** confirmed
**Severity:** weak
Confirmed: lines 174-178 only check `liq_usdc > 0`. With USDC at $0.25 and ETH debt unchanged, the bonus should saturate near the 15% cap. The patch's `bonus_rate > 0.10` lower bound is reasonable.

---

### `liquidation_tests.rs::test_liquidation_protocol_fee_on_bonus_only`
**Disposition:** confirmed
**Severity:** weak
Confirmed: line 200-205 assertion `rev_after >= rev_before` is satisfied even by zero increase. The patch upgrades to `assert_revenue_increased_since` (which strictly requires `current > snapshot` — see `assert.rs:313-322`) and adds a relative magnitude check. Note: a `<1% of total seizure` bound is plausible since `liquidation_fees_bps=100` and the fee applies only to the bonus portion (see `controller/src/positions/liquidation.rs` and the reference impl at `test-harness/src/reference/liquidation.rs:553-558`). Confirmed.

---

### `liquidation_tests.rs::test_liquidation_liquidator_profit`
**Disposition:** confirmed
**Severity:** weak
Confirmed: pure mirror of `test_liquidation_basic_proportional` — only liquidator-side checks. Patch is identical pattern.

---

### `liquidation_tests.rs::test_liquidation_multi_debt_payment`
**Disposition:** refined
**Severity:** weak
Confirmed: the test name is aspirational. Lines 250-263 use `liquidate(...)` and `liquidate(...)` (two separate single-payment calls), then asserts only `liq_usdc > 0`. The patch's pre/post debt comparison strengthens the test, but the auditor's narrative claim that the test "exercises the excess-payment dedup path" is incorrect. The test calls `liquidate()` twice (sequential single calls), not `liquidate_multi(&[(ETH, 0.5), (ETH, 0.3)])`. The two calls are independent — there is no dedup path involved. The test is simply two consecutive partial liquidations with poor assertions.

**Reviewer note:** the patch itself is correct; only the auditor's "why" narrative is wrong. The fixed test should also rename: it is currently NOT a "multi debt payment" test (it's "sequential partial liquidations" — single debt asset, two consecutive calls). Either rename to `test_sequential_partial_liquidations` or restructure the body to use `liquidate_multi(&[("ETH", 0.5), ("USDC", 50.0)])` with a two-asset borrow, matching the name.

**Patch (refined):**
```diff
--- before
+++ after
@@ -231,7 +231,7 @@
 // ---------------------------------------------------------------------------
-// 9. test_liquidation_multi_debt_payment
+// 9. test_liquidation_sequential_partial_liquidations
 // ---------------------------------------------------------------------------

 #[test]
-fn test_liquidation_multi_debt_payment() {
+fn test_liquidation_sequential_partial_liquidations() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
@@ -249,16 +249,22 @@
     // First liquidation.
-    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
+    let debt_before = t.borrow_balance(ALICE, "ETH");
+    t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
+    let debt_after_first = t.borrow_balance(ALICE, "ETH");
+    assert!(debt_after_first < debt_before, "1st liquidation must reduce debt");

     // Check whether still liquidatable for a second pass.
     if t.can_be_liquidated(ALICE) {
         t.liquidate(LIQUIDATOR, ALICE, "ETH", 0.3);
+        assert!(t.borrow_balance(ALICE, "ETH") < debt_after_first,
+            "2nd liquidation must reduce debt further");
     }

     // The liquidator should have accumulated collateral.
     let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
     assert!(
         liq_usdc > 0.0,
         "liquidator should receive collateral from liquidation(s)"
     );
+    assert!(t.supply_balance(ALICE, "USDC") < 10_000.0,
+        "Alice USDC collateral must be seized");
 }
```

---

### `liquidation_tests.rs::test_liquidation_caps_at_actual_debt`
**Disposition:** refined
**Severity:** weak
Confirmed: line 275-282 only checks `liq_usdc > 0`. Patch validates the cap by checking liquidator's residual ETH. Verified by reading `controller/src/positions/liquidation.rs:60-66` — the comment confirms a "pull-model" where excess minted tokens are NOT pulled (liquidator keeps them in their wallet). So `try_liquidate(LIQUIDATOR, ALICE, "ETH", 100.0)` mints 100 ETH but only ~3 ETH (× bonus correction) gets pulled.

**Reviewer note:** the auditor's wording "leftover ETH balance must reflect the refund" is technically wrong — there's no refund, just untransferred mint. But the assertion (`liq_eth_left > 100 - debt_before - 0.01`) is correct because the harness mints 100 ETH up-front (`liquidation.rs:25`) and only some of it gets transferred out. The patch is sound; just clarify the comment.

**Patch (refined):**
```diff
--- before
+++ after
@@ -266,18 +266,30 @@
 // ---------------------------------------------------------------------------
 // 10. test_liquidation_caps_at_actual_debt
 // ---------------------------------------------------------------------------

 #[test]
 fn test_liquidation_caps_at_actual_debt() {
     let mut t = setup_liquidatable();

-    // Try to repay far more debt than the account owes. The liquidation
-    // must cap repayment at the real debt amount.
+    // Repay more than the actual debt. The contract uses a pull-model:
+    // it transfers only the post-cap repayment from the liquidator's
+    // wallet, so the unused mint stays with the liquidator.
+    let debt_before = t.borrow_balance(ALICE, "ETH"); // ~3.0 ETH
     t.liquidate(LIQUIDATOR, ALICE, "ETH", 100.0);

+    // Liquidator started with 100 ETH minted (see harness `liquidate`).
+    // The contract pulls at most `debt_before * (1+bonus)` worth.
+    let liq_eth_left = t.token_balance(LIQUIDATOR, "ETH");
+    assert!(
+        liq_eth_left > 100.0 - debt_before - 0.01,
+        "unused mint (~{}) must stay with liquidator; got {}",
+        100.0 - debt_before, liq_eth_left
+    );
+    // Borrower's debt was paid down (proves repayment was capped, not lost).
+    assert!(t.borrow_balance(ALICE, "ETH") < debt_before,
+        "Alice's ETH debt must have decreased");
+
     let liq_usdc = t.token_balance(LIQUIDATOR, "USDC");
     assert!(
         liq_usdc > 0.0,
         "liquidator should have received USDC collateral: {}",
         liq_usdc
     );
 }
```

---

### `liquidation_tests.rs::test_liquidation_proportional_multi_collateral`
**Disposition:** confirmed
**Severity:** nit
Confirmed: line 297 supplies only USDC. Body comment at line 304 acknowledges "with single collateral, all seizure comes from that asset". This is a single-collateral test with a multi-collateral name (rubric 5). Lines 313-319 do assert `debt_after < 3.0` so item 3 is satisfied. Rename patch is appropriate.

---

### `liquidation_tests.rs::test_liquidation_caps_at_max_bonus`
**Disposition:** confirmed
**Severity:** weak
Confirmed: lines 372-393 verify upper-bound cap `ratio <= 1.16` but never confirm the bonus actually saturated near the cap (a base-rate bonus of ~5% would also pass). Adding `ratio >= 1.10` and a borrower-side assertion is reasonable.

---

### `liquidation_tests.rs::test_liquidation_bad_debt_cleanup_auto`
**Disposition:** confirmed
**Severity:** weak
Confirmed: lines 401-427 only check `liq_usdc > 0`. The "automatic bad-debt cleanup" claim demands `assert_no_positions(ALICE)` and account-removal verification, which the patch adds correctly.

---

### `liquidation_tests.rs::test_liquidation_bad_debt_socializes_loss`
**Disposition:** confirmed
**Severity:** weak
Confirmed: lines 433-460 never verify socialization. Without a second supplier (Bob) on the borrowed-asset side (ETH), there's no one to socialize the loss to. The patch correctly adds `t.supply(test_harness::BOB, "ETH", 100.0)` (BOB is re-exported via `presets::*` per `test-harness/src/lib.rs:23`). The reference impl at `test-harness/src/reference/liquidation.rs:17-20` notes bad-debt socialization is *not* modeled in the reference, so this gap can only be tested via direct supply-balance reads — exactly what the patch does.

---

### `liquidation_tests.rs::test_liquidation_rejects_empty_debt_payments`
**Disposition:** confirmed
**Severity:** broken
Confirmed: line 530-531 uses bare `is_err()`. Verified the panic origin: `aggregate_payment_amount` in `controller/src/utils.rs:87-89` panics with `GenericError::AmountMustBePositive` when `amount == 0` and `zero_is_withdraw_all=false` (liquidate path uses `aggregate_positive_payments`). Code 14 is correct (`common/src/errors.rs`). Patch is sound.

---

### `liquidation_coverage_tests.rs::test_liquidation_skips_excess_debt_payments`
**Disposition:** refined
**Severity:** weak
Confirmed: lines 29-37 only assert `debt > 0.0`. Test name implies dedup but the actual mechanic is **payment summing** (`aggregate_payments` in `controller/src/utils.rs:81-99` calls `checked_add` to combine duplicate-asset entries — line 96-98), then capping at the ideal repayment. So `[("ETH", 2.0), ("ETH", 0.1)]` becomes a single `("ETH", 2.1)` payment, not two payments where the second is "skipped".

**Reviewer note:** the auditor's "dedup must ensure debt_reduction < 2.5 ETH" comment in the patch confuses the actual mechanic. The patch's assertion `reduction <= 2.0 + 0.01` is still correct (because the protocol caps at the ideal repayment, which here is well below 2.0 ETH given Alice's $5,000 debt), but the explanation should reference the cap, not the dedup. Test name should be `test_liquidation_aggregates_duplicate_asset_payments` or the assertion should be retitled.

**Patch (refined):**
```diff
--- before
+++ after
@@ -28,11 +28,17 @@
+    let debt_before = t.borrow_balance(alice, "ETH");
     t.liquidate_multi(LIQUIDATOR, alice, &[("ETH", 2.0), ("ETH", 0.1)]);

     let debt = t.borrow_balance(alice, "ETH");
     assert!(
         debt > 0.0,
         "Alice should have significant debt left, got {}",
         debt
     );
+    // Duplicate asset entries are summed (aggregate_positive_payments) into a
+    // single 2.1 ETH payment, then capped at the ideal repayment. The actual
+    // debt reduction must therefore be at most the ideal cap (~$1500/$2000 =
+    // ~0.75 ETH for this scenario), which is well below the summed 2.1 ETH.
+    let reduction = debt_before - debt;
+    assert!(reduction < 2.0,
+        "summed payment must be capped at ideal repayment: reduction={}", reduction);
 }
```

---

### `liquidation_coverage_tests.rs::test_liquidation_seize_proportional_dust_collateral`
**Disposition:** confirmed
**Severity:** weak
Confirmed: lines 87-90 only assert `eth_bal > 0.0` (i.e., dust ETH still exists). The "seize proportional dust collateral" claim requires evidence that dust *was* touched. Patch reads pre/post both collaterals — correct.

---

### `liquidation_math_tests.rs::test_bonus_formula_at_specific_hf_levels`
**Disposition:** refined
**Severity:** weak
Confirmed: line 95-130 has only "Case 1" despite the plural name. The auditor's tightening from `(500..=700)` to `(550..=600)` is reasonable per the formula (gap=(1.02-0.987)/1.02 ≈ 0.0324, 2*gap ≈ 0.0647, bonus = 500 + 1000*0.0647 ≈ 564.7).

**Reviewer note:** the patch is fine, but the auditor never proposes the *additional cases* the plural test name promises (e.g., HF=0.90, HF=0.70, HF=0.40). A complete fix would add cases for moderate (~0.85) and deep (~0.50) HF — otherwise the plural name is still aspirational. Recommend either renaming to `test_bonus_formula_at_near_threshold_hf` (singular, matches the body) or adding cases.

**Patch (refined, additive):**
```diff
--- before
+++ after
@@ -94,7 +94,8 @@
 #[test]
 fn test_bonus_formula_at_specific_hf_levels() {
-    // bonus = base + (max - base) * min(2 * gap, 1).
-    // gap = (1.02 - HF) / 1.02.
-    // base = 500 BPS (5%), max = 1500 BPS (15%, capped).
+    // bonus = base + (max - base) * min(2 * gap, 1), gap = (1.02 - HF) / 1.02.
+    // base=500 BPS, max=1500 BPS. Verify three points along the curve.
@@ -127,5 +128,30 @@
     assert!(
-        (500..=700).contains(&estimate.bonus_rate_bps),
+        (550..=600).contains(&estimate.bonus_rate_bps),
         "near-threshold HF should give bonus ~500-700 BPS, got {}",
         estimate.bonus_rate_bps
     );
+
+    // Case 2: HF ≈ 0.85 -> gap ≈ 0.167 -> 2*gap ≈ 0.333 -> bonus ≈ 833 BPS.
+    t.set_price("USDC", usd_cents(64));
+    let estimate2 = t.ctrl_client()
+        .liquidation_estimations_detailed(&account_id, &payments);
+    assert!((800..=900).contains(&estimate2.bonus_rate_bps),
+        "moderate HF should give ~833 BPS, got {}", estimate2.bonus_rate_bps);
+
+    // Case 3: HF ≈ 0.5 -> 2*gap ≥ 1 -> bonus saturates at max = 1500 BPS.
+    t.set_price("USDC", usd_cents(38));
+    let estimate3 = t.ctrl_client()
+        .liquidation_estimations_detailed(&account_id, &payments);
+    assert_eq!(estimate3.bonus_rate_bps, 1500,
+        "deep underwater must saturate at max bonus");
 }
```

---

### `liquidation_math_tests.rs::test_hf_improves_quantitatively`
**Disposition:** confirmed
**Severity:** broken
**Confirmed via direct source read of lines 209-219:**
```
let _hf_after = t.health_factor(ALICE);          // line 209 -- discarded
let debt_before = t.total_debt(ALICE);           // line 212 -- AFTER liquidate (line 207)
let debt_after = t.total_debt(ALICE);            // line 213 -- AFTER liquidate
assert!(debt_after <= debt_before, ...)          // line 214 -- vacuous: x <= x
```
Both reads occur after the liquidation. The test name promises HF improvement but `_hf_after` is intentionally discarded. The only "core invariant" assertion is trivially true. Patch correctly captures `debt_before` and `collateral_before` BEFORE liquidation and asserts strict decrease + HF improvement.

---

### `liquidation_math_tests.rs::test_bad_debt_index_decrease_exact`
**Disposition:** confirmed
**Severity:** weak
Confirmed: lines 326-343 only check `actual_ratio > 0.999 && actual_ratio < 1.0` and `bob_loss in [0, 0.005)`. The exact formula `new_index = old_index * (total - bad_debt) / total` is mentioned in the comment but never compared. Patch adds a tighter quantitative bound.

---

### `liquidation_math_tests.rs::test_multiple_partial_liquidations_incremental_hf`
**Disposition:** confirmed
**Severity:** weak
Confirmed: lines 351-407 capture `debt_0..debt_3` but no HF reads between liquidations. Test name promises "incremental HF" but only debt is tracked. Patch adds `hf_0..hf_3` reads and monotonic HF assertions.

---

### `liquidation_math_tests.rs::test_liquidation_bounded_by_available_collateral`
**Disposition:** confirmed
**Severity:** weak
Confirmed: line 436 assertion `liquidator_usdc <= 1_001.0` is trivially true since Alice supplied only 1000 USDC. The "bound" never actually binds (the liquidator can never receive more than what existed). Patch upgrades to assert collateral was drained — that's the meaningful bad-debt-path signal.

---

### `liquidation_mixed_decimal_tests.rs::test_bad_debt_cleanup_mixed_decimals`
**Disposition:** confirmed
**Severity:** broken
**Confirmed via source read of lines 322-349:** the test ends with three comment lines (lines 345-348) explicitly stating "Just verify no panic occurred". There is no `assert!`, `assert_eq!`, or other assertion call after `t.liquidate(...)` on line 343. The test is vacuous. Patch's three assertions (`assert_no_positions`, supply==0, borrow==0) are the right post-state for the bad-debt cleanup path.

---

### `liquidation_mixed_decimal_tests.rs::test_liquidation_protocol_fee_cross_decimal`
**Disposition:** confirmed
**Severity:** weak
Confirmed: lines 358-394 check collateral decrease and debt decrease but never read `snapshot_revenue("USDC6")` despite the test name claiming protocol-fee verification. Patch correctly adds revenue snapshot pre/post and uses `assert_revenue_increased_since`.

---

### `bad_debt_index_tests.rs::test_bad_debt_index_floored_at_safety_floor`
**Disposition:** confirmed
**Severity:** weak
Confirmed: lines 144-172 only check `si_after >= SUPPLY_INDEX_FLOOR_RAW`. With `SUPPLY_INDEX_FLOOR_RAW = WAD = 10^18` (per `common/src/constants.rs:22`) and the base supply index at RAY (10^27), the floor is 10^9× below the starting index. The assertion essentially only fails if the index goes near zero. Patch's added "Bob's stake shrank" check verifies the floor mattered (as opposed to a no-op liquidation). Confirmed.

---

### `bad_debt_index_tests.rs::test_bad_debt_reduction_matches_formula`
**Disposition:** confirmed
**Severity:** weak
Confirmed: lines 296-326 — only `bob_loss in (0, 0.01)` is asserted. The "matches formula" claim demands a quantitative computation against the documented formula. Patch reads Alice's residual debt and computes expected bad-debt amount, then compares within tolerance.

---

### `lifecycle_regression_tests.rs::test_disabled_market_blocks_supply_and_borrow`
**Disposition:** refined
**Severity:** broken
Confirmed: lines 27-36 use bare `is_err()`. **However, the auditor's suggested error code is wrong.** Verified via source:

- `disable_token_oracle` (`controller/src/config.rs:596-600`) sets `market.status = MarketStatus::Disabled`.
- The price-fetch path checks status first (`controller/src/oracle/mod.rs:37-39`):
  ```rust
  MarketStatus::Disabled if !cache.allow_disabled_market_price => {
      panic_with_error!(cache.env(), GenericError::PairNotActive);
  }
  ```
- `GenericError::PairNotActive` is **code 12** (`common/src/errors.rs:21`), NOT 216 (`ORACLE_NOT_CONFIGURED`).

`PAIR_NOT_ACTIVE` is also missing from the harness errors module (`test-harness/src/assert.rs:24-82`) — the constant must be added there too.

**Reviewer note:** the auditor's diagnosis (broken — bare `is_err()`) stands; the suggested code constant is incorrect.

**Patch (refined):**
```diff
--- before
+++ after
@@ -1,3 +1,4 @@
+use test_harness::{assert_contract_error, errors};
 // ... (also requires adding `PAIR_NOT_ACTIVE: u32 = 12;` to
 //      test-harness/src/assert.rs::errors module)
@@ -24,12 +25,8 @@
     t.supply(ALICE, "USDC", 10_000.0);

     let supply_result = t.try_supply(ALICE, "ETH", 1.0);
-    assert!(
-        supply_result.is_err(),
-        "disabled market should block supply"
-    );
+    assert_contract_error(supply_result, errors::PAIR_NOT_ACTIVE);

     let borrow_result = t.try_borrow(ALICE, "ETH", 0.1);
-    assert!(
-        borrow_result.is_err(),
-        "disabled market should block borrow"
-    );
+    assert_contract_error(borrow_result, errors::PAIR_NOT_ACTIVE);
 }
```

---

### `lifecycle_regression_tests.rs::test_disabled_debt_oracle_allows_repay_but_blocks_risk_increasing_ops`
**Disposition:** refined
**Severity:** broken
Confirmed: lines 59-69 use bare `is_err()`. **Same error-code correction as above.** `disable_token_oracle` sets `MarketStatus::Disabled`; subsequent borrow/withdraw price-fetches panic with `GenericError::PairNotActive` (code 12), not `ORACLE_NOT_CONFIGURED` (216).

**Reviewer note:** the `is_ok()` check on `repay_result` at line 53-57 is correct (success path) and not flagged. The auditor's diagnosis is right; only the suggested error code is wrong.

**Patch (refined):**
```diff
--- before
+++ after
@@ -1,3 +1,4 @@
+use test_harness::{assert_contract_error, errors};
@@ -55,15 +56,9 @@

     let borrow_result = t.try_borrow(ALICE, "ETH", 0.1);
-    assert!(
-        borrow_result.is_err(),
-        "disabled debt oracle should block additional borrow"
-    );
+    assert_contract_error(borrow_result, errors::PAIR_NOT_ACTIVE);

     let withdraw_result = t.try_withdraw(ALICE, "USDC", 1_000.0);
-    assert!(
-        withdraw_result.is_err(),
-        "disabled debt oracle should block risk-increasing withdraw"
-    );
+    assert_contract_error(withdraw_result, errors::PAIR_NOT_ACTIVE);
 }
```

---

### `lifecycle_regression_tests.rs::test_create_liquidity_pool_rejects_asset_id_mismatch`
**Disposition:** refined
**Severity:** broken
Confirmed: lines 91-94 use bare `is_err()`. **Auditor's suggested code is wrong.** Source verification:

- `validate_market_creation` in `controller/src/router.rs:91-108`:
  ```rust
  if params.asset_id != *asset {
      panic_with_error!(env, GenericError::WrongToken);
  }
  ```
- `GenericError::WrongToken` is **code 8** (`common/src/errors.rs:17`), NOT `INTERNAL_ERROR` (34).

`WRONG_TOKEN` is missing from the harness errors module and must be added there.

**Patch (refined):**
```diff
--- before
+++ after
@@ -1,3 +1,4 @@
+use test_harness::{assert_contract_error, errors};
 // ... (also requires adding `WRONG_TOKEN: u32 = 8;` to
 //      test-harness/src/assert.rs::errors module)
@@ -90,7 +91,5 @@
-    assert!(
-        result.is_err(),
-        "create_liquidity_pool should reject asset_id mismatch"
-    );
+    assert_contract_error(result, errors::WRONG_TOKEN);
 }
```

---

### `lifecycle_regression_tests.rs::test_create_liquidity_pool_rejects_asset_decimals_mismatch`
**Disposition:** refined
**Severity:** broken
Confirmed: lines 117-120 use bare `is_err()`, gated by `#[ignore]`. **Auditor's suggested code is wrong.** Source verification:

- `validate_market_creation` (`controller/src/router.rs:101-104`):
  ```rust
  #[cfg(not(feature = "testing"))]
  if params.asset_decimals != _token_decimals {
      panic_with_error!(env, GenericError::InvalidAsset);
  }
  ```
- `GenericError::InvalidAsset` is **code 6** (`common/src/errors.rs:15`), NOT `INTERNAL_ERROR` (34).
- Critically: under `feature = "testing"` (which the harness uses), this check is compiled out entirely. So the test will fail to produce *any* error from this branch — that's why it's `#[ignore]`. Even after un-ignoring, the test would only work in a non-testing build.

**Reviewer note:** the auditor missed the `cfg` guard. With it active in test builds, the test will hit `validate_asset_config` or `validate_interest_rate_model` instead, returning a different error. The proper fix requires either:
1. Conditionally pinning the expected code based on the `testing` feature, OR
2. Removing the `#[ignore]` only after the `cfg` guard is reconsidered.

For now, mark the patch as a refined suggestion that pins `INVALID_ASSET` (code 6) and notes the cfg-gate issue:

```diff
--- before
+++ after
@@ -116,7 +116,9 @@
-    assert!(
-        result.is_err(),
-        "create_liquidity_pool should reject asset_decimals mismatch"
-    );
+    use test_harness::{assert_contract_error, errors};
+    // NOTE: this branch is `#[cfg(not(feature = "testing"))]` so it never
+    // fires in standard harness builds — see `controller/src/router.rs:101`.
+    // Ignored test until the cfg-gate is removed or the harness builds
+    // without the `testing` feature.
+    assert_contract_error(result, errors::INVALID_ASSET);
 }
```

---

## New Findings (issues the auditor missed)

### `liquidation_coverage_tests.rs::test_liquidation_rejects_if_no_debt_repaid`
**Disposition:** new
**Severity:** nit
**Why:** line 109 uses `assert_contract_error(result, 14)` — the literal integer `14` instead of `errors::AMOUNT_MUST_BE_POSITIVE`. The constant exists in `test-harness/src/assert.rs:30`. This is a style violation (rubric implicit: use named constants). The test is functionally correct, just inconsistent with the rest of the suite.

**Patch (suggested):**
```diff
--- before
+++ after
@@ -108,5 +108,5 @@
     let result = t.try_liquidate(LIQUIDATOR, "alice_rej", "ETH", 0.000000001);
-    assert_contract_error(result, 14); // AmountMustBePositive.
+    assert_contract_error(result, test_harness::errors::AMOUNT_MUST_BE_POSITIVE);
 }
```

---

### `liquidation_tests.rs::test_liquidation_rejects_when_paused` / `test_liquidation_rejects_during_flash_loan` (cleanup leak)
**Disposition:** new
**Severity:** nit
**Why:** `test_liquidation_rejects_when_paused` (lines 116-124) calls `t.unpause()` after the panic-test, and `test_liquidation_rejects_during_flash_loan` (lines 509-519) calls `t.set_flash_loan_ongoing(false)`. Both calls are dead code in panic-rejection tests because each test has its own fresh `LendingTest` (via `setup_liquidatable()`) — there is no cross-test state to clean up. The trailing calls are confusing scaffolding. Not a correctness bug but degrades readability.

**Reviewer note:** non-blocking; intent is defensive. Could be removed in a follow-up cleanup pass. Not flagging in totals as `weak` because the tests themselves are correct — just slightly noisy.

---

## Summary

The Phase-1 audit is largely accurate: 24 of 30 findings are confirmed without modification. The auditor correctly identified the two most severe vacuous tests (`test_hf_improves_quantitatively` and `test_bad_debt_cleanup_mixed_decimals`) — both verified against the source. The cross-cutting pattern (rubric-3 failures: liquidator-only assertions ignoring borrower post-state) is real and well-traced.

Six findings needed refinement, concentrated in two areas:
1. **All four `lifecycle_regression_tests.rs` broken findings have wrong error-code suggestions.** The auditor proposed `ORACLE_NOT_CONFIGURED` (216) or `INTERNAL_ERROR` (34), but reading the controller source shows the actual codes are `PAIR_NOT_ACTIVE` (12), `WRONG_TOKEN` (8), and `INVALID_ASSET` (6). The harness `errors` module is missing all three constants. Diagnoses (broken — bare `is_err()`) are correct; only the codes need correction.
2. **Two test names are misdescribed in the auditor's narrative** (`test_liquidation_multi_debt_payment` is actually sequential single liquidations; `test_liquidation_skips_excess_debt_payments` actually exercises payment summing, not skipping). The proposed assertions still hold, but the "why" narrative confuses the actual mechanic.

Two new findings surfaced: a style nit on a literal error code, and a redundant cleanup-after-panic pattern.

The reference impl (`test-harness/src/reference/liquidation.rs`) is in scope but never used by any of the audited tests — every "differential" assertion the auditor proposes uses direct view-helper reads (`borrow_balance`, `supply_balance`, `total_debt`), not `compute_liquidation`. That's appropriate per the test rubric, but it means the bigrational reference is currently underutilized for these scenarios.
