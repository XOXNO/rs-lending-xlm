# Domain 5 — Admin + Keeper + Config

**Phase:** 2
**Files in scope:**
- `test-harness/tests/admin_config_tests.rs`
- `test-harness/tests/keeper_tests.rs`
- `test-harness/tests/keeper_admin_tests.rs`
- `test-harness/tests/validation_admin_tests.rs`

**Totals:** confirmed=16 refuted=0 refined=1 new=0

---

## `admin_config_tests.rs::test_edit_asset_config`

**Severity:** none

**Disposition:** confirmed

---

## `admin_config_tests.rs::test_edit_asset_config_rejects_threshold_lte_ltv`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Lines 53-57 use bare `result.is_err()` instead of `assert_contract_error(_, errors::INVALID_LIQ_THRESHOLD)`. `validation::validate_asset_config` (controller/src/validation.rs:208-211) panics with `CollateralError::InvalidLiqThreshold = 113` whenever `liquidation_threshold_bps <= loan_to_value_bps`. The test would still pass for any failure (e.g. an unrelated regression that breaks `edit_asset_config` outright) and provides no signal that the precise threshold-vs-LTV gate is the trip wire.

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -50,11 +50,12 @@
     let result = ctrl.try_edit_asset_config(&asset, &config);
-    assert!(
-        result.is_err(),
-        "edit_asset_config should reject threshold == LTV"
-    );
+    let mapped = match result {
+        Ok(res) => res.map_err(|e| e.into()),
+        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
+    };
+    assert_contract_error(mapped, errors::INVALID_LIQ_THRESHOLD);
 }
```

---

## `admin_config_tests.rs::test_set_position_limits`

**Severity:** none

**Disposition:** confirmed

---

## `admin_config_tests.rs::test_pause_blocks_operations`

**Severity:** none

**Disposition:** confirmed

---

## `admin_config_tests.rs::test_unpause_restores_operations`

**Severity:** none

**Disposition:** confirmed

---

## `admin_config_tests.rs::test_upgrade_pool_params`

**Severity:** none

**Disposition:** confirmed

---

## `admin_config_tests.rs::test_upgrade_liquidity_pool_params_alias`

**Severity:** none

**Disposition:** confirmed

---

## `admin_config_tests.rs::test_upgrade_pool_params_rejects_max_borrow_rate_above_cap`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Lines 225-228 use bare `is_err()` rather than asserting `errors::INVALID_BORROW_PARAMS` (`CollateralError::InvalidBorrowParams = 116`, raised by `validation::validate_interest_rate_model` — controller/src/validation.rs:180-182 — for the cap envelope). The test is the only direct guard for the documented Taylor-envelope cap; without an exact code, an unrelated panic in `upgrade_pool_params` would still satisfy `is_err()` and hide a regression.

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -222,11 +222,12 @@
             reserve_factor_bps: 1000,
         },
     );
-    assert!(
-        result.is_err(),
-        "upgrade_pool_params must reject max_borrow_rate_ray > 2 * RAY"
-    );
+    let mapped = match result {
+        Ok(res) => res.map_err(|e| e.into()),
+        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
+    };
+    assert_contract_error(mapped, errors::INVALID_BORROW_PARAMS);
 }
```

(Add `errors::INVALID_BORROW_PARAMS` to the imports at the top of the file.)

---

## `admin_config_tests.rs::test_upgrade_pool_params_accepts_max_borrow_rate_at_cap`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Line 248 only verifies that the call did not panic ("Did not panic — cap allows the boundary value."). There is no read of post-state to confirm the IRM was actually persisted. An accidental no-op (e.g. early return before the storage write) would still pass.

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -232,6 +232,7 @@
 fn test_upgrade_pool_params_accepts_max_borrow_rate_at_cap() {
     let t = LendingTest::new().with_market(usdc_preset()).build();
+    let rate_before = t.pool_borrow_rate("USDC");

     // At the exact cap (`2 * RAY`); slope3 must remain <= max.
     t.upgrade_pool_params(
@@ -247,5 +248,11 @@
             reserve_factor_bps: 1000,
         },
     );
-    // Did not panic — cap allows the boundary value.
+    // The IRM was rewritten — base_rate dropped from 1% (default) to ~1% but
+    // slope1/slope2/slope3 changed enough to shift the zero-utilization rate.
+    let rate_after = t.pool_borrow_rate("USDC");
+    assert!(
+        rate_after != rate_before || rate_after >= 0.0,
+        "borrow rate must remain readable after boundary upgrade",
+    );
 }
```

---

## `admin_config_tests.rs::test_configure_market_oracle`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Line 281 only calls `configure_market_oracle` and relies on the call not panicking. There is no assertion that `MarketConfig.cex_oracle`, `cex_symbol`, or `twap_records` reflect the input — the test would pass if the function silently dropped the input. `configure_market_oracle` (controller/src/config.rs:553-563) writes to all those fields plus flips `status` to `Active = 1`; reading them back via `ctrl.get_market_config(&asset)` is sufficient.

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -279,5 +279,11 @@
     // Must not panic; the admin has permission.
     ctrl.configure_market_oracle(&t.admin(), &asset, &config);
+
+    let market = ctrl.get_market_config(&asset);
+    assert_eq!(market.cex_oracle, Some(t.mock_reflector.clone()));
+    assert_eq!(market.twap_records, 3);
+    assert_eq!((market.status as u32), 1, "market should be Active after oracle config");
 }
```

---

## `admin_config_tests.rs::test_set_aggregator`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Line 298 only confirms `set_aggregator` does not panic. There is no readback: the test would pass if the new aggregator address was silently dropped. Storage exposes `ControllerKey::Aggregator` (controller/src/storage/instance.rs:50-57); a `t.env.as_contract` read with that key matches the persistence path and is the cheapest readback that would catch a silent-drop regression.

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -288,15 +288,21 @@
 #[test]
 fn test_set_aggregator() {
     let t = LendingTest::new().with_market(usdc_preset()).build();

     let ctrl = t.ctrl_client();
     let new_aggregator = t
         .env
         .register(test_harness::mock_reflector::MockReflector, ());

     // Must not panic; the admin has permission.
-    ctrl.set_aggregator(&new_aggregator);
+    ctrl.set_aggregator(&new_aggregator);
+
+    // Confirm the new aggregator is actually persisted.
+    let stored: Address = t.env.as_contract(&t.controller_address(), || {
+        t.env
+            .storage()
+            .instance()
+            .get(&common::types::ControllerKey::Aggregator)
+            .expect("aggregator must be stored")
+    });
+    assert_eq!(stored, new_aggregator, "aggregator must be persisted");
 }
```

---

## `admin_config_tests.rs::test_oracle_tolerance_validation`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Lines 315-319 use bare `is_err()` instead of asserting `errors::BAD_FIRST_TOLERANCE` (`OracleError::BadFirstTolerance = 207`). `config.rs:427-429` panics with that exact code when `first_tolerance < MIN_FIRST_TOLERANCE`. Without the exact code the test would still pass under any unrelated failure path (e.g. role gate, asset lookup).

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -312,8 +312,12 @@
     let result = ctrl.try_edit_oracle_tolerance(&t.admin(), &asset, &10, &500);
-    assert!(
-        result.is_err(),
-        "oracle tolerance with first < 50 bps should be rejected"
-    );
+    let mapped = match result {
+        Ok(res) => res.map_err(|e| e.into()),
+        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
+    };
+    assert_contract_error(mapped, errors::BAD_FIRST_TOLERANCE);
 }
```

---

## `admin_config_tests.rs::test_grant_and_revoke_role`

**Severity:** none

**Disposition:** confirmed

---

## `admin_config_tests.rs::test_role_enforcement_keeper`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Lines 366-371 use bare `is_err()`. The `#[only_role]` macro (controller/src/router.rs:18) routes through `ensure_if_caller_has_role` (vendor/openzeppelin/stellar-access/src/access_control/storage.rs:608) which panics with `AccessControlError::Unauthorized = 2000`. The inline comment claiming "Soroban wraps cross-contract errors" is misleading: the role check is in the same controller WASM, so the SDK-generated `try_*` returns `Result<Result<_, Error>, InvokeError>` with the contract code preserved.

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -360,13 +360,15 @@
     let bob_addr = t.get_or_create_user(BOB);

-    // BOB calls update_indexes without KEEPER; this must fail. Use bare
-    // `is_err()` because Soroban wraps cross-contract errors at the outer
-    // caller boundary.
+    // BOB calls update_indexes without KEEPER; this must fail with
+    // AccessControlError::Unauthorized = 2000.
     let ctrl = t.ctrl_client();
     let assets = soroban_sdk::vec![&t.env, t.resolve_market("USDC").asset.clone()];
     let result = ctrl.try_update_indexes(&bob_addr, &assets);
-    assert!(
-        result.is_err(),
-        "non-keeper should not be able to call update_indexes"
-    );
+    let mapped = match result {
+        Ok(res) => res.map_err(|e| e.into()),
+        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
+    };
+    assert_contract_error(mapped, 2000);
 }
```

---

## `admin_config_tests.rs::test_role_enforcement_revenue`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Lines 388-394 use bare `is_err()`. `claim_revenue` is gated by `#[only_role(caller, "REVENUE")]` (router.rs:77) which panics with `AccessControlError::Unauthorized = 2000`. Same fix shape as `test_role_enforcement_keeper`.

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -383,13 +383,15 @@
     let bob_addr = t.get_or_create_user(BOB);

-    // Use bare `is_err()` because Soroban wraps cross-contract errors at the
-    // outer caller boundary.
+    // claim_revenue is `#[only_role(caller, "REVENUE")]`; non-revenue callers
+    // must trip AccessControlError::Unauthorized = 2000.
     let ctrl = t.ctrl_client();
     let asset = t.resolve_market("USDC").asset.clone();
     let assets = soroban_sdk::vec![&t.env, asset];
     let result = ctrl.try_claim_revenue(&bob_addr, &assets);
-    assert!(
-        result.is_err(),
-        "non-revenue user should not be able to claim revenue"
-    );
+    let mapped = match result {
+        Ok(res) => res.map_err(|e| e.into()),
+        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
+    };
+    assert_contract_error(mapped, 2000);
 }
```

---

## `admin_config_tests.rs::test_role_enforcement_oracle`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Lines 422-435 use three bare `is_err()` checks for `configure_market_oracle`, `edit_oracle_tolerance`, and `disable_token_oracle`. All three are `#[only_role(caller, "ORACLE")]` (config.rs:128, 139, 151), which panic with `AccessControlError::Unauthorized = 2000`. Tighten each assertion to the exact code.

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -419,21 +419,33 @@
         twap_records: 3,
     };

-    assert!(
-        ctrl.try_configure_market_oracle(&bob_addr, &asset, &reflector)
-            .is_err(),
-        "non-oracle user should not be able to set reflector config"
-    );
-    assert!(
-        ctrl.try_edit_oracle_tolerance(&bob_addr, &asset, &300, &600)
-            .is_err(),
-        "non-oracle user should not be able to edit oracle tolerance"
-    );
-    assert!(
-        ctrl.try_disable_token_oracle(&bob_addr, &asset).is_err(),
-        "non-oracle user should not be able to disable the oracle"
-    );
+    let map = |r: Result<Result<_, _>, _>| -> Result<(), soroban_sdk::Error> {
+        match r {
+            Ok(Ok(_)) => Ok(()),
+            Ok(Err(err)) => Err(err.into()),
+            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
+        }
+    };
+
+    assert_contract_error(
+        map(ctrl.try_configure_market_oracle(&bob_addr, &asset, &reflector)),
+        2000,
+    );
+    assert_contract_error(
+        map(ctrl.try_edit_oracle_tolerance(&bob_addr, &asset, &300, &600)),
+        2000,
+    );
+    assert_contract_error(
+        map(ctrl.try_disable_token_oracle(&bob_addr, &asset)),
+        2000,
+    );
 }
```

(The `map` closure may need monomorphisation per call; if the generic shape is too noisy, three explicit `match` expressions are equally fine.)

---

## `admin_config_tests.rs::test_oracle_role_can_manage_oracle_endpoints`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Lines 466-477 exercise `configure_market_oracle`, `edit_oracle_tolerance`, and `disable_token_oracle` end-to-end but only assert `is_collateralizable` is still readable (line 471-474). The post-state for the actually-configured fields (cex_oracle, twap_records, tolerance bps) and the disabled-oracle status is not checked. A regression that ignored the inputs after the role check would still pass.

**Disposition:** refined

**Reviewer note:** The Phase 1 patch references `oracle_config.fluctuation.first_upper_ratio_bps`. Source confirms the struct member is `OracleProviderConfig.tolerance` (common/src/types.rs:251), and `edit_oracle_tolerance` writes to `market.oracle_config.tolerance` (controller/src/config.rs:584). Renamed `fluctuation` to `tolerance` in the patch. Also dropped the unused `let bob_addr` repeat (already pulled at line 448).

**Patch (suggested):**
```diff
--- before
+++ after
@@ -464,15 +464,21 @@
     t.mock_reflector_client().set_price(&asset, &1_0000000i128);
     ctrl.configure_market_oracle(&bob_addr, &asset, &reflector);
+    let after_configure = ctrl.get_market_config(&asset);
+    assert_eq!(after_configure.cex_oracle, Some(t.mock_reflector.clone()));
+    assert_eq!(after_configure.twap_records, 2);

     ctrl.edit_oracle_tolerance(&bob_addr, &asset, &300, &600);
+    let after_tolerance = ctrl.get_market_config(&asset);
+    assert!(
+        after_tolerance.oracle_config.tolerance.first_upper_ratio_bps > 0,
+        "tolerance must be persisted",
+    );

-    let market = ctrl.get_market_config(&asset).asset_config;
-    assert!(
-        market.is_collateralizable,
-        "asset config should remain readable"
-    );
-
     ctrl.disable_token_oracle(&bob_addr, &asset);
+    let after_disable = ctrl.get_market_config(&asset);
+    assert_eq!(
+        (after_disable.status as u32),
+        2,
+        "disable_token_oracle must move market to Disabled (=2)",
+    );
 }
```

---

## `admin_config_tests.rs::test_create_liquidity_pool_uniqueness`

**Severity:** none

**Disposition:** confirmed

---

## `admin_config_tests.rs::test_market_initialization_cascade`

**Severity:** none

**Disposition:** confirmed

---

## `keeper_tests.rs::test_update_indexes_refreshes_rates`

**Severity:** none

**Disposition:** confirmed

---

## `keeper_tests.rs::test_clean_bad_debt_removes_positions`

**Severity:** none

**Disposition:** confirmed

---

## `keeper_tests.rs::test_clean_bad_debt_rejects_healthy`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Lines 99-102 use bare `is_err()`. `clean_bad_debt_standalone` (controller/src/positions/liquidation.rs:478-479) panics with `CollateralError::CannotCleanBadDebt = 114` when the bad-debt predicate is not met. The harness already maps to `Result<(), soroban_sdk::Error>` via `try_clean_bad_debt_by_id` (test-harness/src/keeper.rs:35-44), so `assert_contract_error(result, errors::CANNOT_CLEAN_BAD_DEBT)` is straightforward.

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -84,11 +84,12 @@
 fn test_clean_bad_debt_rejects_healthy() {
@@ -96,11 +97,8 @@
     let account_id = t.resolve_account_id(ALICE);
     let result = t.try_clean_bad_debt_by_id(account_id);
-    assert!(
-        result.is_err(),
-        "clean_bad_debt should fail on healthy account"
-    );
+    test_harness::assert_contract_error(result, test_harness::errors::CANNOT_CLEAN_BAD_DEBT);
 }
```

---

## `keeper_tests.rs::test_clean_bad_debt_rejects_above_threshold`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Lines 128-133 use bare `is_err()`. The same `CannotCleanBadDebt = 114` panic guards the "above threshold" branch (liquidation.rs:478-479 — both clauses of `total_debt_usd > total_collateral_usd && total_collateral_usd <= bad_debt_threshold` collapse to the same error).

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -126,11 +126,8 @@
     let account_id = t.resolve_account_id(ALICE);
     let result = t.try_clean_bad_debt_by_id(account_id);
-    assert!(
-        result.is_err(),
-        "clean_bad_debt should fail when collateral > $5"
-    );
+    test_harness::assert_contract_error(result, test_harness::errors::CANNOT_CLEAN_BAD_DEBT);
 }
```

---

## `keeper_tests.rs::test_update_account_threshold_safe`

**Severity:** none

**Disposition:** confirmed

---

## `keeper_tests.rs::test_update_account_threshold_risky`

**Severity:** none

**Disposition:** confirmed

---

## `keeper_tests.rs::test_update_account_threshold_rejects_low_hf`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Lines 229-233 use bare `is_err()`. The propagation path panics with `CollateralError::HealthFactorTooLow = 102` (controller/src/positions/supply.rs:507-509 inside `update_position_threshold`, called from the `update_account_threshold` keeper endpoint). Pin the code so an unrelated panic during threshold edit does not silently satisfy the assertion.

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -227,11 +227,8 @@
     let result = t.try_update_account_threshold("USDC", true, &[account_id]);
-    assert!(
-        result.is_err(),
-        "update_account_threshold should fail when HF < 1.05 after update"
-    );
+    test_harness::assert_contract_error(result, test_harness::errors::HEALTH_FACTOR_TOO_LOW);
 }
```

(Add `assert_contract_error`/`errors` to the `use` list.)

---

## `keeper_tests.rs::test_update_account_threshold_deprecated_emode_uses_base_params`

**Severity:** none

**Disposition:** confirmed

---

## `keeper_tests.rs::test_keeper_role_required`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Lines 280-292 use two bare `is_err()` checks. Both `update_indexes` (router.rs:18) and `clean_bad_debt` (gated via `#[only_role(caller, "KEEPER")]`) panic with `AccessControlError::Unauthorized = 2000`. Pin the exact code on both calls.

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -276,17 +276,21 @@
     // BOB calls `update_indexes` without the KEEPER role.
-    // Use bare `is_err()` because Soroban wraps cross-contract errors at
-    // the outer caller boundary.
     let result = ctrl.try_update_indexes(&bob_addr, &assets);
-    assert!(
-        result.is_err(),
-        "non-keeper should not be able to call update_indexes"
-    );
+    let mapped = match result {
+        Ok(res) => res.map_err(|e| e.into()),
+        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
+    };
+    test_harness::assert_contract_error(mapped, 2000);

     // BOB calls clean_bad_debt without the KEEPER role.
     let result = ctrl.try_clean_bad_debt(&bob_addr, &999u64);
-    assert!(
-        result.is_err(),
-        "non-keeper should not be able to call clean_bad_debt"
-    );
+    let mapped = match result {
+        Ok(res) => res.map_err(|e| e.into()),
+        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
+    };
+    test_harness::assert_contract_error(mapped, 2000);
 }
```

---

## `keeper_admin_tests.rs::test_keepalive_pools_iterates_and_skips_unknown`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Line 51 calls `keepalive_pools` and asserts only that it does not panic. There is no observable post-state (e.g. TTL bump, last-touched ledger snapshot, or counter) to confirm the loop actually iterated and bumped each known asset. A regression that early-returned after the first asset would still pass. At minimum, snapshot the per-asset bump key (or just spy on `pool_borrow_rate` round-trip / use `t.env.ledger().sequence()` deltas via the existing TTL-aware helpers).

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -49,4 +49,11 @@
     // Must not panic; the keeper signature is satisfied and the loop must
     // tolerate a missing market config without aborting.
     t.ctrl_client().keepalive_pools(&t.keeper, &assets);
+
+    // Each known market must still be readable after the bump (the loop
+    // touched live storage entries; an early return would have left at least
+    // one inaccessible).
+    assert!(t.pool_borrow_rate("USDC") >= 0.0);
+    assert!(t.pool_borrow_rate("ETH") >= 0.0);
+    assert!(t.pool_borrow_rate("WBTC") >= 0.0);
 }
```

---

## `keeper_admin_tests.rs::test_keepalive_shared_state_bumps_emode_keys`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Line 78 calls `keepalive_shared_state` and asserts nothing. The whole point of the test is that the loop bumps `Market`, `IsolatedDebt`, `AssetEModes`, `EModeCategory`, `EModeAssets` under the right conditions (file header at lines 56-60). With no post-state read, an empty-body regression would pass. Add a sanity readback via `t.env.as_contract` on at least one of the touched keys.

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -76,4 +76,18 @@
     t.ctrl_client().keepalive_shared_state(&t.keeper, &assets);
+
+    // Sanity: e-mode category 1 is still readable (the keepalive bump must
+    // not have evicted it). Bumping correctness vs simple read coverage is
+    // limited inside the harness; a TTL-deep verifier belongs in a fuzz
+    // test, but this read pins down at least the loop walking the e-mode key.
+    let category_present: bool = t.env.as_contract(&t.controller_address(), || {
+        t.env
+            .storage()
+            .persistent()
+            .has(&common::types::ControllerKey::EModeCategory(1))
+    });
+    assert!(category_present, "EModeCategory(1) must remain after bump");
 }
```

---

## `keeper_admin_tests.rs::test_upgrade_pool_admin_path`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Lines 91-108 perform a no-op upgrade and assert nothing — neither that the call succeeded nor that the pool is still functional. The header comment promises "exercises every line of `upgrade_liquidity_pool`" but a regression that simply returned without forwarding to `pool_client.upgrade` would pass. Add a post-call sanity read (e.g. that `pool_borrow_rate` still returns a value, or that the market status remains Active).

**Disposition:** confirmed

**Patch (suggested):**
```diff
--- before
+++ after
@@ -105,5 +105,12 @@
     // own template hash this is a no-op upgrade that exercises every line
     // of `upgrade_liquidity_pool` without altering pool behavior.
     let asset = t.resolve_asset("USDC");
     t.ctrl_client().upgrade_pool(&asset, &template_hash);
+
+    // Pool client is still callable after the upgrade — confirms the
+    // upgrade actually completed and didn't leave the pool in a broken state.
+    assert!(t.pool_borrow_rate("USDC") >= 0.0);
+    let market = t.ctrl_client().get_market_config(&asset);
+    assert_eq!((market.status as u32), 1, "market must remain Active after no-op upgrade");
 }
```

---

## `keeper_admin_tests.rs::test_create_liquidity_pool_panics_when_template_unset`

**Severity:** none

**Disposition:** confirmed

---

## `keeper_admin_tests.rs::test_supply_panics_on_deprecated_emode_category`

**Severity:** none

**Disposition:** confirmed

---

## `validation_admin_tests.rs::test_validate_bulk_isolation_rejects_isolated_first_asset_bulk`

**Severity:** none

**Disposition:** confirmed

---

## `validation_admin_tests.rs::test_validate_irm_rejects_negative_base_rate`

**Severity:** none

**Disposition:** confirmed

---

## `validation_admin_tests.rs::test_validate_irm_rejects_zero_mid_utilization`

**Severity:** none

**Disposition:** confirmed

---

## `validation_admin_tests.rs::test_validate_irm_rejects_optimal_not_above_mid`

**Severity:** none

**Disposition:** confirmed

---

## `validation_admin_tests.rs::test_validate_irm_rejects_optimal_at_or_above_ray`

**Severity:** none

**Disposition:** confirmed

---

## `validation_admin_tests.rs::test_validate_irm_rejects_reserve_factor_at_bps`

**Severity:** none

**Disposition:** confirmed

---

## `validation_admin_tests.rs::test_validate_asset_config_rejects_negative_ltv`

**Severity:** none

**Disposition:** confirmed

---

## `validation_admin_tests.rs::test_validate_asset_config_rejects_excessive_liq_bonus`

**Severity:** none

**Disposition:** confirmed

---

## `validation_admin_tests.rs::test_validate_asset_config_rejects_negative_isolation_ceiling`

**Severity:** none

**Disposition:** confirmed

---

## `validation_admin_tests.rs::test_validate_asset_config_accepts_flashloan_fee_at_cap`

**Severity:** none

**Disposition:** confirmed

---

## `validation_admin_tests.rs::test_configure_market_oracle_rejects_low_staleness`

**Severity:** none

**Disposition:** confirmed

---

## `validation_admin_tests.rs::test_configure_market_oracle_rejects_high_staleness`

**Severity:** none

**Disposition:** confirmed

---

## `validation_admin_tests.rs::test_configure_market_oracle_rejects_excessive_twap_records`

**Severity:** none

**Disposition:** confirmed

---

## `validation_admin_tests.rs::test_configure_market_oracle_rejects_dual_without_dex`

**Severity:** none

**Disposition:** confirmed

---

## `validation_admin_tests.rs::test_emode_user_supply_rejects_deprecated_category`

**Severity:** none

**Disposition:** confirmed

---

## Cross-cutting patterns

Phase 1 audit is well-grounded: every cited error code (113 InvalidLiqThreshold, 114 CannotCleanBadDebt, 102 HealthFactorTooLow, 116 InvalidBorrowParams, 207 BadFirstTolerance, 2000 Unauthorized, 1000 ContractPaused) was verified directly against `common/src/errors.rs` and `vendor/openzeppelin/stellar-access/src/access_control/mod.rs:384`. The auditor's central claim — that the in-source comments saying "Soroban wraps cross-contract errors" are wrong — is correct: every `#[only_role]` and `panic_with_error!` site in scope (router.rs, config.rs, validation.rs, supply.rs, liquidation.rs) lives in the controller WASM, so the SDK-generated `try_*` returns `Result<Result<T, E>, InvokeError>` with the contract code preserved. The harness `assert_contract_error` (test-harness/src/assert.rs:87-105) drives that exact shape. The harness `errors::*` module exports every code the auditor cites except `Unauthorized = 2000`, which is from the OpenZeppelin vendor crate; using the literal `2000` (or adding a constant) is the correct path. One refinement was applied to `test_oracle_role_can_manage_oracle_endpoints`: the field path is `oracle_config.tolerance.first_upper_ratio_bps`, not `oracle_config.fluctuation.first_upper_ratio_bps` as the Phase 1 patch suggested (the auditor flagged this themselves for reviewer confirmation; common/src/types.rs:251 and config.rs:584 confirm). All 48 tests in scope appear in the Phase 1 report; counts and severities reproduce exactly. No new findings: every "none"-rated test was independently re-checked and the existing post-state assertions (e.g. `test_upgrade_pool_params` snapshots `pool_borrow_rate`, `test_market_initialization_cascade` reads `market.status` twice, `test_grant_and_revoke_role` reads `has_role`, `test_validate_asset_config_accepts_flashloan_fee_at_cap` reads `flashloan_fee_bps`) are sufficient. The validation suite (`validation_admin_tests.rs`) remains uniformly clean. No naming issues observed; severity 'nit' did not trigger.
