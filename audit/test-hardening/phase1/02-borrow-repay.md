# Domain 2 — Borrow + Repay

**Phase:** 1
**Files in scope:**
- `test-harness/tests/borrow_tests.rs`
- `test-harness/tests/repay_tests.rs`
- `test-harness/tests/oracle_tolerance_tests.rs`

**Totals:** broken=17 weak=19 nit=0

---

## `borrow_tests.rs` (18 tests)

### `borrow_tests.rs::test_borrow_basic`

**Severity:** none

### `borrow_tests.rs::test_borrow_same_asset_xlm`

**Severity:** none

### `borrow_tests.rs::test_borrow_multiple_assets_bulk`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** The test asserts both borrow positions exist and that the account is healthy (lines 81-83), but never checks the post-state magnitude (`assert_borrow_near` for ETH=1.0, WBTC=0.01) and never asserts the wallet received the borrowed tokens. A regression that creates the position rows but routes zero tokens to the borrower (or routes the wrong amounts) would still pass.

**Patch (suggested):**
```diff
--- a/test-harness/tests/borrow_tests.rs
+++ b/test-harness/tests/borrow_tests.rs
@@ -78,9 +78,15 @@ fn test_borrow_multiple_assets_bulk() {
     // Borrow 1 ETH ($2000) and 0.01 WBTC ($600) in one bulk call.
     t.borrow_bulk(ALICE, &[("ETH", 1.0), ("WBTC", 0.01)]);
 
     t.assert_position_exists(ALICE, "ETH", PositionType::Borrow);
     t.assert_position_exists(ALICE, "WBTC", PositionType::Borrow);
+    t.assert_borrow_near(ALICE, "ETH", 1.0, 0.01);
+    t.assert_borrow_near(ALICE, "WBTC", 0.01, 0.0001);
+    let eth_wallet = t.token_balance(ALICE, "ETH");
+    let wbtc_wallet = t.token_balance(ALICE, "WBTC");
+    assert!(eth_wallet > 0.99, "ETH wallet should be ~1.0, got {}", eth_wallet);
+    assert!(wbtc_wallet > 0.0099, "WBTC wallet should be ~0.01, got {}", wbtc_wallet);
     t.assert_healthy(ALICE);
 }
```

### `borrow_tests.rs::test_borrow_duplicate_asset_bulk_accumulates_single_position`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Strong post-state assertions on borrow-count and accumulated borrow balance (lines 96-98), but no token-balance check. The contract must mint 1.25 ETH to Alice; without that delta check, a regression that records the debt but skips one of the two transfers would still pass.

**Patch (suggested):**
```diff
--- a/test-harness/tests/borrow_tests.rs
+++ b/test-harness/tests/borrow_tests.rs
@@ -93,6 +93,12 @@ fn test_borrow_duplicate_asset_bulk_accumulates_single_position() {
     t.supply(ALICE, "USDC", 100_000.0);
     t.borrow_bulk(ALICE, &[("ETH", 0.5), ("ETH", 0.75)]);
 
     t.assert_borrow_count(ALICE, 1);
     t.assert_borrow_near(ALICE, "ETH", 1.25, 0.01);
+    let eth_wallet = t.token_balance(ALICE, "ETH");
+    assert!(
+        eth_wallet > 1.24 && eth_wallet < 1.26,
+        "ETH wallet should be ~1.25 after duplicate-asset borrow, got {}",
+        eth_wallet
+    );
     t.assert_healthy(ALICE);
 }
```

### `borrow_tests.rs::test_borrow_rejects_exceeding_ltv`

**Severity:** none

### `borrow_tests.rs::test_borrow_rejects_zero_amount`

**Severity:** none

### `borrow_tests.rs::test_borrow_rejects_non_borrowable`

**Severity:** none

### `borrow_tests.rs::test_borrow_rejects_during_flash_loan`

**Severity:** none

### `borrow_tests.rs::test_borrow_rejects_when_paused`

**Severity:** none

### `borrow_tests.rs::test_borrow_cap_enforcement`

**Severity:** none

### `borrow_tests.rs::test_borrow_position_limit_exceeded`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Line 238-241 asserts a generic `result.is_err()` with a comment claiming "Soroban wraps the error as InvalidAction on the cross-contract path." That is incorrect — `errors::POSITION_LIMIT_EXCEEDED` (109) is asserted directly via `assert_contract_error` in `strategy_edge_tests.rs:475,1195` and `invariant_tests.rs:217`. A regression that fails for an unrelated reason (e.g. ASSET_NOT_BORROWABLE, INSUFFICIENT_COLLATERAL, FLASH_LOAN_ONGOING) would falsely pass this test.

**Patch (suggested):**
```diff
--- a/test-harness/tests/borrow_tests.rs
+++ b/test-harness/tests/borrow_tests.rs
@@ -232,13 +232,9 @@ fn test_borrow_position_limit_exceeded() {
     t.supply(ALICE, "USDC", 100_000.0);
     t.borrow(ALICE, "ETH", 0.1);
 
-    // The second borrow position must exceed the limit. Soroban wraps the
-    // error as InvalidAction on the cross-contract path.
+    // The second borrow position must exceed the limit.
     let result = t.try_borrow(ALICE, "WBTC", 0.001);
-    assert!(
-        result.is_err(),
-        "borrow exceeding position limit should fail"
-    );
+    assert_contract_error(result, errors::POSITION_LIMIT_EXCEEDED);
 }
```

### `borrow_tests.rs::test_borrow_siloed_asset_blocks_mixed`

**Severity:** none

### `borrow_tests.rs::test_borrow_bulk_rejects_siloed_asset_mixed_in_same_batch`

**Severity:** none

### `borrow_tests.rs::test_borrow_isolated_requires_enabled`

**Severity:** none

### `borrow_tests.rs::test_borrow_isolated_debt_ceiling`

**Severity:** none

### `borrow_tests.rs::test_borrow_emode_enhanced_ltv`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** Asserts healthy and `hf >= 1.0` (lines 359-362) but does not verify the position itself: no `assert_position_exists` for USDT borrow, no `assert_borrow_near(ALICE, "USDT", 9_500.0, …)`, no wallet delta. The test could pass if the controller silently records a smaller-than-requested borrow.

**Patch (suggested):**
```diff
--- a/test-harness/tests/borrow_tests.rs
+++ b/test-harness/tests/borrow_tests.rs
@@ -355,7 +355,11 @@ fn test_borrow_emode_enhanced_ltv() {
     // Standard LTV = 75% caps the normal limit at $7500.
     // E-mode LTV = 97%, so a $9500 borrow stays allowed.
     t.borrow(ALICE, "USDT", 9_500.0);
+    t.assert_position_exists(ALICE, "USDT", PositionType::Borrow);
+    t.assert_borrow_near(ALICE, "USDT", 9_500.0, 1.0);
+    let usdt_wallet = t.token_balance(ALICE, "USDT");
+    assert!(usdt_wallet > 9_499.0, "USDT wallet should be ~9500, got {}", usdt_wallet);
     t.assert_healthy(ALICE);
 
     let hf = t.health_factor(ALICE);
     assert!(hf >= 1.0, "should be healthy with e-mode LTV, HF = {}", hf);
 }
```

### `borrow_tests.rs::test_borrow_health_factor_exactly_one`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Excellent post-state HF assertion (lines 384-390) and `assert_healthy`. Missing: token-balance delta on USDT. A regression that records the debt but does not transfer the borrowed tokens to Alice would still pass.

**Patch (suggested):**
```diff
--- a/test-harness/tests/borrow_tests.rs
+++ b/test-harness/tests/borrow_tests.rs
@@ -380,6 +380,8 @@ fn test_borrow_health_factor_exactly_one() {
     t.borrow(ALICE, "USDT", 7_500.0);
     t.assert_healthy(ALICE);
+    let usdt_wallet = t.token_balance(ALICE, "USDT");
+    assert!(usdt_wallet > 7_499.0, "USDT wallet should hold ~7500, got {}", usdt_wallet);
 
     let hf = t.health_factor(ALICE);
     assert!(
         (1.0..1.15).contains(&hf),
         "HF should be tight (~1.07), got {}",
         hf
     );
 }
```

### `borrow_tests.rs::test_borrow_bulk_passes_cumulative_hf_check`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** Mirrors `test_borrow_multiple_assets_bulk` — only checks position existence + healthy. No `assert_borrow_near` for either asset and no wallet delta. The "cumulative HF check" claim in the test name is exercised by the operation completing without revert, but the test doesn't verify the magnitudes recorded match what was requested.

**Patch (suggested):**
```diff
--- a/test-harness/tests/borrow_tests.rs
+++ b/test-harness/tests/borrow_tests.rs
@@ -407,9 +407,15 @@ fn test_borrow_bulk_passes_cumulative_hf_check() {
     // Borrow small amounts of each in one batch through the harness.
     t.borrow_bulk(ALICE, &[("ETH", 0.5), ("WBTC", 0.005)]);
 
     t.assert_position_exists(ALICE, "ETH", PositionType::Borrow);
     t.assert_position_exists(ALICE, "WBTC", PositionType::Borrow);
+    t.assert_borrow_near(ALICE, "ETH", 0.5, 0.01);
+    t.assert_borrow_near(ALICE, "WBTC", 0.005, 0.0001);
+    let eth_wallet = t.token_balance(ALICE, "ETH");
+    let wbtc_wallet = t.token_balance(ALICE, "WBTC");
+    assert!(eth_wallet > 0.49, "ETH wallet ~0.5, got {}", eth_wallet);
+    assert!(wbtc_wallet > 0.0049, "WBTC wallet ~0.005, got {}", wbtc_wallet);
     t.assert_healthy(ALICE);
 }
```

---

## `repay_tests.rs` (10 tests)

### `repay_tests.rs::test_repay_partial`

**Severity:** none

### `repay_tests.rs::test_repay_full_clears_position`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Strong post-state: borrow ~0 plus `assert_borrow_count(ALICE, 0)` (lines 53-61). However the test does not check the wallet delta on ETH — `t.repay()` auto-mints `amount` then transfers ~1 ETH to the controller, so the net wallet delta should be near 0 (mint-then-burn). A regression that mints but does not transfer would leave Alice +1.01 ETH richer; a regression that transfers without burning the position scaled amount would still satisfy `assert_borrow_count` only if the cleanup runs.

**Patch (suggested):**
```diff
--- a/test-harness/tests/repay_tests.rs
+++ b/test-harness/tests/repay_tests.rs
@@ -45,9 +45,15 @@ fn test_repay_full_clears_position() {
     t.supply(ALICE, "USDC", 10_000.0);
     t.borrow(ALICE, "ETH", 1.0);
 
+    let wallet_before = t.token_balance(ALICE, "ETH");
     // Repay slightly more to clear the position fully.
     t.repay(ALICE, "ETH", 1.01);
 
+    // repay() auto-mints `amount` then the contract pulls the actual debt.
+    // Net wallet delta should be the refunded surplus (~0.01 ETH, since the
+    // outstanding debt was ~1.0).
+    let wallet_after = t.token_balance(ALICE, "ETH");
+    assert!((wallet_after - wallet_before).abs() < 0.05,
+        "wallet delta should be ~0 after exact-repay (auto-mint cancels transfer): before={}, after={}",
+        wallet_before, wallet_after);
+
     let borrow = t.borrow_balance(ALICE, "ETH");
     assert!(
         borrow < 0.01,
         "borrow should be ~0 after full repay, got {}",
         borrow
     );
 
     // The borrow position must be removed.
     t.assert_borrow_count(ALICE, 0);
 }
```

### `repay_tests.rs::test_repay_overpayment_refunded`

**Severity:** none

### `repay_tests.rs::test_repay_by_third_party`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Verifies Alice's borrow is cleared (lines 130-136) but never checks Bob's wallet delta. Bob was minted 1.01 ETH at line 124 and the repay should consume ~1.0 ETH, leaving ~0.01 in Bob's wallet (or whatever the refund is). A regression that fails to charge the third-party payer (or refunds the wrong account) would still pass.

**Patch (suggested):**
```diff
--- a/test-harness/tests/repay_tests.rs
+++ b/test-harness/tests/repay_tests.rs
@@ -120,6 +120,7 @@ fn test_repay_by_third_party() {
     // Mint ETH to Bob so he can pay.
     let repay_amount = 1_0100000i128; // 1.01 ETH (7 decimals)
     eth_market.token_admin.mint(&bob_addr, &repay_amount);
+    let bob_before = t.token_balance(BOB, "ETH");
 
     let ctrl = t.ctrl_client();
     let payments = soroban_sdk::vec![&t.env, (eth_addr, repay_amount)];
@@ -132,6 +133,11 @@ fn test_repay_by_third_party() {
         "ALICE's borrow should be ~0 after BOB's repay, got {}",
         borrow
     );
+    let bob_after = t.token_balance(BOB, "ETH");
+    assert!(bob_before - bob_after >= 0.99,
+        "Bob's wallet must be debited by ~1.0 ETH for Alice's repay: before={}, after={}",
+        bob_before, bob_after);
+    assert_eq!(t.token_balance(ALICE, "ETH"), 1.0,
+        "Alice's wallet must be untouched by Bob's repay");
 }
```

### `repay_tests.rs::test_repay_multiple_assets`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Verifies borrow balances are cleared for ETH and WBTC (lines 174-185) but does not check the caller's wallet delta. Alice was minted exactly enough to cover both repays at lines 165-168, so post-repay wallets should be ~0. A regression that records repayment but does not transfer would leave Alice with the minted balance.

**Patch (suggested):**
```diff
--- a/test-harness/tests/repay_tests.rs
+++ b/test-harness/tests/repay_tests.rs
@@ -163,6 +163,9 @@ fn test_repay_multiple_assets() {
     // Mint tokens for repayment.
     t.resolve_market("ETH").token_admin.mint(&addr, &eth_repay);
     t.resolve_market("WBTC")
         .token_admin
         .mint(&addr, &wbtc_repay);
+    let eth_before = t.token_balance(ALICE, "ETH");
+    let wbtc_before = t.token_balance(ALICE, "WBTC");
 
     let ctrl = t.ctrl_client();
     let payments = soroban_sdk::vec![&t.env, (eth_addr, eth_repay), (wbtc_addr, wbtc_repay)];
@@ -179,6 +182,16 @@ fn test_repay_multiple_assets() {
     assert!(
         wbtc_borrow < 0.0001,
         "WBTC borrow should be cleared, got {}",
         wbtc_borrow
     );
+    let eth_after = t.token_balance(ALICE, "ETH");
+    let wbtc_after = t.token_balance(ALICE, "WBTC");
+    assert!(eth_before - eth_after >= 0.99,
+        "ETH wallet must drop by ~1.0 after repay: before={}, after={}",
+        eth_before, eth_after);
+    assert!(wbtc_before - wbtc_after >= 0.0099,
+        "WBTC wallet must drop by ~0.01 after repay: before={}, after={}",
+        wbtc_before, wbtc_after);
 }
```

### `repay_tests.rs::test_repay_rejects_zero_amount`

**Severity:** none

### `repay_tests.rs::test_repay_rejects_position_not_found`

**Severity:** none

### `repay_tests.rs::test_repay_rejects_during_flash_loan`

**Severity:** none

### `repay_tests.rs::test_repay_isolated_debt_decremented`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Asserts only `debt_after < debt_before` (lines 273-279). For a full repay of 0.5 ETH the isolated debt counter must reach (or near) zero. A regression that decrements by a single wei would still pass. Tighten by asserting `debt_after == 0` (or near-zero) since the test repays 100% of the borrow.

**Patch (suggested):**
```diff
--- a/test-harness/tests/repay_tests.rs
+++ b/test-harness/tests/repay_tests.rs
@@ -270,11 +270,12 @@ fn test_repay_isolated_debt_decremented() {
     t.repay(ALICE, "ETH", 0.5);
 
     let debt_after = t.get_isolated_debt("USDC");
-    assert!(
-        debt_after < debt_before,
-        "isolated debt should decrease after repay: before={}, after={}",
+    // Full repay of the only borrow must zero the isolated counter.
+    assert_eq!(
+        debt_after, 0,
+        "isolated debt should be zero after full repay: before={}, after={}",
         debt_before,
         debt_after
     );
 }
```

### `repay_tests.rs::test_repay_cleans_up_empty_account`

**Severity:** none

---

## `oracle_tolerance_tests.rs` (32 tests)

### `oracle_tolerance_tests.rs::test_safe_price_allows_all_operations`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** The test invokes supply/borrow/repay/withdraw (lines 38-47) but asserts nothing afterwards — no position checks, no balance checks, no HF check. Any regression that silently no-ops one of the four ops would still pass as long as the controller does not panic.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -34,15 +34,21 @@ fn test_safe_price_allows_all_operations() {
     t.set_safe_price("USDC", usd(1), true, true);
     t.set_safe_price("ETH", usd(2000), true, true);
 
     // Supply (risk-decreasing).
     t.supply(ALICE, "USDC", 100_000.0);
+    t.assert_supply_near(ALICE, "USDC", 100_000.0, 1.0);
 
     // Borrow (risk-increasing).
     t.borrow(ALICE, "ETH", 10.0);
+    t.assert_borrow_near(ALICE, "ETH", 10.0, 0.01);
 
     // Repay (risk-decreasing).
     t.repay(ALICE, "ETH", 1.0);
+    t.assert_borrow_near(ALICE, "ETH", 9.0, 0.01);
 
     // Withdraw (risk-increasing when borrows exist).
     t.withdraw(ALICE, "USDC", 1_000.0);
+    t.assert_supply_near(ALICE, "USDC", 99_000.0, 1.0);
+    t.assert_healthy(ALICE);
 }
```

### `oracle_tolerance_tests.rs::test_second_tolerance_allows_risk_decreasing`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Performs supply, borrow, and repay (lines 67-73) but asserts no post-state. As written, the test merely verifies "no panic"; it cannot detect an off-by-one or no-op regression. Add `assert_borrow_near`/`assert_supply_near`/`assert_healthy` after each step.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -65,11 +65,15 @@ fn test_second_tolerance_allows_risk_decreasing() {
     // Supply succeeds (risk-decreasing).
     t.supply(ALICE, "USDC", 100_000.0);
+    t.assert_supply_near(ALICE, "USDC", 100_000.0, 1.0);
 
     // Borrow also succeeds (within second tolerance, uses average price).
     t.borrow(ALICE, "ETH", 10.0);
+    t.assert_borrow_near(ALICE, "ETH", 10.0, 0.01);
+    t.assert_healthy(ALICE);
 
     // Repay succeeds (risk-decreasing).
     t.repay(ALICE, "ETH", 1.0);
+    t.assert_borrow_near(ALICE, "ETH", 9.0, 0.01);
 }
```

### `oracle_tolerance_tests.rs::test_second_tolerance_allows_borrow`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** Confirms only `result.is_ok()` on the borrow (line 90). Does not assert the borrow position was actually recorded, nor the wallet delta. Use `t.borrow(...)` (which panics on err) and then assert post-state to also enforce success-path correctness.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -85,9 +85,12 @@ fn test_second_tolerance_allows_borrow() {
     t.supply(ALICE, "USDC", 100_000.0);
 
     // Borrow succeeds: price deviation is within the second tolerance band.
-    let result = t.try_borrow(ALICE, "ETH", 10.0);
-    assert!(result.is_ok(), "borrow should work within second tolerance");
+    t.try_borrow(ALICE, "ETH", 10.0)
+        .expect("borrow should work within second tolerance");
+    t.assert_borrow_near(ALICE, "ETH", 10.0, 0.01);
+    let eth_wallet = t.token_balance(ALICE, "ETH");
+    assert!(eth_wallet > 9.99, "ETH wallet should be ~10, got {}", eth_wallet);
 }
```

### `oracle_tolerance_tests.rs::test_unsafe_price_allows_supply`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** Only asserts `result.is_ok()` on the supply (line 108). No supply-balance, account-existence, or wallet-delta check. Use `t.supply(...)` and add `assert_supply_near` plus a wallet check.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -103,11 +103,12 @@ fn test_unsafe_price_allows_supply() {
     // Aggregator: $1.00, Safe: $1.10 (10% deviation).
     t.set_safe_price("USDC", usd_cents(110), true, true);
 
     // Supply still succeeds (allow_unsafe_price=true for supply).
-    let result = t.try_supply(ALICE, "USDC", 10_000.0);
-    assert!(result.is_ok(), "supply should work even with unsafe price");
+    t.try_supply(ALICE, "USDC", 10_000.0)
+        .expect("supply should work even with unsafe price");
+    t.assert_supply_near(ALICE, "USDC", 10_000.0, 1.0);
 }
```

### `oracle_tolerance_tests.rs::test_unsafe_price_allows_repay`

**Severity:** none

### `oracle_tolerance_tests.rs::test_unsafe_price_blocks_borrow`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Generic `result.is_err()` (lines 161-165) without a specific code. The expected failure is `OracleError::UnsafePriceNotAllowed = 205` (already used in `withdraw_blocked_under_oracle_deviation_when_debt_exists` at line 270 of the same file). Any unrelated error (e.g. INSUFFICIENT_COLLATERAL, ASSET_NOT_BORROWABLE) would falsely satisfy `is_err()`.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -158,12 +158,12 @@ fn test_unsafe_price_blocks_borrow() {
     // Borrow fails: USDC (collateral) price is unsafe, and borrow uses
     // allow_unsafe_price=false.
-    let result = t.try_borrow(ALICE, "ETH", 10.0);
-    assert!(
-        result.is_err(),
-        "borrow should fail with unsafe collateral price"
-    );
+    let err = t.try_borrow(ALICE, "ETH", 10.0)
+        .expect_err("borrow should fail with unsafe collateral price");
+    assert_eq!(err, soroban_sdk::Error::from_contract_error(
+        common::errors::OracleError::UnsafePriceNotAllowed as u32),
+        "expected UnsafePriceNotAllowed (205), got {:?}", err);
 }
```

### `oracle_tolerance_tests.rs::test_unsafe_price_blocks_borrow_debt_asset`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Same defect as above (lines 184-188): generic `is_err()` instead of asserting `UnsafePriceNotAllowed = 205`.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -181,12 +181,12 @@ fn test_unsafe_price_blocks_borrow_debt_asset() {
     t.set_safe_price("ETH", usd(2200), true, true); // 10% above aggregator
 
     // Borrow fails: ETH (debt asset) price is unsafe.
-    let result = t.try_borrow(ALICE, "ETH", 10.0);
-    assert!(
-        result.is_err(),
-        "borrow should fail with unsafe debt asset price"
-    );
+    let err = t.try_borrow(ALICE, "ETH", 10.0)
+        .expect_err("borrow should fail with unsafe debt asset price");
+    assert_eq!(err, soroban_sdk::Error::from_contract_error(
+        common::errors::OracleError::UnsafePriceNotAllowed as u32),
+        "expected UnsafePriceNotAllowed (205), got {:?}", err);
 }
```

### `oracle_tolerance_tests.rs::test_unsafe_price_blocks_withdraw_with_borrows`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Generic `is_err()` (lines 209-213). The sibling test `withdraw_blocked_under_oracle_deviation_when_debt_exists` already shows the precise pattern with code 205. Tighten this one identically.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -206,11 +206,12 @@ fn test_unsafe_price_blocks_withdraw_with_borrows() {
     // Withdraw fails when the user has borrows (risk-increasing,
     // allow_unsafe_price=false).
-    let result = t.try_withdraw(ALICE, "USDC", 1_000.0);
-    assert!(
-        result.is_err(),
-        "withdraw with borrows should fail with unsafe price"
-    );
+    let err = t.try_withdraw(ALICE, "USDC", 1_000.0)
+        .expect_err("withdraw with borrows should fail with unsafe price");
+    assert_eq!(err, soroban_sdk::Error::from_contract_error(
+        common::errors::OracleError::UnsafePriceNotAllowed as u32),
+        "expected UnsafePriceNotAllowed (205), got {:?}", err);
 }
```

### `oracle_tolerance_tests.rs::withdraw_succeeds_under_oracle_deviation_when_no_debt`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** Asserts only `result.is_ok()` (lines 240-244). Doesn't verify the supply balance dropped by 1000 USDC and the wallet rose by ~1000 USDC.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -237,12 +237,12 @@ fn withdraw_succeeds_under_oracle_deviation_when_no_debt() {
     // With no debt, the withdraw cache runs with allow_unsafe_price=true,
     // and the post-loop health-factor gate short-circuits when no borrows
     // exist. Supply-only users must keep liveness during oracle deviation.
-    let result = t.try_withdraw(ALICE, "USDC", 1_000.0);
-    assert!(
-        result.is_ok(),
-        "withdraw should succeed under oracle deviation when account has no debt: {:?}",
-        result
-    );
+    let wallet_before = t.token_balance(ALICE, "USDC");
+    t.try_withdraw(ALICE, "USDC", 1_000.0)
+        .expect("withdraw should succeed under oracle deviation when account has no debt");
+    t.assert_supply_near(ALICE, "USDC", 99_000.0, 1.0);
+    let wallet_after = t.token_balance(ALICE, "USDC");
+    assert!(wallet_after - wallet_before > 999.0,
+        "wallet should grow by ~1000: before={}, after={}", wallet_before, wallet_after);
 }
```

### `oracle_tolerance_tests.rs::withdraw_blocked_under_oracle_deviation_when_debt_exists`

**Severity:** none

### `oracle_tolerance_tests.rs::test_unsafe_price_blocks_liquidation`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Generic `is_err()` (lines 303-304). The expected failure is `UnsafePriceNotAllowed = 205`. Without that, the test passes whenever liquidation fails for any reason — including a regression where the price is permitted but the position is no longer liquidatable.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -300,9 +300,12 @@ fn test_unsafe_price_blocks_liquidation() {
     // Deviate the safe price beyond tolerance so liquidation is blocked.
     t.set_safe_price("USDC", usd_cents(110), true, true);
 
     // Liquidation fails: allow_unsafe_price=false for liquidate.
-    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
-    assert!(result.is_err(), "liquidation should fail with unsafe price");
+    let err = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0)
+        .expect_err("liquidation should fail with unsafe price");
+    assert_eq!(err, soroban_sdk::Error::from_contract_error(
+        common::errors::OracleError::UnsafePriceNotAllowed as u32),
+        "expected UnsafePriceNotAllowed (205), got {:?}", err);
 }
```

### `oracle_tolerance_tests.rs::test_stale_price_blocks_supply`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Generic `is_err()` (lines 326-329). The expected failure is `OracleError::PriceFeedStale = 206` (already imported as `errors::PRICE_FEED_STALE`).

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -1,4 +1,4 @@
-use test_harness::{eth_preset, usd, usd_cents, usdc_preset, LendingTest, ALICE, LIQUIDATOR};
+use test_harness::{assert_contract_error, errors, eth_preset, usd, usd_cents, usdc_preset, LendingTest, ALICE, LIQUIDATOR};
@@ -322,11 +322,8 @@ fn test_stale_price_blocks_supply() {
     // Supply also fails with a stale price because the oracle adapter's
     // get_price() enforces staleness unconditionally before the controller
     // sees the price.
     let result = t.try_supply(ALICE, "USDC", 1_000.0);
-    assert!(
-        result.is_err(),
-        "supply should fail with stale price (adapter enforces staleness)"
-    );
+    assert_contract_error(result, errors::PRICE_FEED_STALE);
 }
```

### `oracle_tolerance_tests.rs::test_stale_price_blocks_borrow`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Generic `is_err()` (line 343). Expected: `PRICE_FEED_STALE`.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -340,7 +340,7 @@ fn test_stale_price_blocks_borrow() {
     // Borrow fails: stale price blocked for risk-increasing ops.
     let result = t.try_borrow(ALICE, "ETH", 10.0);
-    assert!(result.is_err(), "borrow should fail with stale price");
+    assert_contract_error(result, errors::PRICE_FEED_STALE);
 }
```

### `oracle_tolerance_tests.rs::test_stale_price_blocks_withdraw_with_borrows`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Generic `is_err()` (lines 357-360). Expected: `PRICE_FEED_STALE`.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -354,11 +354,8 @@ fn test_stale_price_blocks_withdraw_with_borrows() {
     // Withdraw fails when borrows exist (risk-increasing).
     let result = t.try_withdraw(ALICE, "USDC", 1_000.0);
-    assert!(
-        result.is_err(),
-        "withdraw with borrows should fail with stale price"
-    );
+    assert_contract_error(result, errors::PRICE_FEED_STALE);
 }
```

### `oracle_tolerance_tests.rs::test_tolerance_at_exact_first_boundary`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** Asserts `result.is_ok()` (lines 386-390). Doesn't verify the borrow recorded at the boundary or the wallet delta.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -384,11 +384,11 @@ fn test_tolerance_at_exact_first_boundary() {
     // At exactly the first boundary, the price stays within first tolerance
     // and uses the safe price directly (most favorable for the user).
-    let result = t.try_borrow(ALICE, "ETH", 10.0);
-    assert!(
-        result.is_ok(),
-        "borrow should work at first tolerance boundary"
-    );
+    t.try_borrow(ALICE, "ETH", 10.0)
+        .expect("borrow should work at first tolerance boundary");
+    t.assert_borrow_near(ALICE, "ETH", 10.0, 0.01);
+    let eth_wallet = t.token_balance(ALICE, "ETH");
+    assert!(eth_wallet > 9.99, "wallet should be ~10 ETH, got {}", eth_wallet);
 }
```

### `oracle_tolerance_tests.rs::test_tolerance_just_beyond_first_boundary`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** Same shape as above — asserts only `result.is_ok()` on borrow (lines 408-412), no post-state.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -406,11 +406,11 @@ fn test_tolerance_just_beyond_first_boundary() {
     // Still succeeds (average price used, within second tolerance).
-    let result = t.try_borrow(ALICE, "ETH", 10.0);
-    assert!(
-        result.is_ok(),
-        "borrow should work between first and second tolerance"
-    );
+    t.try_borrow(ALICE, "ETH", 10.0)
+        .expect("borrow should work between first and second tolerance");
+    t.assert_borrow_near(ALICE, "ETH", 10.0, 0.01);
+    let eth_wallet = t.token_balance(ALICE, "ETH");
+    assert!(eth_wallet > 9.99, "wallet should be ~10 ETH, got {}", eth_wallet);
 }
```

### `oracle_tolerance_tests.rs::test_safe_price_below_aggregator_blocks_borrow`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Generic `is_err()` (lines 429-433). Expected `UnsafePriceNotAllowed = 205` for risk-increasing borrow with deviation > 5%.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -426,11 +426,12 @@ fn test_safe_price_below_aggregator_blocks_borrow() {
     // Beyond second tolerance in the negative direction: blocked.
-    let result = t.try_borrow(ALICE, "ETH", 10.0);
-    assert!(
-        result.is_err(),
-        "borrow should fail with safe price 10% below aggregator"
-    );
+    let err = t.try_borrow(ALICE, "ETH", 10.0)
+        .expect_err("borrow should fail with safe price 10% below aggregator");
+    assert_eq!(err, soroban_sdk::Error::from_contract_error(
+        common::errors::OracleError::UnsafePriceNotAllowed as u32),
+        "expected UnsafePriceNotAllowed (205), got {:?}", err);
 }
```

### `oracle_tolerance_tests.rs::test_tolerance_config_rejects_first_below_min`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Generic `is_err()` (lines 450-453). The exact failure is `OracleError::BadFirstTolerance = 207` (already exposed as `errors::BAD_FIRST_TOLERANCE`).

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -446,11 +446,11 @@ fn test_tolerance_config_rejects_first_below_min() {
     // MIN_FIRST_TOLERANCE = 50 BPS.
-    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &10, &500);
-    assert!(
-        result.is_err(),
-        "first tolerance below 50 BPS should be rejected"
-    );
+    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &10, &500)
+        .expect("invocation should reach the contract")
+        .expect_err("first tolerance below 50 BPS should be rejected");
+    assert_eq!(result, soroban_sdk::Error::from_contract_error(errors::BAD_FIRST_TOLERANCE),
+        "expected BadFirstTolerance (207), got {:?}", result);
 }
```

### `oracle_tolerance_tests.rs::test_tolerance_config_rejects_first_above_max`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Same — generic `is_err()` (lines 465-469). Expected `BadFirstTolerance = 207`.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -463,11 +463,11 @@ fn test_tolerance_config_rejects_first_above_max() {
     // MAX_FIRST_TOLERANCE = 5000 BPS.
-    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &6000, &7000);
-    assert!(
-        result.is_err(),
-        "first tolerance above 5000 BPS should be rejected"
-    );
+    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &6000, &7000)
+        .expect("invocation should reach the contract")
+        .expect_err("first tolerance above 5000 BPS should be rejected");
+    assert_eq!(result, soroban_sdk::Error::from_contract_error(errors::BAD_FIRST_TOLERANCE),
+        "expected BadFirstTolerance (207), got {:?}", result);
 }
```

### `oracle_tolerance_tests.rs::test_tolerance_config_rejects_last_below_min`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Generic `is_err()` (lines 481-485). Expected `OracleError::BadLastTolerance = 208` (not currently exported in `errors::*`; the test should reference 208 directly via `from_contract_error` or `BAD_FIRST_TOLERANCE` is unrelated). Also note: the call passes `&100, &100` — `last < MIN_LAST_TOLERANCE = 150` triggers `BadLastTolerance`.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -479,11 +479,12 @@ fn test_tolerance_config_rejects_last_below_min() {
     // MIN_LAST_TOLERANCE = 150 BPS, first=200 is valid.
-    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &100, &100);
-    assert!(
-        result.is_err(),
-        "last tolerance below 150 BPS should be rejected"
-    );
+    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &100, &100)
+        .expect("invocation should reach the contract")
+        .expect_err("last tolerance below 150 BPS should be rejected");
+    // OracleError::BadLastTolerance = 208.
+    assert_eq!(result, soroban_sdk::Error::from_contract_error(208),
+        "expected BadLastTolerance (208), got {:?}", result);
 }
```

### `oracle_tolerance_tests.rs::test_tolerance_config_rejects_last_above_max`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Generic `is_err()` (lines 497-501). Expected `BadLastTolerance = 208`.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -495,11 +495,11 @@ fn test_tolerance_config_rejects_last_above_max() {
     // MAX_LAST_TOLERANCE = 10000 BPS.
-    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &200, &11000);
-    assert!(
-        result.is_err(),
-        "last tolerance above 10000 BPS should be rejected"
-    );
+    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &200, &11000)
+        .expect("invocation should reach the contract")
+        .expect_err("last tolerance above 10000 BPS should be rejected");
+    assert_eq!(result, soroban_sdk::Error::from_contract_error(208),
+        "expected BadLastTolerance (208), got {:?}", result);
 }
```

### `oracle_tolerance_tests.rs::test_tolerance_config_rejects_last_less_than_first`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Generic `is_err()` (lines 513-517). The expected failure when `last < first` is `OracleError::BadAnchorTolerances = 209`.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -511,11 +511,11 @@ fn test_tolerance_config_rejects_last_less_than_first() {
     // last (200) < first (300): must fail.
-    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &300, &200);
-    assert!(
-        result.is_err(),
-        "last tolerance < first tolerance should be rejected"
-    );
+    let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &300, &200)
+        .expect("invocation should reach the contract")
+        .expect_err("last tolerance < first tolerance should be rejected");
+    // OracleError::BadAnchorTolerances = 209.
+    assert_eq!(result, soroban_sdk::Error::from_contract_error(209),
+        "expected BadAnchorTolerances (209), got {:?}", result);
 }
```

### `oracle_tolerance_tests.rs::test_tolerance_config_valid_update`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Asserts `result.is_ok()` (lines 529-530) but reads no storage to confirm the new tolerance values were actually written. A regression that returns `Ok` without persisting would pass.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -526,9 +526,17 @@ fn test_tolerance_config_valid_update() {
 
     // Valid tolerance update.
     let result = ctrl.try_edit_oracle_tolerance(&admin, &asset, &300, &600);
     assert!(result.is_ok(), "valid tolerance update should succeed");
+
+    // Verify the new tolerance is in storage.
+    let stored: common::types::OraclePriceFluctuation = t.env.as_contract(&t.controller, || {
+        t.env.storage().persistent()
+            .get(&common::types::ControllerKey::OracleTolerance(asset.clone()))
+            .expect("tolerance must be stored")
+    });
+    assert_eq!(stored.first_tolerance_bps, 300);
+    assert_eq!(stored.last_tolerance_bps, 600);
 }
```

### `oracle_tolerance_tests.rs::test_set_accumulator`

**Severity:** none

### `oracle_tolerance_tests.rs::test_set_liquidity_pool_template`

**Severity:** none

### `oracle_tolerance_tests.rs::test_disable_token_oracle_blocks_operations`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Generic `is_err()` (lines 593-597). After disabling the oracle the price returns 0; the resulting borrow should fail with a specific code (likely `OracleError::OracleNotConfigured = 216` or `InvalidPrice = 217`, depending on the path). Pin down the exact code so a regression that fails for a different reason (e.g. INSUFFICIENT_COLLATERAL) doesn't pass.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -589,11 +589,12 @@ fn test_disable_token_oracle_blocks_operations() {
     // The price now returns 0 for USDC, changing HF-sensitive behavior.
     // Borrowing against zero-value collateral must fail.
-    let result = t.try_borrow(ALICE, "ETH", 1.0);
-    assert!(
-        result.is_err(),
-        "borrow should fail when collateral oracle is disabled (price=0)"
-    );
+    let err = t.try_borrow(ALICE, "ETH", 1.0)
+        .expect_err("borrow should fail when collateral oracle is disabled");
+    // Confirm the precise reason. Replace 216 with the expected code after
+    // running the test once and observing the actual contract error.
+    assert_eq!(err, soroban_sdk::Error::from_contract_error(
+        common::errors::OracleError::OracleNotConfigured as u32),
+        "expected OracleNotConfigured (216), got {:?}", err);
 }
```

### `oracle_tolerance_tests.rs::test_edit_asset_in_e_mode_category`

**Severity:** none

### `oracle_tolerance_tests.rs::test_second_tolerance_uses_average_price`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Asserts `assert_healthy` (line 655) but does not verify the supply/borrow balances were recorded at the requested magnitudes, nor does it observe the *average*-price effect (the test name claims "uses_average_price" but no number that depends on the average is checked). Add `assert_borrow_near(ALICE, "ETH", 10.0, 0.01)` plus a `total_collateral` snapshot tied to the average price (~$101,500) versus aggregator-only (~$100,000).

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -650,9 +650,17 @@ fn test_second_tolerance_uses_average_price() {
     t.supply(ALICE, "USDC", 100_000.0);
     t.borrow(ALICE, "ETH", 10.0);
+    t.assert_supply_near(ALICE, "USDC", 100_000.0, 1.0);
+    t.assert_borrow_near(ALICE, "ETH", 10.0, 0.01);
 
     // The average price drives valuation.
     t.assert_healthy(ALICE);
+    // Average USDC price = ($1.00 + $1.03) / 2 = $1.015 -> ~$101,500 collateral.
+    let collat = t.total_collateral(ALICE);
+    assert!(
+        (101_000.0..=102_000.0).contains(&collat),
+        "averaged collateral should be ~$101,500, got {}",
+        collat
+    );
 }
```

### `oracle_tolerance_tests.rs::test_exchange_source_safe_only`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** Performs supply and borrow but asserts only `assert_healthy` (line 676). No position magnitudes, no wallet delta. A regression that records the operations partially but keeps HF healthy would still pass.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -672,8 +672,12 @@ fn test_exchange_source_safe_only() {
     // Operations succeed using the safe price alone.
     t.supply(ALICE, "USDC", 100_000.0);
     t.borrow(ALICE, "ETH", 10.0);
+    t.assert_supply_near(ALICE, "USDC", 100_000.0, 1.0);
+    t.assert_borrow_near(ALICE, "ETH", 10.0, 0.01);
+    let eth_wallet = t.token_balance(ALICE, "ETH");
+    assert!(eth_wallet > 9.99, "ETH wallet should be ~10, got {}", eth_wallet);
 
     t.assert_healthy(ALICE);
 }
```

### `oracle_tolerance_tests.rs::test_mixed_tolerance_states`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Generic `is_err()` (lines 698-702). Expected `UnsafePriceNotAllowed = 205`.

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -696,11 +696,11 @@ fn test_mixed_tolerance_states() {
     // Borrowing ETH must fail: ETH's price is beyond the second tolerance.
-    let result = t.try_borrow(ALICE, "ETH", 10.0);
-    assert!(
-        result.is_err(),
-        "borrow should fail when debt asset price is unsafe"
-    );
+    let err = t.try_borrow(ALICE, "ETH", 10.0)
+        .expect_err("borrow should fail when debt asset price is unsafe");
+    assert_eq!(err, soroban_sdk::Error::from_contract_error(
+        common::errors::OracleError::UnsafePriceNotAllowed as u32),
+        "expected UnsafePriceNotAllowed (205), got {:?}", err);
 }
```

### `oracle_tolerance_tests.rs::test_liquidation_dos_flash_crash`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Generic `is_err()` (lines 751-754). Expected `UnsafePriceNotAllowed = 205` — the test even narrates this in its preceding comment ("raising an OracleError").

**Patch (suggested):**
```diff
--- a/test-harness/tests/oracle_tolerance_tests.rs
+++ b/test-harness/tests/oracle_tolerance_tests.rs
@@ -747,11 +747,12 @@ fn test_liquidation_dos_flash_crash() {
     // The protocol panics and reverts: liquidation uses
     // allow_unsafe_price=false, and the 30% deviation between SPOT ($1400)
     // and TWAP ($1950) exceeds second_tolerance, raising an OracleError.
     // This perfectly DoSes liquidations precisely when they matter most.
-    assert!(
-        result.is_err(),
-        "Liquidation was perfectly DOSed by the oracle safety bands!"
-    );
+    let err = result.expect_err("Liquidation should be DOSed by oracle safety bands");
+    assert_eq!(err, soroban_sdk::Error::from_contract_error(
+        common::errors::OracleError::UnsafePriceNotAllowed as u32),
+        "expected UnsafePriceNotAllowed (205), got {:?}", err);
 }
```

### `oracle_tolerance_tests.rs::test_liquidation_collateral_extraction_via_averaging`

**Severity:** none

---

## Cross-cutting patterns

The dominant defect across this domain is **error-code laxity**: error-path tests pervasively use `assert!(result.is_err(), …)` instead of `assert_contract_error(result, errors::CODE)`. This is most acute in `oracle_tolerance_tests.rs` (every "blocks_*", "rejects_*" and "stale_*" test except `withdraw_blocked_under_oracle_deviation_when_debt_exists`) and propagates one regression in `borrow_tests.rs::test_borrow_position_limit_exceeded` whose comment misstates the wrapping behavior — adjacent test files (`strategy_edge_tests.rs`, `invariant_tests.rs`) already prove `POSITION_LIMIT_EXCEEDED = 109` surfaces directly. The secondary pattern is **success-path under-assertion**: many oracle-tolerance "allows" tests and several borrow bulk tests perform multi-step operations without `assert_borrow_near` / `assert_supply_near` / wallet-delta checks, so they verify only "no panic" rather than "the right amounts moved." Repay tests are stronger overall but uniformly miss wallet-delta verification, which is the most direct evidence that the auto-mint+transfer mechanics in `t.repay` actually settled the debt rather than no-opped. Five tests in `oracle_tolerance_tests.rs` need `common::errors::OracleError` codes that aren't yet exposed via `errors::*` (codes 208, 209, 216) — these patches reference them via `from_contract_error(N)` to avoid scope creep beyond the test files.
