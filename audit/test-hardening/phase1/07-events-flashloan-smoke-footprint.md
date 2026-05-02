# Domain 7 — Events + FlashLoan + Smoke + Footprint

**Phase:** 1
**Files in scope:**
- `test-harness/tests/events_tests.rs`
- `test-harness/tests/flash_loan_tests.rs`
- `test-harness/tests/footprint_test.rs`
- `test-harness/tests/smoke_test.rs`

**Totals:** broken=1 weak=14 nit=1 (Phase 1)

---

## events_tests.rs

### `events_tests.rs::test_supply_emits_events`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** The test only counts events with `t.env.events().all().events().len() > 0` (events_tests.rs:27-28). It does not verify a specific supply-related event was emitted, the asset, the amount, or the actor. A regression that emits any unrelated event from a host call would still pass. Use the existing `format!("{:#?}", t.env.events().all())` payload-string idiom (already used in `test_supply_position_event_restores_risk_fields` at events_tests.rs:36-49) to assert at minimum a `supply`/position-update topic and the asset symbol appear, plus the canonical post-state (`assert_supply_near` and a wallet-balance delta).

**Patch (suggested):**
```diff
--- before
+++ after
@@ events_tests.rs:24-29 @@
 #[test]
 fn test_supply_emits_events() {
     let mut t = LendingTest::new().with_market(usdc_preset()).build();
+    let wallet_before = t.token_balance(ALICE, "USDC");
     t.supply(ALICE, "USDC", 10_000.0);
-    let count = t.env.events().all().events().len();
-    assert!(count > 0, "supply should emit events, got {}", count);
+
+    // Post-state: position created and wallet debited.
+    t.assert_supply_near(ALICE, "USDC", 10_000.0, 1.0);
+    let wallet_after = t.token_balance(ALICE, "USDC");
+    assert!(
+        (wallet_before - wallet_after - 10_000.0).abs() < 0.01,
+        "wallet should debit 10k USDC: before={} after={}",
+        wallet_before, wallet_after
+    );
+
+    // Event payload: at least one event references the supplied asset and an
+    // expected position/supply topic, not just "any event was emitted".
+    let dump = format!("{:#?}", t.env.events().all());
+    assert!(
+        dump.contains("supply") || dump.contains("position"),
+        "supply must emit a supply/position event; got:\n{}", dump
+    );
+    assert!(
+        dump.contains("USDC"),
+        "supply event payload must reference USDC; got:\n{}", dump
+    );
}
```

### `events_tests.rs::test_supply_position_event_restores_risk_fields`

**Severity:** none

### `events_tests.rs::test_borrow_emits_events`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Asserts only `count > 0` (events_tests.rs:61-62). Does not verify a borrow-specific topic, the asset, or the actor — passes for any unrelated event. The post-state of the borrow is also not asserted (no debt or wallet check).

**Patch (suggested):**
```diff
--- before
+++ after
@@ events_tests.rs:53-63 @@
 #[test]
 fn test_borrow_emits_events() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
         .build();
     t.supply(ALICE, "USDC", 100_000.0);
+    let wallet_before = t.token_balance(ALICE, "ETH");
     t.borrow(ALICE, "ETH", 1.0);
-    // After borrow, at least the borrow operation's events must be present.
-    let count = t.env.events().all().events().len();
-    assert!(count > 0, "borrow should emit events, got {}", count);
+
+    // Post-state: debt recorded and wallet credited.
+    t.assert_borrow_near(ALICE, "ETH", 1.0, 0.01);
+    let wallet_after = t.token_balance(ALICE, "ETH");
+    assert!(
+        (wallet_after - wallet_before - 1.0).abs() < 0.01,
+        "wallet should credit 1 ETH: before={} after={}",
+        wallet_before, wallet_after
+    );
+
+    // Event payload references borrow topic and the borrowed asset.
+    let dump = format!("{:#?}", t.env.events().all());
+    assert!(
+        dump.contains("borrow") || dump.contains("position"),
+        "borrow must emit borrow/position event; got:\n{}", dump
+    );
+    assert!(dump.contains("ETH"), "borrow payload must reference ETH; got:\n{}", dump);
}
```

### `events_tests.rs::test_withdraw_emits_events`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Counts events only (events_tests.rs:70-71). Does not verify a withdraw-specific topic was emitted nor that the supply position decreased.

**Patch (suggested):**
```diff
--- before
+++ after
@@ events_tests.rs:66-72 @@
 #[test]
 fn test_withdraw_emits_events() {
     let mut t = LendingTest::new().with_market(usdc_preset()).build();
     t.supply(ALICE, "USDC", 10_000.0);
+    let wallet_before = t.token_balance(ALICE, "USDC");
     t.withdraw(ALICE, "USDC", 1_000.0);
-    let count = t.env.events().all().events().len();
-    assert!(count > 0, "withdraw should emit events, got {}", count);
+
+    // Post-state: supply reduced, wallet credited.
+    t.assert_supply_near(ALICE, "USDC", 9_000.0, 1.0);
+    let wallet_after = t.token_balance(ALICE, "USDC");
+    assert!(
+        (wallet_after - wallet_before - 1_000.0).abs() < 0.01,
+        "wallet should credit 1k USDC: before={} after={}",
+        wallet_before, wallet_after
+    );
+
+    let dump = format!("{:#?}", t.env.events().all());
+    assert!(
+        dump.contains("withdraw") || dump.contains("position"),
+        "withdraw must emit withdraw/position event; got:\n{}", dump
+    );
+    assert!(dump.contains("USDC"), "withdraw payload must reference USDC; got:\n{}", dump);
}
```

### `events_tests.rs::test_repay_emits_events`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Counts events only (events_tests.rs:83-84). Does not verify a repay-specific topic was emitted nor that the debt decreased.

**Patch (suggested):**
```diff
--- before
+++ after
@@ events_tests.rs:75-85 @@
 #[test]
 fn test_repay_emits_events() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
         .build();
     t.supply(ALICE, "USDC", 100_000.0);
     t.borrow(ALICE, "ETH", 1.0);
+    let debt_before = t.borrow_balance(ALICE, "ETH");
     t.repay(ALICE, "ETH", 0.5);
-    let count = t.env.events().all().events().len();
-    assert!(count > 0, "repay should emit events, got {}", count);
+
+    // Post-state: debt decreased by ~0.5 ETH.
+    let debt_after = t.borrow_balance(ALICE, "ETH");
+    assert!(
+        (debt_before - debt_after - 0.5).abs() < 0.01,
+        "debt should drop by 0.5 ETH: before={} after={}",
+        debt_before, debt_after
+    );
+
+    let dump = format!("{:#?}", t.env.events().all());
+    assert!(
+        dump.contains("repay") || dump.contains("position"),
+        "repay must emit repay/position event; got:\n{}", dump
+    );
+    assert!(dump.contains("ETH"), "repay payload must reference ETH; got:\n{}", dump);
}
```

### `events_tests.rs::test_liquidation_emits_many_events`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Asserts only `count >= 3` (events_tests.rs:101-105). Three arbitrary events of any kind would pass; the test does not verify a liquidation/seizure topic, the seized asset, or the repaid debt asset. Reinforce with topic checks and post-state assertions (debt drop + collateral seizure on the liquidator's wallet).

**Patch (suggested):**
```diff
--- before
+++ after
@@ events_tests.rs:88-106 @@
 #[test]
 fn test_liquidation_emits_many_events() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
         .build();
     t.supply(ALICE, "USDC", 10_000.0);
     t.borrow(ALICE, "ETH", 3.0);
     t.set_price("USDC", usd_cents(50));
+    let debt_before = t.borrow_balance(ALICE, "ETH");
+    let liq_usdc_before = t.token_balance(LIQUIDATOR, "USDC");
     t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
-    // Liquidation combines token transfers, position updates, and seizure.
-    // Even within Soroban's per-invocation event scope, the call itself
-    // must emit several events: debt repay, seizure, and position updates.
-    let count = t.env.events().all().events().len();
-    assert!(
-        count >= 3,
-        "liquidation should emit >= 3 events, got {}",
-        count
-    );
+    // Post-state: debt dropped, liquidator received collateral.
+    let debt_after = t.borrow_balance(ALICE, "ETH");
+    assert!(debt_after < debt_before, "debt should drop after liquidation");
+    let liq_usdc_after = t.token_balance(LIQUIDATOR, "USDC");
+    assert!(
+        liq_usdc_after > liq_usdc_before,
+        "liquidator must receive USDC collateral: before={} after={}",
+        liq_usdc_before, liq_usdc_after
+    );
+
+    // Event payload: liquidation invocation must reference both assets and a
+    // liquidation/seizure topic, not just "many events".
+    let all = t.env.events().all();
+    let count = all.events().len();
+    assert!(count >= 3, "liquidation should emit >= 3 events, got {}", count);
+    let dump = format!("{:#?}", all);
+    assert!(
+        dump.contains("liquidation") || dump.contains("seizure") || dump.contains("liquidate"),
+        "liquidation must emit liquidation/seizure topic; got:\n{}", dump
+    );
+    assert!(dump.contains("ETH") && dump.contains("USDC"),
+        "liquidation payload must reference both assets; got:\n{}", dump);
}
```

### `events_tests.rs::test_add_emode_emits_events`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Counts events only (events_tests.rs:119-120). Does not verify an e-mode-related topic, the category id (9700/9800/200), or that the category is now retrievable.

**Patch (suggested):**
```diff
--- before
+++ after
@@ events_tests.rs:115-121 @@
 #[test]
 fn test_add_emode_emits_events() {
     let t = LendingTest::new().with_market(usdc_preset()).build();
     t.ctrl_client()
         .add_e_mode_category(&9700i128, &9800i128, &200i128);
-    let count = t.env.events().all().events().len();
-    assert!(count > 0, "add_e_mode should emit events, got {}", count);
+    // Event payload: must reference the e-mode topic and the configured BPS.
+    let dump = format!("{:#?}", t.env.events().all());
+    assert!(
+        dump.contains("e_mode") || dump.contains("emode") || dump.contains("category"),
+        "add_e_mode must emit e-mode topic; got:\n{}", dump
+    );
+    assert!(
+        dump.contains("9700") && dump.contains("9800") && dump.contains("200"),
+        "add_e_mode payload must include configured BPS values; got:\n{}", dump
+    );
}
```

### `events_tests.rs::test_index_sync_emits_events`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Counts events only (events_tests.rs:132-133). Does not verify a sync/interest-accrual topic was emitted nor that indexes actually advanced. A future no-op sync path would silently pass.

**Patch (suggested):**
```diff
--- before
+++ after
@@ events_tests.rs:124-134 @@
 #[test]
 fn test_index_sync_emits_events() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
         .build();
     t.supply(ALICE, "USDC", 100_000.0);
     t.borrow(ALICE, "ETH", 1.0);
+    let debt_before = t.borrow_balance(ALICE, "ETH");
     t.advance_and_sync(days(1));
-    let count = t.env.events().all().events().len();
-    assert!(count > 0, "sync should emit events, got {}", count);
+
+    // Post-state: debt grew, indicating the sync actually accrued interest.
+    let debt_after = t.borrow_balance(ALICE, "ETH");
+    assert!(debt_after > debt_before, "sync must accrue interest: {} -> {}", debt_before, debt_after);
+
+    let dump = format!("{:#?}", t.env.events().all());
+    assert!(
+        dump.contains("sync") || dump.contains("index") || dump.contains("accrue"),
+        "sync must emit sync/index event; got:\n{}", dump
+    );
}
```

### `events_tests.rs::test_isolated_borrow_emits_debt_ceiling_event`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Asserts only `count >= 2` (events_tests.rs:155-160). The test claims it covers a debt-ceiling event but never checks the payload contains a debt-ceiling topic or the configured ceiling. Two unrelated events would pass.

**Patch (suggested):**
```diff
--- before
+++ after
@@ events_tests.rs:151-161 @@
     t.create_isolated_account(ALICE, "ETH");
     t.supply(ALICE, "ETH", 10.0);
     t.borrow(ALICE, "USDC", 1_000.0);
-    // Isolated borrow emits position-update and debt-ceiling events.
-    let count = t.env.events().all().events().len();
-    assert!(
-        count >= 2,
-        "isolated borrow should emit >= 2 events, got {}",
-        count
-    );
+
+    // Post-state: borrow position exists for USDC under the isolated account.
+    t.assert_borrow_near(ALICE, "USDC", 1_000.0, 1.0);
+
+    // Event payload must include a debt-ceiling/isolation topic and reference
+    // the ETH (isolated) and USDC (borrowed) markets.
+    let all = t.env.events().all();
+    assert!(all.events().len() >= 2, "isolated borrow should emit >= 2 events");
+    let dump = format!("{:#?}", all);
+    assert!(
+        dump.contains("debt_ceiling") || dump.contains("isolation") || dump.contains("isolated"),
+        "isolated borrow must emit a debt-ceiling/isolation topic; got:\n{}", dump
+    );
+    assert!(dump.contains("ETH") && dump.contains("USDC"),
+        "isolated borrow payload must reference both assets; got:\n{}", dump);
}
```

---

## flash_loan_tests.rs

### `flash_loan_tests.rs::test_flash_loan_mock_auth_limitation_documented`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** Asserts only `result.is_err()` plus a negative check that the error is not one of three precondition codes (flash_loan_tests.rs:44-61). Because no real success path is exercised in this file (the harness limitation forbids it under recording-mode mock_all_auths), the *only* meaningful flash-loan happy-path coverage in the integration suite is missing. The test does not measure pool/token deltas, does not pin the host-error variant (`InvokeError`), and tolerates regressions where the receiver call genuinely succeeds and emits the wrong post-state. Pin the failure shape to "host/auth-level error, not a contract error" using `try_flash_loan`'s actual control flow. Also assert pool reserves and the receiver's balances are unchanged after the rollback — atomicity is the protocol property worth proving here.

**Patch (suggested):**
```diff
--- before
+++ after
@@ flash_loan_tests.rs:22-62 @@
 #[test]
 fn test_flash_loan_mock_auth_limitation_documented() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
         .build();

     // Supply liquidity so the pool has funds.
     t.supply(ALICE, "USDC", 100_000.0);
     t.borrow(ALICE, "ETH", 1.0);

     // Advance and sync to generate baseline revenue.
     t.advance_and_sync(days(30));

     let receiver = t.deploy_flash_loan_receiver();
+    let pool_before = t.pool_reserves("USDC");
     let result = t.try_flash_loan(BOB, "USDC", 10_000.0, &receiver);

-    // `try_flash_loan` already flattens to `Result<(), soroban_sdk::Error>`.
-    // Recording-mode mock_all_auths cannot record the nested SAC mint auth,
-    // so the call must not succeed. An `Ok` here means either (a) the SDK
-    // now records the nested auth (un-ignore the property test in
-    // fuzz_strategy_flashloan) or (b) a new code path falsely reports
-    // success. Both cases warrant investigation.
     assert!(
         result.is_err(),
         "flash loan with good receiver must not return Ok under recording-mode mock_all_auths: {:?}",
         result
     );
-    // Sanity: the returned error must not match FLASHLOAN_NOT_ENABLED or any
-    // other controller-side precondition error. Such a code would mean a
-    // guard fired before the receiver invocation -- a regression.
     if let Err(err) = &result {
         for regression_code in [401u32, 400u32, 14u32] {
             let predictable = soroban_sdk::Error::from_contract_error(regression_code);
             assert_ne!(
                 *err, predictable,
                 "regression: flash_loan returned precondition error {} before reaching the receiver",
                 regression_code
             );
         }
     }
+
+    // Post-state: atomicity. The failed flash loan must leave the pool
+    // reserves and the receiver/caller balances unchanged.
+    let pool_after = t.pool_reserves("USDC");
+    assert!(
+        (pool_before - pool_after).abs() < 0.001,
+        "failed flash loan must not move pool reserves: before={} after={}",
+        pool_before, pool_after
+    );
+    let bob_usdc = t.token_balance(BOB, "USDC");
+    assert!(
+        bob_usdc < 0.001,
+        "BOB must not retain pool funds after a failed flash loan, got {}",
+        bob_usdc
+    );
}
```

### `flash_loan_tests.rs::test_flash_loan_rejects_bad_repayment`

**Severity:** broken
**Rubric items failed:** [1, 3, 4]
**Why:** Uses `assert!(result.is_err(), ...)` (flash_loan_tests.rs:78-81) instead of pinning the contract error code (rubric 1). The protocol exposes `FlashLoanError::InvalidFlashLoanRepay = 402` (`errors::INVALID_FLASHLOAN_REPAY`, assert.rs:74) which is the exact code the bad receiver path should produce; if the helper returns a generic host error instead, that itself is a regression worth pinning. The test also performs no post-state checks confirming pool reserves were not drained (rubric 3) and no balance-delta verification (rubric 4) — both are central to a flash-loan atomicity guarantee.

**Patch (suggested):**
```diff
--- before
+++ after
@@ flash_loan_tests.rs:69-82 @@
 #[test]
 fn test_flash_loan_rejects_bad_repayment() {
     let mut t = LendingTest::new().with_market(usdc_preset()).build();

     t.supply(ALICE, "USDC", 100_000.0);

     let bad_receiver = t.deploy_bad_flash_loan_receiver();
+    let pool_before = t.pool_reserves("USDC");
+    let bob_before = t.token_balance(BOB, "USDC");
     let result = t.try_flash_loan(BOB, "USDC", 10_000.0, &bad_receiver);
-    // The bad receiver triggers a cross-contract failure that surfaces as
-    // a host error, not a specific contract error code.
-    assert!(
-        result.is_err(),
-        "flash loan should fail when receiver doesn't repay"
-    );
+    // Pin the precise contract error code: a non-repaying receiver must
+    // surface as INVALID_FLASHLOAN_REPAY (402), not any error.
+    assert_contract_error(result, errors::INVALID_FLASHLOAN_REPAY);
+
+    // Atomicity: pool reserves and BOB's wallet are unchanged.
+    let pool_after = t.pool_reserves("USDC");
+    let bob_after = t.token_balance(BOB, "USDC");
+    assert!(
+        (pool_before - pool_after).abs() < 0.001,
+        "pool reserves must not move after bad flash loan: before={} after={}",
+        pool_before, pool_after
+    );
+    assert!(
+        (bob_before - bob_after).abs() < 0.001,
+        "BOB wallet must not move after bad flash loan: before={} after={}",
+        bob_before, bob_after
+    );
}
```

> Note: if `INVALID_FLASHLOAN_REPAY` is not the actual code surfaced (because the SAC `transfer_from` panic surfaces as a host error before the controller's check runs), Phase 2 should refute this finding to `assert_contract_error` with the correct code. The current assertion is still too weak regardless.

### `flash_loan_tests.rs::test_flash_loan_rejects_disabled`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** Pins the error code correctly (flash_loan_tests.rs:101) so rubric 1 and 2 pass. But it never verifies the pool reserves and BOB's balance are unchanged — i.e. that the controller rejected the call before any token movement. The check that `FlashloanNotEnabled` truly fires *before* the SAC transfer is the property worth proving.

**Patch (suggested):**
```diff
--- before
+++ after
@@ flash_loan_tests.rs:89-102 @@
 #[test]
 fn test_flash_loan_rejects_disabled() {
     let mut t = LendingTest::new().with_market(usdc_preset()).build();

     t.supply(ALICE, "USDC", 100_000.0);

     // Disable flash loans for USDC.
     t.edit_asset_config("USDC", |cfg| {
         cfg.is_flashloanable = false;
     });

     let receiver = t.deploy_flash_loan_receiver();
+    let pool_before = t.pool_reserves("USDC");
+    let bob_before = t.token_balance(BOB, "USDC");
     let result = t.try_flash_loan(BOB, "USDC", 10_000.0, &receiver);
     assert_contract_error(result, errors::FLASHLOAN_NOT_ENABLED);
+
+    // No tokens moved -- guard fired before the receiver invocation.
+    assert!((t.pool_reserves("USDC") - pool_before).abs() < 0.001);
+    assert!((t.token_balance(BOB, "USDC") - bob_before).abs() < 0.001);
}
```

### `flash_loan_tests.rs::test_flash_loan_rejects_zero_amount`

**Severity:** weak
**Rubric items failed:** [3, 4]
**Why:** Pins `AMOUNT_MUST_BE_POSITIVE` correctly (flash_loan_tests.rs:117). But there are no balance-delta assertions; a zero-amount path that subtly debited fees would still pass. Add the cheap pool/wallet invariance check.

**Patch (suggested):**
```diff
--- before
+++ after
@@ flash_loan_tests.rs:109-118 @@
 #[test]
 fn test_flash_loan_rejects_zero_amount() {
     let mut t = LendingTest::new().with_market(usdc_preset()).build();

     t.supply(ALICE, "USDC", 100_000.0);

     let receiver = t.deploy_flash_loan_receiver();
+    let pool_before = t.pool_reserves("USDC");
+    let bob_before = t.token_balance(BOB, "USDC");
     let result = t.try_flash_loan(BOB, "USDC", 0.0, &receiver);
-    // Must reject with the precise AMOUNT_MUST_BE_POSITIVE (14).
     assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
+
+    // No state movement on a zero-amount rejection.
+    assert!((t.pool_reserves("USDC") - pool_before).abs() < 0.001);
+    assert!((t.token_balance(BOB, "USDC") - bob_before).abs() < 0.001);
}
```

### `flash_loan_tests.rs::test_flash_loan_reentrancy_blocks_supply`

**Severity:** none

### `flash_loan_tests.rs::test_flash_loan_reentrancy_blocks_borrow`

**Severity:** none

### `flash_loan_tests.rs::test_flash_loan_reentrancy_blocks_withdraw`

**Severity:** none

### `flash_loan_tests.rs::test_flash_loan_reentrancy_blocks_repay`

**Severity:** none

### `flash_loan_tests.rs::test_flash_loan_reentrancy_blocks_liquidation`

**Severity:** none

### `flash_loan_tests.rs::test_flash_loan_fee_config_matches_default_preset`

**Severity:** none

---

## footprint_test.rs

### `footprint_test.rs::measure_footprints`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Per the audit guidance, footprint tests' "post-state" is the recorded budget metrics. This test reads `env.cost_estimate().resources()` (footprint_test.rs:19-23) but never *asserts* any threshold — it only `println!`s the metrics. With Cargo capturing stdout by default, the test always passes regardless of whether a regression doubles `disk_read_entries` or pushes `write_bytes` over the 132 KiB limit. Add at least an upper-bound assertion per scenario so the test fails when a future change blows past the documented mainnet caps. The numbers below are conservative ceilings keyed off the existing print labels (entries=100, writes=50, read=200000, write=132096); pick exact values during Phase 2 by reading the current actuals once.

**Patch (suggested):**
```diff
--- before
+++ after
@@ footprint_test.rs:18-23 @@
-fn print_res(env: &soroban_sdk::Env, label: &str) {
+fn check_res(
+    env: &soroban_sdk::Env,
+    label: &str,
+    max_entries: u32,
+    max_writes: u32,
+    max_read_bytes: u32,
+    max_write_bytes: u32,
+) {
     let r = env.cost_estimate().resources();
     let total = r.disk_read_entries + r.memory_read_entries + r.write_entries;
     std::println!("  {:<45} entries={:>3}/100  writes={:>2}/50  read_bytes={:>6}/200000  write_bytes={:>5}/132096  events={:>5}/16384",
         label, total, r.write_entries, r.disk_read_bytes, r.write_bytes, r.contract_events_size_bytes);
+    // Hard ceilings so a regression in entries/writes/bytes fails the test.
+    assert!(total <= max_entries,
+        "{}: entries={} exceeds budget {}", label, total, max_entries);
+    assert!(r.write_entries <= max_writes,
+        "{}: writes={} exceeds budget {}", label, r.write_entries, max_writes);
+    assert!(r.disk_read_bytes <= max_read_bytes,
+        "{}: read_bytes={} exceeds budget {}", label, r.disk_read_bytes, max_read_bytes);
+    assert!(r.write_bytes <= max_write_bytes,
+        "{}: write_bytes={} exceeds budget {}", label, r.write_bytes, max_write_bytes);
 }
@@ footprint_test.rs (each call site) @@
-        print_res(&t.env, "Supply (1 market)");
+        // Mainnet caps: entries=100, writes=50, read=200000, write=132096.
+        check_res(&t.env, "Supply (1 market)", 100, 50, 200_000, 132_096);
@@
-        print_res(&t.env, "Borrow + HF check (2 markets)");
+        check_res(&t.env, "Borrow + HF check (2 markets)", 100, 50, 200_000, 132_096);
@@
-        print_res(&t.env, "Liquidation 1C+1D (2 markets)");
+        check_res(&t.env, "Liquidation 1C+1D (2 markets)", 100, 50, 200_000, 132_096);
@@
-        print_res(&t.env, "Liquidation 2C+1D (3 markets)");
+        check_res(&t.env, "Liquidation 2C+1D (3 markets)", 100, 50, 200_000, 132_096);
@@
-        print_res(&t.env, "Liquidation 2C+2D (4 markets)");
+        check_res(&t.env, "Liquidation 2C+2D (4 markets)", 100, 50, 200_000, 132_096);
```

> Phase 2/3 should run the test once and tighten the per-scenario bounds to ~110% of the observed values; the patch above only enforces the documented mainnet caps so that no scenario can silently exceed protocol limits.

---

## smoke_test.rs

### `smoke_test.rs::test_supply_creates_position`

**Severity:** none

### `smoke_test.rs::test_supply_and_borrow`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Smoke test exercises supply + borrow but does not check that any tokens actually moved. The post-state check uses `assert_position_exists` and `assert_borrow_near` (smoke_test.rs:50-55), which would pass even if the SAC transfer was a no-op (state and tokens decouple in fuzz/regression scenarios). Add a wallet-balance delta on at least one leg.

**Patch (suggested):**
```diff
--- before
+++ after
@@ smoke_test.rs:38-56 @@
 #[test]
 fn test_supply_and_borrow() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
         .build();

-    // Supply 10k USDC as collateral.
+    let usdc_before = t.token_balance(ALICE, "USDC");
+    let eth_before = t.token_balance(ALICE, "ETH");
+
     t.supply(ALICE, "USDC", 10_000.0);
-
-    // Borrow 1 ETH (~$2000, well within 75% LTV of $10k = $7500).
     t.borrow(ALICE, "ETH", 1.0);

     t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
     t.assert_position_exists(ALICE, "ETH", PositionType::Borrow);
     t.assert_healthy(ALICE);
-
-    // Verify the borrow balance is ~1 ETH.
     t.assert_borrow_near(ALICE, "ETH", 1.0, 0.01);
+
+    // Token deltas: 10k USDC out, 1 ETH in.
+    let usdc_after = t.token_balance(ALICE, "USDC");
+    let eth_after = t.token_balance(ALICE, "ETH");
+    assert!((usdc_before - usdc_after - 10_000.0).abs() < 0.01,
+        "wallet should be debited 10k USDC: {} -> {}", usdc_before, usdc_after);
+    assert!((eth_after - eth_before - 1.0).abs() < 0.01,
+        "wallet should be credited 1 ETH: {} -> {}", eth_before, eth_after);
}
```

### `smoke_test.rs::test_liquidation_after_price_drop`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Asserts only that the liquidator received *some* USDC (smoke_test.rs:87-92). Does not verify Alice's debt actually dropped, nor that her supply was reduced — both are core liquidation outcomes. A future bug that credits the liquidator without burning Alice's debt would still pass.

**Patch (suggested):**
```diff
--- before
+++ after
@@ smoke_test.rs:63-93 @@
 #[test]
 fn test_liquidation_after_price_drop() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
         .build();

-    // Alice supplies 10k USDC as collateral.
     t.supply(ALICE, "USDC", 10_000.0);
-
-    // Borrow 3 ETH (~$6000, near the 75% LTV limit of $7500).
     t.borrow(ALICE, "ETH", 3.0);
     t.assert_healthy(ALICE);

-    // Drop USDC price to $0.50: collateral now worth $5000.
-    // liquidation_threshold = 80% => weighted collateral = $4000.
-    // debt = $6000 => HF = 4000/6000 ~ 0.67 => liquidatable.
     t.set_price("USDC", usd_cents(50));

     t.assert_liquidatable(ALICE);

-    // The liquidator repays part of Alice's ETH debt.
+    let alice_debt_before = t.borrow_balance(ALICE, "ETH");
+    let alice_supply_before = t.supply_balance(ALICE, "USDC");
+
     t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

-    // The liquidator must have received USDC collateral.
+    // Liquidator received collateral.
     let liq_usdc_after = t.token_balance(LIQUIDATOR, "USDC");
     assert!(
         liq_usdc_after > 0.0,
         "liquidator should have received collateral, got {}",
         liq_usdc_after
     );
+
+    // Alice's debt must have decreased and supply must have been seized.
+    let alice_debt_after = t.borrow_balance(ALICE, "ETH");
+    let alice_supply_after = t.supply_balance(ALICE, "USDC");
+    assert!(alice_debt_after < alice_debt_before,
+        "Alice debt should drop: {} -> {}", alice_debt_before, alice_debt_after);
+    assert!(alice_supply_after < alice_supply_before,
+        "Alice supply should be seized: {} -> {}", alice_supply_before, alice_supply_after);
}
```

### `smoke_test.rs::test_interest_accrues`

**Severity:** none

### `smoke_test.rs::test_withdraw_and_repay`

**Severity:** none

### `smoke_test.rs::test_emode_higher_ltv`

**Severity:** none

### `smoke_test.rs::test_revenue_snapshot`

**Severity:** nit
**Rubric items failed:** [5]
**Why:** Name reads as a noun — "snapshot revenue" — but the test's actual scenario is "revenue grows as time advances under an active borrow". A more descriptive name (e.g., `test_revenue_grows_with_time_under_borrow`) would make the intent clear at a glance. Verified no collision: `grep -r test_revenue_grows .` returns nothing in the repo.

**Patch (suggested):**
```diff
--- before
+++ after
@@ smoke_test.rs:210-211 @@
 #[test]
-fn test_revenue_snapshot() {
+fn test_revenue_grows_with_time_under_borrow() {
```

---

## Cross-cutting patterns

The events suite uniformly relies on `t.env.events().all().events().len() > 0` (or `>= N`) as its sole assertion, which proves only that *some* event fired during the last invocation — it does not bind the topic, asset, or actor and therefore cannot detect a regression that emits a wrong event or stops emitting the right one. Every event test should additionally assert (a) the canonical post-state for the operation (`assert_supply_near`, `assert_borrow_near`, debt deltas) and (b) at least one topic/payload string match using the existing `format!("{:#?}", t.env.events().all())` idiom already present in `test_supply_position_event_restores_risk_fields`. The flash-loan suite does the opposite: most reentrancy reject paths pin the exact error code, but the two non-reentrancy reject paths (`test_flash_loan_rejects_bad_repayment`, the `mock_auth_limitation_documented` test) skip both code-pinning and atomicity (pool/wallet delta) checks, which are the most important properties of a flash-loan failure path. The footprint test prints metrics but never asserts on them, so any future regression past the documented mainnet caps would silently slip through. The smoke suite is mostly fine but two scenarios (supply-and-borrow, liquidation-after-price-drop) lack the wallet/debt deltas that prove tokens actually moved.
