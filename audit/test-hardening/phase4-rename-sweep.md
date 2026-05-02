# Phase 4 — Misleading-name sweep

**Files scanned:** 49
**Misleading names found:** 4
**Stale section headers found:** 4

## Misleading names

### `liquidation_math_tests.rs::test_hf_improves_quantitatively` (line 191)

**Current:** `test_hf_improves_quantitatively`
**Proposed:** `test_liquidation_does_not_increase_debt`
**Why:** Body at lines 209, 213-219 binds `_hf_after` (discarded with leading underscore) and never asserts anything about HF; the only assertions are `debt_after <= debt_before` (which compares `total_debt` to itself read twice post-liquidation — see lines 212-214) and `ratio > 0.0`. The test does not verify HF improvement, quantitatively or otherwise.
**Patch:**

```diff
--- a/test-harness/tests/liquidation_math_tests.rs
+++ b/test-harness/tests/liquidation_math_tests.rs
@@ -186,11 +186,11 @@
 // ---------------------------------------------------------------------------
-// 4. Verify HF improves after liquidation (quantitative)
+// 4. Verify liquidation does not increase debt
 // ---------------------------------------------------------------------------

 #[test]
-fn test_hf_improves_quantitatively() {
+fn test_liquidation_does_not_increase_debt() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
```

### `liquidation_tests.rs::test_liquidation_rejects_empty_debt_payments` (line 582)

**Current:** `test_liquidation_rejects_empty_debt_payments`
**Proposed:** `test_liquidation_rejects_zero_amount`
**Why:** Body at line 586 calls `t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.0)` with a single non-empty payment whose amount is 0.0 — the trigger is "amount must be positive" (`AMOUNT_MUST_BE_POSITIVE`, line 587), not an empty payment vector. The comment at line 585 even says "Use an exact zero payment". An empty vector test would call `liquidate_multi` with `&[]`.
**Patch:**

```diff
--- a/test-harness/tests/liquidation_tests.rs
+++ b/test-harness/tests/liquidation_tests.rs
@@ -577,11 +577,11 @@
 // ---------------------------------------------------------------------------
-// 18. test_liquidation_rejects_empty_debt_payments
+// 18. test_liquidation_rejects_zero_amount
 // ---------------------------------------------------------------------------

 #[test]
-fn test_liquidation_rejects_empty_debt_payments() {
+fn test_liquidation_rejects_zero_amount() {
     let mut t = setup_liquidatable();

     // Use an exact zero payment. `0.0000001` ETH stays non-zero at 7 decimals.
```

### `borrow_tests.rs::test_borrow_health_factor_exactly_one` (line 394)

**Current:** `test_borrow_health_factor_exactly_one`
**Proposed:** `test_borrow_at_ltv_limit_stays_healthy`
**Why:** Comment at line 405 computes "HF = (10_000 * 0.80) / 7500 = 1.0667" (HF≈1.07, not 1.0) and assertion at lines 416-420 expects `(1.0..1.15).contains(&hf)` — i.e., HF can be anywhere up to 1.15 to pass, never "exactly one". The scenario borrows at the LTV limit ($7500 of $10k, 75% LTV) and verifies the resulting position is healthy via the higher liquidation threshold (80%).
**Patch:**

```diff
--- a/test-harness/tests/borrow_tests.rs
+++ b/test-harness/tests/borrow_tests.rs
@@ -389,11 +389,11 @@
 // ---------------------------------------------------------------------------
-// 14. test_borrow_health_factor_exactly_one
+// 14. test_borrow_at_ltv_limit_stays_healthy
 // ---------------------------------------------------------------------------

 #[test]
-fn test_borrow_health_factor_exactly_one() {
+fn test_borrow_at_ltv_limit_stays_healthy() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(usdt_stable_preset())
```

### `views_tests.rs::test_can_be_liquidated_just_below` (line 128)

**Current:** `test_can_be_liquidated_just_below`
**Proposed:** `test_can_be_liquidated_when_unhealthy`
**Why:** "Just below" implies a marginal HF, but the body's commentary at lines 141-142 reads "HF = 4000/6000 = 0.67 < 1.0" — HF≈0.67 is far below 1.0, not "just below". The test crashes USDC by 50% to $0.50 (line 143), a deliberate large drop. There is no boundary-case configuration; the body verifies a clearly-unhealthy account is reported as liquidatable.
**Patch:**

```diff
--- a/test-harness/tests/views_tests.rs
+++ b/test-harness/tests/views_tests.rs
@@ -123,11 +123,11 @@
 // ---------------------------------------------------------------------------
-// 6. test_can_be_liquidated_just_below
+// 6. test_can_be_liquidated_when_unhealthy
 // ---------------------------------------------------------------------------

 #[test]
-fn test_can_be_liquidated_just_below() {
+fn test_can_be_liquidated_when_unhealthy() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
```

## Stale section headers

These are section banner comments above already-renamed functions; the comment still references the old name even though the `fn` line has been updated.

### `admin_config_tests.rs:522`

Old comment: `// 15. test_init_market_uniqueness`
New comment: `// 15. test_create_liquidity_pool_uniqueness`

### `smoke_test.rs:207`

Old comment: `// 7. test_revenue_snapshot`
New comment: `// 7. test_revenue_accrues_over_time`

(This is the section header above one of the already-fixed renames listed in the prompt; the function was renamed but the banner comment was not.)

### `utils_tests.rs:202`

Old comment: `// 8. test_pool_utilization_increases_with_borrows`
New comment: `// 8. test_pool_borrow_rate_increases_with_borrows`

### `views_tests.rs:182`

Old comment: `// 8. test_get_all_markets_empty_count`
New comment: `// 8. test_get_all_markets_single`
