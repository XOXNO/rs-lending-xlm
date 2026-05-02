# Domain 1 — Supply + EMode + Isolation

**Phase:** 1
**Files in scope:**
- `test-harness/tests/supply_tests.rs`
- `test-harness/tests/emode_tests.rs`
- `test-harness/tests/isolation_tests.rs`
- `test-harness/tests/account_tests.rs`
- `test-harness/tests/decimal_diversity_tests.rs`

**Totals:** broken=9 weak=19 nit=0

---

## supply_tests.rs

### `supply_tests.rs::test_supply_single_asset`

**Severity:** none

### `supply_tests.rs::test_supply_to_existing_account`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** The test supplies tokens twice (`5_000` then `3_000`) and asserts cumulative supply via `assert_supply_near` (post-state OK), but never verifies the caller's wallet balance dropped by the supplied amount. A regression that double-mints, fails the transfer, or skips the actual token-pull would not be caught. Lines 47-56.

**Patch (suggested):**
```diff
--- a/test-harness/tests/supply_tests.rs
+++ b/test-harness/tests/supply_tests.rs
@@ -47,11 +47,15 @@ fn test_supply_to_existing_account() {
     let mut t = LendingTest::new().with_market(usdc_preset()).build();
 
     t.supply(ALICE, "USDC", 5_000.0);
     t.assert_supply_near(ALICE, "USDC", 5_000.0, 1.0);
+    let wallet_after_first = t.token_balance(ALICE, "USDC");
+    assert!(wallet_after_first < 0.01, "wallet should be ~0 after first supply, got {}", wallet_after_first);
 
     // Supply more to the same account.
     t.supply(ALICE, "USDC", 3_000.0);
     t.assert_supply_near(ALICE, "USDC", 8_000.0, 1.0);
+    let wallet_after_second = t.token_balance(ALICE, "USDC");
+    assert!(wallet_after_second < 0.01, "wallet should be ~0 after second supply, got {}", wallet_after_second);
 }
```

### `supply_tests.rs::test_supply_multiple_assets_bulk`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** After `supply_bulk`, the test only checks that supply positions exist (lines 73-74) and never asserts the actual supply amounts (`assert_supply_near`) nor that wallet balances dropped to ~0 for both USDC and ETH. A bulk-supply that silently drops the second asset would still pass `assert_position_exists`-style checks if the same position were re-credited, and even a no-op wallet pull would not be caught. Lines 63-75.

**Patch (suggested):**
```diff
--- a/test-harness/tests/supply_tests.rs
+++ b/test-harness/tests/supply_tests.rs
@@ -69,8 +69,14 @@ fn test_supply_multiple_assets_bulk() {
     // Bulk supply via the harness method: a single controller call that
     // auto-mints.
     t.supply_bulk(ALICE, &[("USDC", 10_000.0), ("ETH", 1.0)]);
 
     t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
     t.assert_position_exists(ALICE, "ETH", PositionType::Supply);
+    t.assert_supply_near(ALICE, "USDC", 10_000.0, 1.0);
+    t.assert_supply_near(ALICE, "ETH", 1.0, 0.001);
+    let usdc_wallet = t.token_balance(ALICE, "USDC");
+    let eth_wallet = t.token_balance(ALICE, "ETH");
+    assert!(usdc_wallet < 0.01, "USDC wallet should be ~0, got {}", usdc_wallet);
+    assert!(eth_wallet < 0.0001, "ETH wallet should be ~0, got {}", eth_wallet);
 }
```

### `supply_tests.rs::test_supply_duplicate_asset_bulk_accumulates_single_position`

**Severity:** none

### `supply_tests.rs::test_supply_duplicate_isolated_asset_bulk_is_allowed`

**Severity:** none

### `supply_tests.rs::test_supply_creates_account_on_first_call`

**Severity:** none

### `supply_tests.rs::test_supply_with_emode_category`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Only `e_mode_category_id == 1` and `assert_position_exists` are asserted (lines 137-138). The actual supply amount is never verified via `assert_supply_near`, so a regression that creates a zero-amount position would still pass. Lines 124-139.

**Patch (suggested):**
```diff
--- a/test-harness/tests/supply_tests.rs
+++ b/test-harness/tests/supply_tests.rs
@@ -134,7 +134,9 @@ fn test_supply_with_emode_category() {
     t.supply(ALICE, "USDC", 10_000.0);
 
     let attrs = t.get_account_attributes(ALICE);
     assert_eq!(attrs.e_mode_category_id, 1);
     t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
+    t.assert_supply_near(ALICE, "USDC", 10_000.0, 1.0);
+    assert!(t.token_balance(ALICE, "USDC") < 0.01, "wallet should be ~0 after supply");
 }
```

### `supply_tests.rs::test_supply_rejects_zero_amount`

**Severity:** none

### `supply_tests.rs::test_supply_rejects_non_collateralizable`

**Severity:** none

### `supply_tests.rs::test_supply_rejects_during_flash_loan`

**Severity:** none

### `supply_tests.rs::test_supply_rejects_when_paused`

**Severity:** none

### `supply_tests.rs::test_supply_cap_enforcement`

**Severity:** none

### `supply_tests.rs::test_supply_position_limit_exceeded`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Lines 239-242 use `result.is_err()` only — the comment claims "the Soroban host wraps the error as InvalidAction on the cross-contract path", but `controller/src/validation.rs:160` directly emits `CollateralError::PositionLimitExceeded` (109) and `invariant_tests.rs::test_position_limits_enforced` (line 217) asserts that exact code via `assert_contract_error`. A bare `is_err()` accepts any failure (pause, oracle stale, internal error) and would miss a regression that swapped the code.

**Patch (suggested):**
```diff
--- a/test-harness/tests/supply_tests.rs
+++ b/test-harness/tests/supply_tests.rs
@@ -233,15 +233,11 @@ fn test_supply_position_limit_exceeded() {
     t.supply(ALICE, "USDC", 1_000.0);
     t.supply(ALICE, "ETH", 1.0);
 
-    // The third supply must exceed the limit of 2. Note: the Soroban host
-    // wraps the error as InvalidAction on the cross-contract path.
+    // The third supply must reject with the specific PositionLimitExceeded
+    // error.
     let result = t.try_supply(ALICE, "WBTC", 0.01);
-    assert!(
-        result.is_err(),
-        "supply exceeding position limit should fail"
-    );
+    assert_contract_error(result, errors::POSITION_LIMIT_EXCEEDED);
 }
```

### `supply_tests.rs::test_supply_isolated_account_single_asset`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Post-state is asserted via `assert_position_exists` and `assert_supply_near` (lines 263-264), but the wallet delta is not verified. A regression where the harness mints but the contract fails to pull the funds would still pass — the supply position would be credited but Alice would also still hold the minted USDC. Lines 250-265.

**Patch (suggested):**
```diff
--- a/test-harness/tests/supply_tests.rs
+++ b/test-harness/tests/supply_tests.rs
@@ -260,7 +260,8 @@ fn test_supply_isolated_account_single_asset() {
     t.create_isolated_account(ALICE, "USDC");
     t.supply(ALICE, "USDC", 5_000.0);
 
     t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
     t.assert_supply_near(ALICE, "USDC", 5_000.0, 1.0);
+    assert!(t.token_balance(ALICE, "USDC") < 0.01, "wallet should be ~0 after supply");
 }
```

### `supply_tests.rs::test_supply_isolated_rejects_second_asset`

**Severity:** none

### `supply_tests.rs::test_supply_emode_rejects_non_category_asset`

**Severity:** none

### `supply_tests.rs::test_supply_raw_precision`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Asserts post-state via `supply_balance_raw >= 1` (lines 325-329) but never checks the wallet delta — a regression that mints but never transfers would still pass. The sentinel value (1 raw unit) makes a wallet check meaningful: pre-balance should be 0 and post-balance should be 0 (the 1 raw unit was pulled into the pool). Lines 316-330.

**Patch (suggested):**
```diff
--- a/test-harness/tests/supply_tests.rs
+++ b/test-harness/tests/supply_tests.rs
@@ -316,18 +316,22 @@ fn test_supply_raw_precision() {
 fn test_supply_raw_precision() {
     let mut t = LendingTest::new().with_market(usdc_preset()).build();
 
     // Supply exactly 1 unit (smallest: 1 with 7 decimals = 0.0000001 USDC).
     let raw_amount = 1i128;
     t.supply_raw(ALICE, "USDC", raw_amount);
 
     let balance = t.supply_balance_raw(ALICE, "USDC");
     // Must be at least 1 (could be exactly 1 or close due to the index).
     assert!(
         balance >= 1,
         "raw supply should preserve precision, got {}",
         balance
     );
+    // The 1 raw unit must have left Alice's wallet.
+    let wallet_raw = t.token_balance_raw(ALICE, "USDC");
+    assert_eq!(wallet_raw, 0, "raw wallet should be 0 after pulling 1 unit, got {}", wallet_raw);
 }
```

---

## emode_tests.rs

### `emode_tests.rs::test_emode_category_creation`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Only asserts `account_id > 0` (line 26) — does not verify the e-mode category id stuck on the created account, nor that the category exists in storage with the expected ltv/threshold/bonus. A regression that created the account at a different category id (or category 0) would still pass. Lines 14-27.

**Patch (suggested):**
```diff
--- a/test-harness/tests/emode_tests.rs
+++ b/test-harness/tests/emode_tests.rs
@@ -22,7 +22,9 @@ fn test_emode_category_creation() {
     // The build created the category. Verify by creating an e-mode account;
     // a missing category would fail.
     let mut t = t;
     let account_id = t.create_emode_account(ALICE, 1);
     assert!(account_id > 0, "should create e-mode account");
+    let attrs = t.get_account_attributes(ALICE);
+    assert_eq!(attrs.e_mode_category_id, 1, "account should be in e-mode category 1");
 }
```

### `emode_tests.rs::test_emode_enhanced_ltv_and_threshold`

**Severity:** none

### `emode_tests.rs::test_emode_supply_with_category_asset`

**Severity:** none

### `emode_tests.rs::test_emode_borrow_with_category_asset`

**Severity:** none

### `emode_tests.rs::test_emode_rejects_non_category_supply`

**Severity:** none

### `emode_tests.rs::test_emode_rejects_non_category_borrow`

**Severity:** none

### `emode_tests.rs::test_emode_rejects_with_isolation`

**Severity:** broken
**Rubric items failed:** [1, 2]
**Why:** Lines 159-167 use `std::panic::catch_unwind` over `t2.create_account_full(ALICE, 1, ..., true)`. `create_account_full` calls into the harness's own `create_account_direct` (test-harness/src/user.rs:53-107) which panics with a native Rust `assert!` (line 68-71): `assert!(!(e_mode_category > 0 && is_isolated), "e-mode and isolation are mutually exclusive")` — NOT through the contract. The actual `EModeError::EModeWithIsolated` (302) panic in `controller/src/positions/emode.rs:160` is never executed. The test passes vacuously regardless of contract behavior. Furthermore, bare `result.is_err()` (line 164) doesn't pin the error code.

**Patch (suggested):**
```diff
--- a/test-harness/tests/emode_tests.rs
+++ b/test-harness/tests/emode_tests.rs
@@ -143,27 +143,25 @@
 // ---------------------------------------------------------------------------
 // 7. test_emode_rejects_with_isolation
 // ---------------------------------------------------------------------------
 
 #[test]
 fn test_emode_rejects_with_isolation() {
-    let t = LendingTest::new()
+    let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
         .with_market_config("ETH", |cfg| {
             cfg.is_isolated_asset = true;
             cfg.isolation_debt_ceiling_usd_wad = 1_000_000 * WAD;
         })
         .with_emode(1, STABLECOIN_EMODE)
         .with_emode_asset(1, "USDC", true, true)
+        .with_emode_asset(1, "ETH", true, true)
         .build();
 
-    // Creating an account with both e-mode and isolation must panic.
-    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
-        let mut t2 = t;
-        t2.create_account_full(ALICE, 1, common::types::PositionMode::Normal, true);
-    }));
-    assert!(
-        result.is_err(),
-        "should reject creating account with both e-mode and isolation"
-    );
+    // Drive the contract path: an e-mode account that supplies an isolated
+    // asset must be rejected by `ensure_e_mode_compatible_with_asset` with
+    // `EModeWithIsolated` (302). This exercises the controller, not the
+    // harness's local assert in `create_account_direct`.
+    t.create_emode_account(ALICE, 1);
+    let result = t.try_supply(ALICE, "ETH", 1.0);
+    assert_contract_error(result, errors::EMODE_WITH_ISOLATED);
 }
```

### `emode_tests.rs::test_emode_deprecated_blocks_new_accounts`

**Severity:** broken
**Rubric items failed:** [1, 2]
**Why:** Lines 185-192 use `catch_unwind` over `t2.create_emode_account(ALICE, 1)`, which calls the harness's `create_account_direct` (test-harness/src/user.rs:73-81) — that path panics with a native Rust `assert!(!category.is_deprecated, "e-mode category is deprecated")`, NOT the contract's `EModeError::EModeCategoryDeprecated` (301). The `result.is_err()` check is a tautology over the harness assert; the actual contract `panic_with_error!` at `controller/src/positions/emode.rs:95` is never run. To exercise the contract, drive an e-mode supply with a deprecated category.

**Patch (suggested):**
```diff
--- a/test-harness/tests/emode_tests.rs
+++ b/test-harness/tests/emode_tests.rs
@@ -171,22 +171,29 @@
 // ---------------------------------------------------------------------------
 // 8. test_emode_deprecated_blocks_new_accounts
 // ---------------------------------------------------------------------------
 
 #[test]
 fn test_emode_deprecated_blocks_new_accounts() {
-    let t = LendingTest::new()
+    let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_emode(1, STABLECOIN_EMODE)
         .with_emode_asset(1, "USDC", true, true)
         .build();
 
+    // Create the e-mode account BEFORE deprecation so the harness's local
+    // deprecation assert does not short-circuit. The contract path under
+    // test is the one that supplies under a deprecated category, which
+    // routes through `active_e_mode_category` -> `ensure_e_mode_not_deprecated`.
+    t.create_emode_account(ALICE, 1);
+
     // Deprecate the e-mode category.
     t.remove_e_mode_category(1);
 
-    // Creating an account with this category must now fail.
-    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
-        let mut t2 = t;
-        t2.create_emode_account(ALICE, 1);
-    }));
-    assert!(
-        result.is_err(),
-        "should reject new accounts for deprecated e-mode category"
-    );
+    // Supplying under the now-deprecated category must reject with the
+    // contract error EModeCategoryDeprecated (301).
+    let result = t.try_supply(ALICE, "USDC", 1_000.0);
+    assert_contract_error(result, errors::EMODE_CATEGORY_DEPRECATED);
 }
```

### `emode_tests.rs::test_emode_edit_category_params`

**Severity:** none

### `emode_tests.rs::test_emode_remove_category_deprecates`

**Severity:** broken
**Rubric items failed:** [1, 2]
**Why:** Same root cause as `test_emode_deprecated_blocks_new_accounts`. Lines 235-239 catch a harness `assert!` panic from `create_account_direct` rather than the contract's `EModeCategoryDeprecated` (301). Drive the contract path via supply after deprecation.

**Patch (suggested):**
```diff
--- a/test-harness/tests/emode_tests.rs
+++ b/test-harness/tests/emode_tests.rs
@@ -222,16 +222,16 @@
 // ---------------------------------------------------------------------------
 // 10. test_emode_remove_category_deprecates
 // ---------------------------------------------------------------------------
 
 #[test]
 fn test_emode_remove_category_deprecates() {
-    let t = LendingTest::new()
+    let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_emode(1, STABLECOIN_EMODE)
         .with_emode_asset(1, "USDC", true, true)
         .build();
 
+    // Create the e-mode account before deprecation; the harness's local
+    // deprecation assert blocks creation under a deprecated category.
+    t.create_emode_account(ALICE, 1);
+
     t.remove_e_mode_category(1);
 
-    // Confirm deprecation: creating a new account must panic.
-    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
-        let mut t2 = t;
-        t2.create_emode_account(ALICE, 1);
-    }));
-    assert!(result.is_err(), "removed category should be deprecated");
+    // Confirm deprecation via the contract path: supply must reject with
+    // EModeCategoryDeprecated (301).
+    let result = t.try_supply(ALICE, "USDC", 1_000.0);
+    assert_contract_error(result, errors::EMODE_CATEGORY_DEPRECATED);
 }
```

### `emode_tests.rs::test_emode_add_asset_to_category`

**Severity:** none

### `emode_tests.rs::test_emode_remove_asset_from_category`

**Severity:** none

### `emode_tests.rs::test_remove_asset_e_mode_category_alias`

**Severity:** none

### `emode_tests.rs::test_emode_liquidation_uses_emode_bonus`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Tests the e-mode liquidation bonus, but the bonus assertion (lines 348-352) is one-sided: only `ratio < 1.06` is checked. The standard bonus is 5% so `ratio < 1.06` would also pass with the standard 5% bonus when the ratio is, e.g., 1.05. The test should also bound the ratio from below (`ratio > 1.015`) to confirm the e-mode bonus actually applied (close to 1.02), not zero-bonus and not standard 5%. Also no debt-position post-state assertion (e.g., that Alice's USDT debt actually decreased). Lines 316-354.

**Patch (suggested):**
```diff
--- a/test-harness/tests/emode_tests.rs
+++ b/test-harness/tests/emode_tests.rs
@@ -325,15 +325,18 @@ fn test_emode_liquidation_uses_emode_bonus() {
     t.create_emode_account(ALICE, 1);
     t.supply(ALICE, "USDC", 10_000.0);
     t.borrow(ALICE, "USDT", 9_500.0);
 
     // Drop USDC price to force clear liquidation.
     t.set_price("USDC", usd_cents(90));
     t.assert_liquidatable(ALICE);
 
+    let debt_before = t.borrow_balance(ALICE, "USDT");
     t.liquidate(LIQUIDATOR, ALICE, "USDT", 2_000.0);
+    let debt_after = t.borrow_balance(ALICE, "USDT");
+    assert!(debt_after < debt_before, "USDT debt should decrease after liquidation: before={}, after={}", debt_before, debt_after);
 
     // The liquidator must receive collateral with the 2% e-mode bonus.
     let usdc_received = t.token_balance(LIQUIDATOR, "USDC");
     assert!(usdc_received > 0.0, "liquidator should receive collateral");
 
     // The value ratio must hover near 1.02 (2% e-mode bonus), not 1.05
@@ -344,7 +347,11 @@ fn test_emode_liquidation_uses_emode_bonus() {
     if usdc_value > 0.0 {
         let ratio = usdc_value / debt_value;
-        // E-mode bonus is 2%, so the ratio must sit near 1.02, not 1.05.
+        // E-mode bonus is 2%, so the ratio must sit near 1.02 (between 1.015
+        // and 1.04). A one-sided `< 1.06` check would also pass under the
+        // standard 5% bonus.
         assert!(
-            ratio < 1.06,
-            "e-mode bonus should be lower than standard: ratio={}",
+            ratio > 1.015 && ratio < 1.04,
+            "e-mode bonus should be ~1.02 (not zero, not 5%): ratio={}",
             ratio
         );
     }
 }
```

### `emode_tests.rs::test_emode_two_assets_same_category`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** After `borrow(ALICE, "USDC", 2_000.0)` the test asserts only `assert_healthy` (line 381). It does not assert the borrow position exists, the borrow amount is correct, or the supply positions still hold the expected balances. Lines 361-382.

**Patch (suggested):**
```diff
--- a/test-harness/tests/emode_tests.rs
+++ b/test-harness/tests/emode_tests.rs
@@ -373,12 +373,14 @@ fn test_emode_two_assets_same_category() {
     // Supply both stablecoins.
     t.supply(ALICE, "USDC", 5_000.0);
     t.supply(ALICE, "USDT", 5_000.0);
 
     t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
     t.assert_position_exists(ALICE, "USDT", PositionType::Supply);
+    t.assert_supply_near(ALICE, "USDC", 5_000.0, 1.0);
+    t.assert_supply_near(ALICE, "USDT", 5_000.0, 1.0);
 
     // Borrow USDC against USDT collateral and vice versa.
     t.borrow(ALICE, "USDC", 2_000.0);
+    t.assert_position_exists(ALICE, "USDC", PositionType::Borrow);
+    t.assert_borrow_near(ALICE, "USDC", 2_000.0, 1.0);
     t.assert_healthy(ALICE);
 }
```

### `emode_tests.rs::test_emode_rejects_threshold_lte_ltv`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Lines 393-409 use `std::panic::catch_unwind` and bare `result.is_err()`. The contract path (`controller/src/config.rs:233`) panics with `CollateralError::InvalidLiqThreshold` (113) — `try_add_e_mode_category` would surface this exact code. Bare `is_err()` accepts any panic (e.g., a future fixture-build panic). The first `_t = ...build()` (line 390) is also dead setup. The test should call `try_*` on the controller and assert the specific error code.

**Patch (suggested):**
```diff
--- a/test-harness/tests/emode_tests.rs
+++ b/test-harness/tests/emode_tests.rs
@@ -385,28 +385,21 @@
 // ---------------------------------------------------------------------------
 // 15. test_emode_rejects_threshold_lte_ltv
 // ---------------------------------------------------------------------------
 
 #[test]
 fn test_emode_rejects_threshold_lte_ltv() {
-    let _t = LendingTest::new().with_market(usdc_preset()).build();
-
-    // Adding an e-mode category where threshold <= ltv must panic.
-    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
-        let _t2 = LendingTest::new()
-            .with_market(usdc_preset())
-            .with_emode(
-                1,
-                EModeCategoryPreset {
-                    ltv: 9000,
-                    threshold: 8000, // threshold < ltv: invalid.
-                    bonus: 200,
-                },
-            )
-            .build();
-    }));
-    assert!(
-        result.is_err(),
-        "should reject e-mode category where threshold <= ltv"
-    );
+    let t = LendingTest::new().with_market(usdc_preset()).build();
+
+    // Call the controller directly and assert the specific error code.
+    // threshold (8000) <= ltv (9000) must reject with InvalidLiqThreshold (113).
+    let result = t.ctrl_client().try_add_e_mode_category(&9000i128, &8000i128, &200i128);
+    let flat = match result {
+        Ok(Ok(_)) => panic!("expected contract error, got Ok"),
+        Ok(Err(err)) => Err(err.into()),
+        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
+    };
+    assert_contract_error(flat, errors::INVALID_LIQ_THRESHOLD);
 }
```

> Note: confirm the `try_add_e_mode_category` client signature in `controller/src/config.rs` and adapt the call shape if the SDK requires `i128` references or different argument names. The `EModeCategoryPreset` import becomes unused after this change.

---

## isolation_tests.rs

### `isolation_tests.rs::test_isolated_account_creation`

**Severity:** none

### `isolation_tests.rs::test_isolated_supply_single_asset`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Asserts position exists and supply ~5.0 ETH (lines 58-59) but does not check the wallet delta. The `setup_isolated()` fixture uses `eth_preset()` so Alice should be minted 5 ETH; supply pulls them; wallet should be ~0. Lines 53-60.

**Patch (suggested):**
```diff
--- a/test-harness/tests/isolation_tests.rs
+++ b/test-harness/tests/isolation_tests.rs
@@ -53,9 +53,10 @@ fn test_isolated_supply_single_asset() {
     let mut t = setup_isolated();
     t.create_isolated_account(ALICE, "ETH");
     t.supply(ALICE, "ETH", 5.0);
 
     t.assert_position_exists(ALICE, "ETH", PositionType::Supply);
     t.assert_supply_near(ALICE, "ETH", 5.0, 0.01);
+    assert!(t.token_balance(ALICE, "ETH") < 0.0001, "ETH wallet should be ~0 after supply");
 }
```

### `isolation_tests.rs::test_isolated_rejects_second_collateral`

**Severity:** none

### `isolation_tests.rs::test_isolated_borrow_enabled_asset`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** After `borrow(ALICE, "USDC", 5_000.0)` only the position-exists and HF checks run (lines 89-90). The borrow path transfers borrowed USDC to Alice's wallet; the test should confirm Alice received ~5,000 USDC. A regression where the borrow accounting succeeds but the transfer to caller fails would still pass. Lines 82-91.

**Patch (suggested):**
```diff
--- a/test-harness/tests/isolation_tests.rs
+++ b/test-harness/tests/isolation_tests.rs
@@ -82,11 +82,13 @@ fn test_isolated_borrow_enabled_asset() {
     let mut t = setup_isolated();
     t.create_isolated_account(ALICE, "ETH");
     t.supply(ALICE, "ETH", 5.0); // ~$10,000
 
     // USDC has isolation_borrow_enabled = true.
     t.borrow(ALICE, "USDC", 5_000.0);
     t.assert_position_exists(ALICE, "USDC", PositionType::Borrow);
+    t.assert_borrow_near(ALICE, "USDC", 5_000.0, 1.0);
+    let usdc_wallet = t.token_balance(ALICE, "USDC");
+    assert!((usdc_wallet - 5_000.0).abs() < 1.0, "Alice should receive ~5000 USDC, got {}", usdc_wallet);
     t.assert_healthy(ALICE);
 }
```

### `isolation_tests.rs::test_isolated_borrow_disabled_asset`

**Severity:** none

### `isolation_tests.rs::test_isolated_debt_ceiling_enforced`

**Severity:** none

### `isolation_tests.rs::test_isolated_debt_decremented_on_repay`

**Severity:** none

### `isolation_tests.rs::test_isolated_debt_decremented_on_liquidation`

**Severity:** none

### `isolation_tests.rs::test_isolated_rejects_emode`

**Severity:** broken
**Rubric items failed:** [1, 2]
**Why:** Lines 207-214 use `catch_unwind` over `create_account_full(ALICE, 1, ..., true)` — that hits the harness's native `assert!(!(e_mode_category > 0 && is_isolated), ...)` (test-harness/src/user.rs:68-71), NOT the contract's `EModeError::EModeWithIsolated` (302) panic in `controller/src/positions/emode.rs:160`. The test passes regardless of whether the contract enforces the rule. Drive the contract path: create an e-mode account, then try to supply an isolated asset — `ensure_e_mode_compatible_with_asset` (controller/src/positions/emode.rs:48) emits the proper code.

**Patch (suggested):**
```diff
--- a/test-harness/tests/isolation_tests.rs
+++ b/test-harness/tests/isolation_tests.rs
@@ -191,28 +191,25 @@
 // ---------------------------------------------------------------------------
 // 9. test_isolated_rejects_emode
 // ---------------------------------------------------------------------------
 
 #[test]
 fn test_isolated_rejects_emode() {
-    let t = LendingTest::new()
+    let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
         .with_market_config("ETH", |cfg| {
             cfg.is_isolated_asset = true;
             cfg.isolation_debt_ceiling_usd_wad = ISOLATION_CEILING_WAD;
         })
         .with_emode(1, STABLECOIN_EMODE)
         .with_emode_asset(1, "USDC", true, true)
+        .with_emode_asset(1, "ETH", true, true)
         .build();
 
-    // Creating an account with both e-mode and isolation must panic.
-    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
-        let mut t2 = t;
-        t2.create_account_full(ALICE, 1, common::types::PositionMode::Normal, true);
-    }));
-    assert!(
-        result.is_err(),
-        "should reject account with both e-mode and isolation"
-    );
+    // Drive the contract path: an e-mode account that supplies an isolated
+    // asset must be rejected with EModeWithIsolated (302). The harness's
+    // `create_account_*` helpers bypass the contract validator.
+    t.create_emode_account(ALICE, 1);
+    let result = t.try_supply(ALICE, "ETH", 1.0);
+    assert_contract_error(result, errors::EMODE_WITH_ISOLATED);
 }
```

### `isolation_tests.rs::test_isolated_rejects_swap_collateral`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Lines 230-234 use bare `result.is_err()`. The contract path (`controller/src/strategy.rs:350`) emits `FlashLoanError::SwapCollateralNoIso` (404), which `try_swap_collateral` surfaces as a contract error. `is_err()` would also accept oracle/tolerance/host failures and would not catch a regression that swapped the code or removed the isolation check entirely.

**Patch (suggested):**
```diff
--- a/test-harness/tests/isolation_tests.rs
+++ b/test-harness/tests/isolation_tests.rs
@@ -225,12 +225,9 @@ fn test_isolated_rejects_swap_collateral() {
     t.create_isolated_account(ALICE, "ETH");
     t.supply(ALICE, "ETH", 5.0);
 
     let steps = t.mock_swap_steps("ETH", "USDC", usd(2000));
     let result = t.try_swap_collateral(ALICE, "ETH", 1.0, "USDC", &steps);
-    // Strategy cross-contract calls may surface as host errors.
-    assert!(
-        result.is_err(),
-        "should reject swap_collateral on isolated account"
-    );
+    // The contract panics with SwapCollateralNoIso (404) for isolated accounts.
+    assert_contract_error(result, errors::SWAP_COLLATERAL_NO_ISO);
 }
```

### `isolation_tests.rs::test_isolated_liquidation_works`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Asserts debt decreased and liquidator received ETH (lines 257-266) but does not check the isolated-debt tracker decreased — the central isolation invariant ("isolated debt counter is decremented on liquidation"). `test_isolated_debt_decremented_on_liquidation` covers a similar scenario but here we should at minimum assert `get_isolated_debt("ETH")` decreased. Lines 240-267.

**Patch (suggested):**
```diff
--- a/test-harness/tests/isolation_tests.rs
+++ b/test-harness/tests/isolation_tests.rs
@@ -242,18 +242,21 @@ fn test_isolated_liquidation_works() {
     let mut t = setup_isolated();
     t.create_isolated_account(ALICE, "ETH");
     t.supply(ALICE, "ETH", 5.0);
     t.borrow(ALICE, "USDC", 5_000.0);
 
     // Drop ETH price moderately to make Alice mildly liquidatable.
     // At $1000: collateral = $5000, threshold 80% => weighted = $4000,
     // debt = $5000 => HF = 0.8.
     t.set_price("ETH", usd(1000));
     t.assert_liquidatable(ALICE);
 
     let debt_before = t.borrow_balance(ALICE, "USDC");
+    let iso_debt_before = t.get_isolated_debt("ETH");
     t.liquidate(LIQUIDATOR, ALICE, "USDC", 1_000.0);
     let debt_after = t.borrow_balance(ALICE, "USDC");
+    let iso_debt_after = t.get_isolated_debt("ETH");
 
     assert!(
         debt_after < debt_before,
         "debt should decrease after liquidation: before={}, after={}",
         debt_before,
         debt_after
     );
 
+    assert!(iso_debt_after < iso_debt_before, "isolated debt tracker should decrement: before={}, after={}", iso_debt_before, iso_debt_after);
+
     // The liquidator should have received ETH collateral.
     let liq_eth = t.token_balance(LIQUIDATOR, "ETH");
     assert!(liq_eth > 0.0, "liquidator should receive ETH collateral");
 }
```

### `isolation_tests.rs::test_isolated_bad_debt_clears_isolated_tracker`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** The test name promises "clears" the isolated tracker (i.e., back to zero), but only asserts `iso_debt_after < iso_debt_before` (line 305-310). The header comment (lines 301-303) explicitly says the bad-debt cleanup socializes the loss and the tracker must be cleared to zero. A regression that decrements by a tiny amount but never clears would still pass. Lines 274-311.

**Patch (suggested):**
```diff
--- a/test-harness/tests/isolation_tests.rs
+++ b/test-harness/tests/isolation_tests.rs
@@ -298,16 +298,15 @@ fn test_isolated_bad_debt_clears_isolated_tracker() {
     // Liquidate -- bad-debt handling must engage for tiny underwater positions.
     t.liquidate(LIQUIDATOR, ALICE, "USDC", 100.0);
 
     // After liquidation + bad-debt cleanup, the account is removed
     // (collateral was tiny, so bad-debt cleanup socializes the loss).
     // The isolated-debt tracker must be cleared to zero.
     let iso_debt_after = t.get_isolated_debt("ETH");
-    assert!(
-        iso_debt_after < iso_debt_before,
-        "isolated debt should decrease: before={}, after={}",
+    assert_eq!(
+        iso_debt_after, 0,
+        "isolated debt tracker must be cleared to zero after bad-debt cleanup, got {}",
         iso_debt_before,
-        iso_debt_after
     );
 }
```

> Note: if the harness preserves a non-zero rounding residue, relax to `assert!(iso_debt_after < iso_debt_before / 100, ...)` — but `< before` alone is too loose for the invariant the test name describes.

---

## account_tests.rs

### `account_tests.rs::test_create_normal_account`

**Severity:** none

### `account_tests.rs::test_create_emode_account`

**Severity:** none

### `account_tests.rs::test_create_isolated_account`

**Severity:** none

### `account_tests.rs::test_create_account_full_custom`

**Severity:** none

### `account_tests.rs::test_remove_empty_account`

**Severity:** none

### `account_tests.rs::test_remove_rejects_with_positions`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Line 120-124 uses bare `result.is_err()`. The harness's `try_remove_account` -> `remove_account_direct` (test-harness/src/user.rs:524-528) returns `CollateralError::PositionNotFound` (110) when supply or borrow positions exist. A specific contract-error assertion would catch a regression that returned the wrong code or no error.

**Patch (suggested):**
```diff
--- a/test-harness/tests/account_tests.rs
+++ b/test-harness/tests/account_tests.rs
@@ -1,9 +1,9 @@ extern crate std;
 
 use common::constants::WAD;
 
 use test_harness::{
-    eth_preset, usdc_preset, usdt_stable_preset, LendingTest, ALICE, BOB, STABLECOIN_EMODE,
+    assert_contract_error, errors, eth_preset, usdc_preset, usdt_stable_preset, LendingTest, ALICE, BOB, STABLECOIN_EMODE,
 };
@@ -116,12 +116,9 @@ fn test_remove_rejects_with_positions() {
     let mut t = LendingTest::new().with_market(usdc_preset()).build();
 
     t.supply(ALICE, "USDC", 1_000.0);
 
     let result = t.try_remove_account(ALICE);
-    assert!(
-        result.is_err(),
-        "remove should fail when account has positions"
-    );
+    assert_contract_error(result, errors::POSITION_NOT_FOUND);
 }
```

### `account_tests.rs::test_multiple_accounts_per_user`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Asserts post-state via `supply_balance_for` (lines 146-149) but does not assert wallet deltas — Alice should have ~0 of USDC and ETH after the per-account supplies. A regression where `supply_to` credits the position but never pulls tokens would still pass. Lines 132-153.

**Patch (suggested):**
```diff
--- a/test-harness/tests/account_tests.rs
+++ b/test-harness/tests/account_tests.rs
@@ -141,15 +141,17 @@ fn test_multiple_accounts_per_user() {
     // Supply to each account.
     t.supply_to(ALICE, id1, "USDC", 1_000.0);
     t.supply_to(ALICE, id2, "ETH", 0.5);
 
     let bal1 = t.supply_balance_for(ALICE, id1, "USDC");
     let bal2 = t.supply_balance_for(ALICE, id2, "ETH");
     assert!(bal1 > 999.0, "account 1 should have ~1000 USDC supply");
     assert!(bal2 > 0.49, "account 2 should have ~0.5 ETH supply");
+    assert!(t.token_balance(ALICE, "USDC") < 0.01, "USDC wallet should be ~0 after supply_to");
+    assert!(t.token_balance(ALICE, "ETH") < 0.0001, "ETH wallet should be ~0 after supply_to");
 
     let accounts = t.get_active_accounts(ALICE);
     assert!(accounts.len() >= 2, "should have at least 2 accounts");
 }
```

### `account_tests.rs::test_account_auto_removed_after_full_repay_withdraw`

**Severity:** none

### `account_tests.rs::test_get_active_accounts`

**Severity:** none

### `account_tests.rs::test_account_owner_verified`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Lines 222-225 use `result.is_err() || result.unwrap().is_err()` — accepts ANY failure, including a malformed-input or panic-during-setup outcome. The contract's `require_account_owner_match` (controller/src/validation.rs:31-35) panics with `GenericError::AccountNotInMarket` (13). The check should pin that specific code via `assert_contract_error` (after flattening via the standard `Ok(Ok)` / `Ok(Err)` / `Err` shape used elsewhere in the harness).

**Patch (suggested):**
```diff
--- a/test-harness/tests/account_tests.rs
+++ b/test-harness/tests/account_tests.rs
@@ -1,9 +1,9 @@ extern crate std;
 
 use common::constants::WAD;
 
 use test_harness::{
-    eth_preset, usdc_preset, usdt_stable_preset, LendingTest, ALICE, BOB, STABLECOIN_EMODE,
+    assert_contract_error, errors, eth_preset, usdc_preset, usdt_stable_preset, LendingTest, ALICE, BOB, STABLECOIN_EMODE,
 };
@@ -213,15 +213,17 @@ fn test_account_owner_verified() {
     let alice_account_id = t.resolve_account_id(ALICE);
     let bob_addr = t.get_or_create_user(BOB);
     let usdc_addr = t.resolve_asset("USDC");
 
     let ctrl = t.ctrl_client();
     let withdrawals = soroban_sdk::vec![&t.env, (usdc_addr, 10_000_000_000i128)];
     let result = ctrl.try_withdraw(&bob_addr, &alice_account_id, &withdrawals);
-    assert!(
-        result.is_err() || result.unwrap().is_err(),
-        "BOB should not be able to withdraw from ALICE's account"
-    );
+    let flat = match result {
+        Ok(Ok(())) => panic!("expected contract error, got Ok"),
+        Ok(Err(err)) => Err(err.into()),
+        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
+    };
+    assert_contract_error(flat, errors::ACCOUNT_NOT_IN_MARKET);
 }
```

> Note: if a sole `import` adjustment for `assert_contract_error, errors` is already covered by the patch in `test_remove_rejects_with_positions`, deduplicate when applying.

---

## decimal_diversity_tests.rs

### `decimal_diversity_tests.rs::test_supply_6dec_borrow_18dec`

**Severity:** none

### `decimal_diversity_tests.rs::test_supply_18dec_borrow_6dec`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Asserts borrow amount and HF (lines 117-119) but never asserts the supply post-state (`assert_supply_near(ALICE, "DAI18", 10_000.0, ...)`) for the 18-decimal token. The whole point of the cross-decimal test is precision preservation — without the supply assertion, a regression that under-credits 18-dec supply by orders of magnitude could still pass since the 5,000 USDC borrow is well within any conceivable LTV. Lines 110-120.

**Patch (suggested):**
```diff
--- a/test-harness/tests/decimal_diversity_tests.rs
+++ b/test-harness/tests/decimal_diversity_tests.rs
@@ -111,9 +111,10 @@ fn test_supply_18dec_borrow_6dec() {
     let mut t = LendingTest::new()
         .with_market(usdc_6dec())
         .with_market(dai_18dec())
         .build();
 
     t.supply(ALICE, "DAI18", 10_000.0);
+    t.assert_supply_near(ALICE, "DAI18", 10_000.0, 0.01);
     t.borrow(ALICE, "USDC6", 5_000.0);
     t.assert_borrow_near(ALICE, "USDC6", 5_000.0, 0.01);
     t.assert_healthy(ALICE);
 }
```

### `decimal_diversity_tests.rs::test_supply_9dec_borrow_8dec`

**Severity:** none

### `decimal_diversity_tests.rs::test_mixed_decimal_types_single_account`

**Severity:** none

### `decimal_diversity_tests.rs::test_tiny_amounts_18dec`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Only asserts `supply > 0.0` (lines 200-206). For an 18-decimal token with input 0.000001, the supply balance should round to roughly that input value, not "any positive number". A regression that credits 1 raw unit (10^-18 DAI ≈ 1e-18) instead of 10^12 raw units would still pass. Tighten the bound. Lines 192-207.

**Patch (suggested):**
```diff
--- a/test-harness/tests/decimal_diversity_tests.rs
+++ b/test-harness/tests/decimal_diversity_tests.rs
@@ -192,16 +192,16 @@ fn test_tiny_amounts_18dec() {
 fn test_tiny_amounts_18dec() {
     let mut t = LendingTest::new()
         .with_market(dai_18dec())
         .with_market(usdc_6dec())
         .build();
 
     // Supply 0.000001 DAI (1 microDAI = 10^12 raw units at 18 decimals).
     t.supply(ALICE, "DAI18", 0.000001);
 
-    let supply = t.supply_balance(ALICE, "DAI18");
-    assert!(
-        supply > 0.0,
-        "Supply balance should be positive even for tiny 18-dec amount, got {}",
-        supply
-    );
+    // The 1 microDAI must be credited within rounding, not as an arbitrary
+    // positive value (e.g., 1 raw unit ~ 1e-18). Confirm raw and float views.
+    let raw = t.supply_balance_raw(ALICE, "DAI18");
+    assert!(raw >= 999_999_999_000i128, "raw 18-dec supply should be ~10^12, got {}", raw);
+    let supply = t.supply_balance(ALICE, "DAI18");
+    assert!((supply - 0.000_001).abs() < 1e-9, "supply should be ~1 microDAI, got {}", supply);
 }
```

### `decimal_diversity_tests.rs::test_large_amounts_6dec`

**Severity:** none

### `decimal_diversity_tests.rs::test_interest_accrual_mixed_decimals`

**Severity:** none

### `decimal_diversity_tests.rs::test_repay_cross_decimal`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Tests partial then full repay across decimals but does not check the wallet delta on either repay leg. The repay path should pull DAI from Alice's wallet for the partial repay and refund any overpay surplus on the full repay; neither is verified. Lines 274-297.

**Patch (suggested):**
```diff
--- a/test-harness/tests/decimal_diversity_tests.rs
+++ b/test-harness/tests/decimal_diversity_tests.rs
@@ -278,16 +278,21 @@ fn test_repay_cross_decimal() {
         .with_market(dai_18dec())
         .build();
 
     t.supply(ALICE, "USDC6", 10_000.0);
     t.borrow(ALICE, "DAI18", 5_000.0);
+    let dai_after_borrow = t.token_balance(ALICE, "DAI18");
+    assert!((dai_after_borrow - 5_000.0).abs() < 1.0, "Alice should have ~5000 DAI from borrow, got {}", dai_after_borrow);
 
     // Partial repay.
     t.repay(ALICE, "DAI18", 2_500.0);
     t.assert_borrow_near(ALICE, "DAI18", 2_500.0, 1.0);
     t.assert_healthy(ALICE);
 
     // Full repay; overpay to force closure, and the pool refunds the excess.
     t.repay(ALICE, "DAI18", 3_000.0);
     let remaining = t.borrow_balance(ALICE, "DAI18");
     assert!(
         remaining < 1.0,
         "Borrow should be fully repaid (or near-zero), got {}",
         remaining
     );
+    // After full repay-with-overpay the wallet should be near zero (the
+    // surplus was refunded into the borrow position closure path, but any
+    // wallet residue must be small relative to the original 5000 DAI).
+    let dai_wallet_final = t.token_balance(ALICE, "DAI18");
+    assert!(dai_wallet_final < 100.0, "DAI wallet should be small after full repay, got {}", dai_wallet_final);
 }
```

> Note: confirm refund behavior in the harness's `repay` helper (test-harness/src/user.rs) — the repay helper auto-mints the repay amount, so the residual wallet check should bound by the auto-minted dust rather than expect zero. The intent is to catch a regression that double-pulls or fails to refund.

### `decimal_diversity_tests.rs::test_withdraw_cross_decimal_hf_check`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Asserts post-state via `assert_supply_near` (line 317) but does not check Alice's USDC wallet increased by ~3,000 after the withdraw. The cross-decimal withdraw is the entire focus of the test: the asset moved from supply to wallet, and the wallet delta is the most direct check that the rescale produced the right raw amount. Lines 304-318.

**Patch (suggested):**
```diff
--- a/test-harness/tests/decimal_diversity_tests.rs
+++ b/test-harness/tests/decimal_diversity_tests.rs
@@ -307,12 +307,16 @@ fn test_withdraw_cross_decimal_hf_check() {
     t.supply(ALICE, "USDC6", 10_000.0);
     t.borrow(ALICE, "DAI18", 4_000.0);
+    let usdc_before_withdraw = t.token_balance(ALICE, "USDC6");
 
     // Withdraw $3,000 USDC; this must succeed (remaining $7,000 at 80%
     // threshold = $5,600 > $4,000).
     t.withdraw(ALICE, "USDC6", 3_000.0);
+    let usdc_after_withdraw = t.token_balance(ALICE, "USDC6");
+    let delta = usdc_after_withdraw - usdc_before_withdraw;
+    assert!((delta - 3_000.0).abs() < 1.0, "USDC wallet should grow by ~3000 from withdraw, got delta={}", delta);
     t.assert_healthy(ALICE);
     t.assert_supply_near(ALICE, "USDC6", 7_000.0, 1.0);
 }
```

### `decimal_diversity_tests.rs::test_liquidation_6dec_collateral_18dec_debt`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** Only asserts `debt_after < 7_500.0` (lines 350-355). The lower bound is meaningless — a regression that liquidated 1 raw unit would still pass. The test should bound the debt reduction near the 3,000 DAI repay amount and verify the liquidator received USDC collateral (token-balance delta). Lines 325-356.

**Patch (suggested):**
```diff
--- a/test-harness/tests/decimal_diversity_tests.rs
+++ b/test-harness/tests/decimal_diversity_tests.rs
@@ -333,18 +333,22 @@ fn test_liquidation_6dec_collateral_18dec_debt() {
     // Price drop: USDC falls to $0.90, pushing HF below 1.0.
     t.set_price("USDC6", usd(1) * 90 / 100);
     t.advance_and_sync(1000);
 
     let hf = t.health_factor(ALICE);
     assert!(
         hf < 1.0,
         "HF should be below 1.0 after price drop, got {}",
         hf
     );
 
     // Liquidate: repay 3,000 DAI of Alice's debt.
+    let liq_usdc_before = t.token_balance(LIQUIDATOR, "USDC6");
+    let debt_before = t.borrow_balance(ALICE, "DAI18");
     t.liquidate(LIQUIDATOR, ALICE, "DAI18", 3_000.0);
 
     // Confirm the debt dropped.
     let debt_after = t.borrow_balance(ALICE, "DAI18");
-    assert!(
-        debt_after < 7_500.0,
-        "Debt should be reduced after liquidation, got {}",
-        debt_after
-    );
+    let debt_delta = debt_before - debt_after;
+    assert!(debt_delta > 2_500.0 && debt_delta < 3_500.0,
+        "Debt should drop by ~3000 DAI: before={}, after={}, delta={}",
+        debt_before, debt_after, debt_delta);
+    let liq_usdc_after = t.token_balance(LIQUIDATOR, "USDC6");
+    assert!(liq_usdc_after > liq_usdc_before, "Liquidator should receive USDC6 collateral: before={}, after={}", liq_usdc_before, liq_usdc_after);
 }
```

### `decimal_diversity_tests.rs::test_liquidation_18dec_collateral_6dec_debt`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** Same shape as the previous test: `debt_after < 7_500.0` (line 383) is too loose, and the liquidator's DAI collateral receipt is never verified. Lines 363-384.

**Patch (suggested):**
```diff
--- a/test-harness/tests/decimal_diversity_tests.rs
+++ b/test-harness/tests/decimal_diversity_tests.rs
@@ -370,12 +370,17 @@ fn test_liquidation_18dec_collateral_6dec_debt() {
     // Price drop: DAI falls to $0.90.
     t.set_price("DAI18", usd(1) * 90 / 100);
     t.advance_and_sync(1000);
 
     let hf = t.health_factor(ALICE);
     assert!(hf < 1.0, "HF should be below 1.0, got {}", hf);
 
+    let liq_dai_before = t.token_balance(LIQUIDATOR, "DAI18");
+    let debt_before = t.borrow_balance(ALICE, "USDC6");
     t.liquidate(LIQUIDATOR, ALICE, "USDC6", 3_000.0);
 
     let debt_after = t.borrow_balance(ALICE, "USDC6");
-    assert!(debt_after < 7_500.0, "Debt reduced, got {}", debt_after);
+    let debt_delta = debt_before - debt_after;
+    assert!(debt_delta > 2_500.0 && debt_delta < 3_500.0,
+        "Debt should drop by ~3000 USDC6: before={}, after={}, delta={}", debt_before, debt_after, debt_delta);
+    let liq_dai_after = t.token_balance(LIQUIDATOR, "DAI18");
+    assert!(liq_dai_after > liq_dai_before, "Liquidator should receive DAI18 collateral: before={}, after={}", liq_dai_before, liq_dai_after);
 }
```

### `decimal_diversity_tests.rs::test_multi_user_mixed_decimals`

**Severity:** none

### `decimal_diversity_tests.rs::test_low_value_high_quantity_7dec`

**Severity:** none

---

## Cross-cutting patterns

The audit surfaces three repeating issues across this domain. First, several rejection tests for account-creation invariants (`test_emode_rejects_with_isolation`, `test_emode_deprecated_blocks_new_accounts`, `test_emode_remove_category_deprecates`, `test_isolated_rejects_emode`) catch panics from the test harness's own `create_account_direct` (test-harness/src/user.rs:53-107) — that helper writes account state directly to storage and short-circuits validation with native Rust `assert!` calls. The contract's `validate_e_mode_isolation_exclusion`, `ensure_e_mode_not_deprecated`, and `ensure_e_mode_compatible_with_asset` panics are never exercised; the tests pass vacuously regardless of whether the contract still enforces the rule. The fix is to drive the contract path via `try_supply` after creating an e-mode account against an isolated/deprecated configuration. Second, multiple negative tests use bare `result.is_err()` or `catch_unwind` instead of `assert_contract_error(result, errors::SPECIFIC_CODE)` (`test_supply_position_limit_exceeded`, `test_isolated_rejects_swap_collateral`, `test_emode_rejects_threshold_lte_ltv`, `test_remove_rejects_with_positions`, `test_account_owner_verified`); a pin-coded check would catch regressions that swap an error code for a less informative one or accidentally route through a different validator. Third, several success-path tests assert position/HF post-state but never verify wallet token-balance deltas (`test_supply_to_existing_account`, `test_supply_multiple_assets_bulk`, `test_supply_isolated_account_single_asset`, `test_isolated_borrow_enabled_asset`, `test_repay_cross_decimal`, `test_withdraw_cross_decimal_hf_check`, the cross-decimal liquidation pair, and `test_multiple_accounts_per_user`) — a regression where accounting succeeds but the underlying token transfer is dropped or mis-scaled would not be caught.
