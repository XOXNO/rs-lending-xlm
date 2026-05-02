# Domain 4 — Strategy

**Phase:** 1
**Files in scope:**
- `test-harness/tests/strategy_tests.rs`
- `test-harness/tests/strategy_bad_router_tests.rs`
- `test-harness/tests/strategy_coverage_tests.rs`
- `test-harness/tests/strategy_edge_tests.rs`
- `test-harness/tests/strategy_happy_tests.rs`
- `test-harness/tests/strategy_panic_coverage_tests.rs`

**Totals:** broken=1 weak=11 nit=0

---

## `strategy_tests.rs`

### `strategy_tests.rs::test_multiply_rejects_non_borrowable_debt`

**Severity:** none

### `strategy_tests.rs::test_multiply_rejects_non_collateralizable`

**Severity:** none

### `strategy_tests.rs::test_multiply_rejects_during_flash_loan`

**Severity:** none

### `strategy_tests.rs::test_swap_collateral_rejects_isolated`

**Severity:** none

### `strategy_tests.rs::test_multiply_rejects_isolated_debt_ceiling_breach`

**Severity:** none

---

## `strategy_bad_router_tests.rs`

### `strategy_bad_router_tests.rs::test_swap_tokens_panics_when_router_refunds_token_in`

**Severity:** none

### `strategy_bad_router_tests.rs::test_swap_tokens_rejects_router_pulling_more_than_allowance`

**Severity:** broken
**Rubric items failed:** [1]
**Why:** Lines 123-127 only assert `result.is_err()`, accepting any error including upstream regressions (e.g., a future bug that rejects the multiply at validation before the bad router is even invoked would still satisfy `is_err()`). The OverPull case panics inside the token contract (host-level token allowance error, not a controller contract error code), so `assert_contract_error` cannot be used directly. Tighten by asserting the error is the host-level `Error(Auth, InvalidAction)` produced by the token's `transfer_from` allowance check, which proves the controller pre-approved exactly `amount_in` and the bad router's overshoot was rejected at the right layer.

**Patch (suggested):**
```diff
--- a/test-harness/tests/strategy_bad_router_tests.rs
+++ b/test-harness/tests/strategy_bad_router_tests.rs
@@ -119,12 +119,21 @@ fn test_swap_tokens_rejects_router_pulling_more_than_allowance() {
         &steps,
     );

-    // The transfer_from for 2x amount_in fails inside the token contract.
-    // Any concrete contract error is acceptable evidence that the controller
-    // did not pre-approve more than requested; !is_ok is enough.
-    assert!(
-        result.is_err(),
-        "bad router should have been blocked by the token allowance, got Ok({:?})",
-        result
+    // The transfer_from for 2x amount_in must fail inside the token contract
+    // because the controller approves exactly `amount_in`. Pin the host-level
+    // auth error so a regression that rejects multiply earlier (e.g. inside
+    // validation) -- still an `is_err()` -- does not silently pass.
+    let err = result.expect_err("bad router should have been blocked by the token allowance");
+    let expected = soroban_sdk::Error::from_type_and_code(
+        soroban_sdk::xdr::ScErrorType::Auth,
+        soroban_sdk::xdr::ScErrorCode::InvalidAction,
+    );
+    assert_eq!(
+        err, expected,
+        "expected token-contract allowance rejection (Auth/InvalidAction), got {:?}",
+        err
     );
 }
```

### `strategy_bad_router_tests.rs::test_swap_tokens_handles_zero_output_from_router`

**Severity:** none

---

## `strategy_coverage_tests.rs`

### `strategy_coverage_tests.rs::test_strategy_swap_collateral_supply_cap_reached`

**Severity:** none

### `strategy_coverage_tests.rs::test_strategy_multiply_supply_cap_reached`

**Severity:** none

### `strategy_coverage_tests.rs::test_strategy_multiply_unsupported_category`

**Severity:** none

---

## `strategy_edge_tests.rs`

### `strategy_edge_tests.rs::test_multiply_with_debt_token_initial_payment`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Alice's wallet is pre-minted 0.5 ETH at line 100 and then `Some((eth, 5_000000))` is passed as `initial_payment` (line 114). The ETH initial payment must be transferred from Alice's wallet into the strategy by `multiply`. The test asserts the supply (4499..=4501) and borrow (~1) post-state, but never asserts the wallet decrement. A regression that supplied the initial payment from somewhere else (e.g. controller's own balance) would still produce the expected supply/borrow but leak ETH from the controller. Add a balance delta assertion on Alice's ETH wallet.

**Patch (suggested):**
```diff
--- a/test-harness/tests/strategy_edge_tests.rs
+++ b/test-harness/tests/strategy_edge_tests.rs
@@ -98,6 +98,7 @@ fn test_multiply_with_debt_token_initial_payment() {
     let eth_market = t.resolve_market("ETH");
     eth_market.token_admin.mint(&alice, &5_000000i128); // 0.5 ETH

+    let alice_eth_before = t.token_balance(ALICE, "ETH");
     t.fund_router("USDC", 4_500.0);
     let steps = build_swap_steps(&t, "ETH", "USDC", 4500_0000000);

@@ -125,6 +126,12 @@ fn test_multiply_with_debt_token_initial_payment() {
     assert!(
         (0.99..=1.01).contains(&borrow),
         "borrowed ETH should remain the strategy debt amount only, got {}",
         borrow
     );
+    let alice_eth_after = t.token_balance(ALICE, "ETH");
+    assert!(
+        (alice_eth_before - alice_eth_after - 0.5).abs() < 0.001,
+        "Alice's ETH wallet must shrink by the 0.5 ETH initial payment: before={}, after={}",
+        alice_eth_before,
+        alice_eth_after
+    );
 }
```

### `strategy_edge_tests.rs::test_multiply_rejects_when_paused`

**Severity:** none

### `strategy_edge_tests.rs::test_multiply_borrow_cap_would_exceed`

**Severity:** none

### `strategy_edge_tests.rs::test_multiply_preserves_existing_collateral_balance`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Lines 232-238 assert only `final_supply > 3_500.0`. The test also opens a borrow leg via multiply (1 ETH debt), but never asserts the ETH borrow exists or that the account stays healthy. The test passes for any final supply above 3500, which is satisfied even if the swap output were halved. Tighten by checking the borrow position and HF.

**Patch (suggested):**
```diff
--- a/test-harness/tests/strategy_edge_tests.rs
+++ b/test-harness/tests/strategy_edge_tests.rs
@@ -230,11 +230,21 @@ fn test_multiply_preserves_existing_collateral_balance() {
     assert!(matches!(result, Ok(Ok(_))), "multiply should succeed");

     let final_supply = t.supply_balance_for(ALICE, account_id, "USDC");
     assert!(
         final_supply > 3_500.0,
         "existing collateral must be preserved and increased, got {}",
         final_supply
     );
+    let final_borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
+    assert!(
+        (0.99..=1.01).contains(&final_borrow),
+        "ETH borrow must be the strategy debt amount, got {}",
+        final_borrow
+    );
+    assert!(
+        t.health_factor_for(ALICE, account_id) >= 1.0,
+        "account must remain healthy after multiply on existing collateral"
+    );
 }
```

### `strategy_edge_tests.rs::test_multiply_emode_wrong_category_debt`

**Severity:** none

### `strategy_edge_tests.rs::test_multiply_emode_wrong_category_collateral`

**Severity:** none

### `strategy_edge_tests.rs::test_multiply_isolated_debt_not_enabled`

**Severity:** none

### `strategy_edge_tests.rs::test_multiply_rejects_isolated_collateral_on_existing_non_isolated_account`

**Severity:** none

### `strategy_edge_tests.rs::test_multiply_rejects_mode_4`

**Severity:** none

### `strategy_edge_tests.rs::test_multiply_rejects_new_collateral_when_supply_limit_reached`

**Severity:** none

### `strategy_edge_tests.rs::test_multiply_existing_account_wrong_owner`

**Severity:** none

### `strategy_edge_tests.rs::test_multiply_rejects_supply_cap_after_deposit`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_debt_refund_only_uses_strategy_excess`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_debt_health_factor_guard_after_swap`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_debt_rejects_when_paused`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_debt_rejects_during_flash_loan`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_collateral_applies_emode_params_to_destination_position`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_debt_non_borrowable_new_debt`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_debt_siloed_conflict`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_debt_existing_siloed_borrow_blocks_new`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_debt_isolated_not_borrowable`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_debt_borrow_cap_new_debt`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_debt_emode_wrong_category`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_collateral_rejects_when_paused`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_collateral_rejects_during_flash_loan`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_collateral_non_collateralizable`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_collateral_to_isolated_asset`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_collateral_rejects_supply_cap_after_deposit`

**Severity:** none

### `strategy_edge_tests.rs::test_repay_debt_with_collateral_same_token_nets_positions`

**Severity:** none

### `strategy_edge_tests.rs::test_repay_debt_with_collateral_refund_only_uses_repay_excess`

**Severity:** none

### `strategy_edge_tests.rs::test_repay_debt_with_collateral_health_factor_guard`

**Severity:** none

### `strategy_edge_tests.rs::test_repay_debt_with_collateral_close_position_removes_account`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Lines 1136-1140 assert only that the account no longer exists. The test does not verify that the ETH debt was zeroed (close_position is supposed to repay all debt), nor that any leftover collateral was returned to Alice's wallet (close drains and refunds the residual). A regression that removed the account but left an orphan debt position keyed by the old account_id, or that swept the residual collateral to the controller, would still pass. Add `borrow_balance == 0` and a sanity check that Alice's USDC wallet did not increase impossibly (no collateral was swept).

**Patch (suggested):**
```diff
--- a/test-harness/tests/strategy_edge_tests.rs
+++ b/test-harness/tests/strategy_edge_tests.rs
@@ -1129,12 +1129,22 @@ fn test_repay_debt_with_collateral_close_position_removes_account() {

     t.fund_router("ETH", 1.0);
     let steps = build_swap_steps(&t, "USDC", "ETH", 1_0000000);
+    let alice_usdc_before = t.token_balance(ALICE, "USDC");
     t.repay_debt_with_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps, true);

     assert!(
         !t.account_exists(account_id),
         "close_position should remove the fully closed account"
     );
+    assert_eq!(
+        t.borrow_balance_raw(ALICE, "ETH"),
+        0,
+        "close_position must zero the ETH debt"
+    );
+    let alice_usdc_after = t.token_balance(ALICE, "USDC");
+    assert!(
+        alice_usdc_after >= alice_usdc_before,
+        "residual USDC collateral must be refunded (or unchanged), not swept: before={}, after={}",
+        alice_usdc_before,
+        alice_usdc_after
+    );
 }
```

### `strategy_edge_tests.rs::test_repay_debt_with_collateral_removes_empty_account_without_close`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_collateral_rejects_new_asset_when_supply_limit_reached`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_collateral_emode_wrong_category`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_collateral_no_borrows_skip_hf`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Lines 1247-1259 verify success and that ETH supply > 0, but never check that USDC supply shrank. A regression where swap_collateral mints fresh ETH supply without withdrawing the USDC counterpart (double-counting collateral) would pass. Add an assertion that USDC supply decreased.

**Patch (suggested):**
```diff
--- a/test-harness/tests/strategy_edge_tests.rs
+++ b/test-harness/tests/strategy_edge_tests.rs
@@ -1240,6 +1240,7 @@ fn test_swap_collateral_no_borrows_skip_hf() {

     // Supply only, no borrows.
     t.supply(ALICE, "USDC", 100_000.0);
+    let usdc_before = t.supply_balance(ALICE, "USDC");

     // Swap collateral: the HF check is skipped (no borrows). With the
     // working mock router, this succeeds.
@@ -1255,6 +1256,12 @@ fn test_swap_collateral_no_borrows_skip_hf() {
         eth_supply > 0.0,
         "should have ETH supply: got {}",
         eth_supply
     );
+    let usdc_after = t.supply_balance(ALICE, "USDC");
+    assert!(
+        usdc_after < usdc_before,
+        "USDC supply must shrink to fund the ETH leg: before={}, after={}",
+        usdc_before,
+        usdc_after
+    );
 }
```

### `strategy_edge_tests.rs::test_strategy_empty_swap_steps_multiply`

**Severity:** none

### `strategy_edge_tests.rs::test_multiply_zero_debt_amount`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_debt_zero_amount`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_collateral_zero_amount`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_debt_wrong_account_owner`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_collateral_wrong_account_owner`

**Severity:** none

### `strategy_edge_tests.rs::test_multiply_same_asset_is_caught`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_debt_same_token_error_code`

**Severity:** none

### `strategy_edge_tests.rs::test_swap_collateral_same_token_error_code`

**Severity:** none

---

## `strategy_happy_tests.rs`

### `strategy_happy_tests.rs::test_multiply_creates_leveraged_position`

**Severity:** none

### `strategy_happy_tests.rs::test_multiply_mode_long`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Lines 116-126 assert only the stored `mode` and `HF >= 1.0`. The test claims to verify "same flow" as `test_multiply_creates_leveraged_position` but does not assert the supply (~3000 USDC) or borrow (~1 ETH) magnitudes. A regression where Long mode skipped the deposit would still satisfy `mode == Long` and `HF >= 1.0` (an empty position has `HF = i128::MAX / WAD`). Add the supply/borrow magnitude assertions.

**Patch (suggested):**
```diff
--- a/test-harness/tests/strategy_happy_tests.rs
+++ b/test-harness/tests/strategy_happy_tests.rs
@@ -118,6 +118,16 @@ fn test_multiply_mode_long() {
     assert_eq!(
         attrs.mode,
         common::types::PositionMode::Long,
         "mode should be Long"
     );

+    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
+    assert!(
+        (2999.0..=3001.0).contains(&supply),
+        "USDC supply should be ~3000, got {}",
+        supply
+    );
+    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
+    assert!(
+        (0.99..=1.01).contains(&borrow),
+        "ETH borrow should be ~1.0, got {}",
+        borrow
+    );
+
     let hf = t.health_factor_for(ALICE, account_id);
     assert!(hf >= 1.0, "HF should be >= 1.0, got {}", hf);
 }
```

### `strategy_happy_tests.rs::test_multiply_mode_short`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Same gap as `test_multiply_mode_long`: lines 152-162 assert only mode and `HF >= 1.0`. Supply and borrow magnitudes are not pinned. Add them.

**Patch (suggested):**
```diff
--- a/test-harness/tests/strategy_happy_tests.rs
+++ b/test-harness/tests/strategy_happy_tests.rs
@@ -154,6 +154,16 @@ fn test_multiply_mode_short() {
     assert_eq!(
         attrs.mode,
         common::types::PositionMode::Short,
         "mode should be Short"
     );

+    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
+    assert!(
+        (2999.0..=3001.0).contains(&supply),
+        "USDC supply should be ~3000, got {}",
+        supply
+    );
+    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
+    assert!(
+        (0.99..=1.01).contains(&borrow),
+        "ETH borrow should be ~1.0, got {}",
+        borrow
+    );
+
     let hf = t.health_factor_for(ALICE, account_id);
     assert!(hf >= 1.0, "HF should be >= 1.0, got {}", hf);
 }
```

### `strategy_happy_tests.rs::test_multiply_wbtc_collateral`

**Severity:** none

### `strategy_happy_tests.rs::test_swap_debt_replaces_borrow`

**Severity:** none

### `strategy_happy_tests.rs::test_swap_debt_partial`

**Severity:** none

### `strategy_happy_tests.rs::test_swap_collateral_replaces_supply`

**Severity:** none

### `strategy_happy_tests.rs::test_swap_collateral_no_borrows`

**Severity:** none

### `strategy_happy_tests.rs::test_repay_debt_with_collateral_reduces_positions`

**Severity:** none

### `strategy_happy_tests.rs::test_multiply_emode_stablecoin`

**Severity:** none

### `strategy_happy_tests.rs::test_multiply_large_amounts`

**Severity:** none

### `strategy_happy_tests.rs::test_multiply_two_users`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** Lines 574-585 assert only `alice_id != bob_id` and that both HFs are `>= 1.0`. The test does not verify that each user actually owns their own account or that the supply/borrow magnitudes were created independently (Alice ~3000 USDC supply / 1 ETH borrow vs. Bob ~6000 USDC supply / 2 ETH borrow). A regression that double-routed Bob's swap output into Alice's account would still satisfy the existing assertions.

**Patch (suggested):**
```diff
--- a/test-harness/tests/strategy_happy_tests.rs
+++ b/test-harness/tests/strategy_happy_tests.rs
@@ -572,6 +572,21 @@ fn test_multiply_two_users() {

     assert_ne!(alice_id, bob_id, "accounts should be different");

+    let alice_supply = t.supply_balance_for(ALICE, alice_id, "USDC");
+    let bob_supply = t.supply_balance_for(BOB, bob_id, "USDC");
+    assert!(
+        (2999.0..=3001.0).contains(&alice_supply),
+        "Alice should have ~3000 USDC, got {}",
+        alice_supply
+    );
+    assert!(
+        (5999.0..=6001.0).contains(&bob_supply),
+        "Bob should have ~6000 USDC, got {}",
+        bob_supply
+    );
+    assert_eq!(t.get_account_owner(alice_id), t.users.get(ALICE).unwrap().address);
+    assert_eq!(t.get_account_owner(bob_id), t.users.get(BOB).unwrap().address);
+
     let alice_hf = t.health_factor_for(ALICE, alice_id);
     let bob_hf = t.health_factor_for(BOB, bob_id);
```

### `strategy_happy_tests.rs::test_swap_debt_hf_improvement`

**Severity:** weak
**Rubric items failed:** [3]
**Why:** The test name implies the HF must improve, but lines 616-621 assert only `hf_after >= 1.0`. The pre-swap HF (`hf_before`) is captured at line 607 yet never compared to `hf_after`. A regression where the swap left HF unchanged or worsened (but still above 1.0) would pass. Either rename the test to drop the "improvement" claim or assert `hf_after > hf_before`.

**Patch (suggested):**
```diff
--- a/test-harness/tests/strategy_happy_tests.rs
+++ b/test-harness/tests/strategy_happy_tests.rs
@@ -614,9 +614,14 @@ fn test_swap_debt_hf_improvement() {

     let hf_after = t.health_factor(ALICE);
     assert!(
         hf_after >= 1.0,
         "HF should still be >= 1.0 after swap_debt, got {}",
         hf_after
     );
+    assert!(
+        hf_after > hf_before,
+        "swap to cheaper debt must improve HF, before={}, after={}",
+        hf_before,
+        hf_after
+    );
 }
```

---

## `strategy_panic_coverage_tests.rs`

### `strategy_panic_coverage_tests.rs::test_multiply_third_token_payment_without_convert_steps_rejects`

**Severity:** none

### `strategy_panic_coverage_tests.rs::test_multiply_existing_account_mode_mismatch_rejects`

**Severity:** none

### `strategy_panic_coverage_tests.rs::test_swap_debt_existing_position_missing_rejects`

**Severity:** none

### `strategy_panic_coverage_tests.rs::test_swap_collateral_position_missing_rejects`

**Severity:** none

### `strategy_panic_coverage_tests.rs::test_repay_debt_with_collateral_missing_collateral_rejects`

**Severity:** none

### `strategy_panic_coverage_tests.rs::test_repay_debt_with_collateral_missing_debt_rejects`

**Severity:** none

### `strategy_panic_coverage_tests.rs::test_repay_debt_with_collateral_close_with_remaining_debt_rejects`

**Severity:** none

### `strategy_panic_coverage_tests.rs::test_multiply_with_collateral_token_initial_payment`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Alice is minted 500 USDC at line 304 and supplies a `Some((usdc, 500_0000000))` initial payment (line 319). The 500 USDC must move out of Alice's wallet into the strategy, but the test never asserts the wallet decrement. A regression where the initial-payment branch was satisfied from a different source (controller balance, donation pool, etc.) would still produce supply ~3500 and borrow ~1 ETH. Add a balance delta on Alice's USDC wallet.

**Patch (suggested):**
```diff
--- a/test-harness/tests/strategy_panic_coverage_tests.rs
+++ b/test-harness/tests/strategy_panic_coverage_tests.rs
@@ -302,6 +302,7 @@ fn test_multiply_with_collateral_token_initial_payment() {
     // Mint 500 USDC to Alice so she can pay in with the same token she will
     // use as collateral.
     usdc_market.token_admin.mint(&alice, &500_0000000i128);
+    let alice_usdc_before = t.token_balance(ALICE, "USDC");

     t.fund_router("USDC", 3_000.0);
     let steps = build_swap_steps(&t, "ETH", "USDC", 30_000_000_000);
@@ -337,6 +338,13 @@ fn test_multiply_with_collateral_token_initial_payment() {
     assert!(
         (0.99..=1.01).contains(&borrow),
         "borrow must be only the flash debt: got {}",
         borrow
     );
+    let alice_usdc_after = t.token_balance(ALICE, "USDC");
+    assert!(
+        (alice_usdc_before - alice_usdc_after - 500.0).abs() < 0.001,
+        "Alice's USDC wallet must decrement by the 500 USDC initial payment: before={}, after={}",
+        alice_usdc_before,
+        alice_usdc_after
+    );
 }
```

### `strategy_panic_coverage_tests.rs::test_multiply_with_third_token_initial_payment_swaps_via_convert_steps`

**Severity:** weak
**Rubric items failed:** [4]
**Why:** Alice is minted 0.1 WBTC at line 362 and pays it in via the third-token branch with `convert_steps`. The test asserts the resulting USDC supply (~3500), but never verifies Alice's WBTC wallet was decremented by 0.1 (the entire initial payment). A regression that consumed only a fraction of the WBTC payment (or none, sourcing from elsewhere) could still leave the supply close to 3500 if the convert_steps amount happened to align. Add a WBTC balance delta on Alice.

**Patch (suggested):**
```diff
--- a/test-harness/tests/strategy_panic_coverage_tests.rs
+++ b/test-harness/tests/strategy_panic_coverage_tests.rs
@@ -360,6 +360,7 @@ fn test_multiply_with_third_token_initial_payment_swaps_via_convert_steps() {

     // Alice pays in with WBTC. Mint some to her.
     wbtc_market.token_admin.mint(&alice, &10_000_000i128); // 0.1 WBTC
+    let alice_wbtc_before = t.token_balance(ALICE, "WBTC");

     // Main debt swap (ETH -> USDC) and initial-payment convert (WBTC ->
     // USDC). The mock aggregator funds each side independently, so fund
@@ -388,6 +389,13 @@ fn test_multiply_with_third_token_initial_payment_swaps_via_convert_steps() {
         (3_499.0..=3_501.0).contains(&supply),
         "third-token payment must be converted and added to collateral: got {}",
         supply
     );
+    let alice_wbtc_after = t.token_balance(ALICE, "WBTC");
+    assert!(
+        (alice_wbtc_before - alice_wbtc_after - 0.1).abs() < 0.0001,
+        "Alice's WBTC wallet must decrement by the 0.1 WBTC initial payment: before={}, after={}",
+        alice_wbtc_before,
+        alice_wbtc_after
+    );
 }
```

### `strategy_panic_coverage_tests.rs::test_swap_tokens_allowance_remains_zero_after_overpull_rejection`

**Severity:** weak
**Rubric items failed:** [1]
**Why:** Line 433 only asserts `result.is_err()`. The OverPull rollback test specifically targets the controller's pre-approval safety: a future regression that rejects multiply earlier (e.g. a bogus validation panic) would still satisfy `is_err()`, hide the underlying bug, and let the allowance happen to be zero because nothing was approved. Pin the host-level token allowance error (same `Auth/InvalidAction` as the bad-router suite) so a regression that changes the rejection layer is loud.

**Patch (suggested):**
```diff
--- a/test-harness/tests/strategy_panic_coverage_tests.rs
+++ b/test-harness/tests/strategy_panic_coverage_tests.rs
@@ -429,7 +429,15 @@ fn test_swap_tokens_allowance_remains_zero_after_overpull_rejection() {
         common::types::PositionMode::Multiply,
         &steps,
     );
-    assert!(result.is_err(), "OverPull must be rejected");
+    let err = result.expect_err("OverPull must be rejected");
+    let expected = soroban_sdk::Error::from_type_and_code(
+        soroban_sdk::xdr::ScErrorType::Auth,
+        soroban_sdk::xdr::ScErrorCode::InvalidAction,
+    );
+    assert_eq!(
+        err, expected,
+        "expected token allowance rejection (Auth/InvalidAction) so the rollback is the right one, got {:?}",
+        err
+    );

     // After rollback, the controller's ETH allowance on the bad router must
     // be zero. A regression that leaks the pre-approved allowance would
```

### `strategy_panic_coverage_tests.rs::test_swap_tokens_allowance_zero_after_successful_multiply`

**Severity:** none

### `strategy_panic_coverage_tests.rs::test_multiply_reusing_account_wrong_owner_rejects`

**Severity:** none

---

## Cross-cutting patterns

The strategy suite is in strong shape overall: 69 of 81 tests pass all five rubric items and use `assert_contract_error` with the precise contract code. The recurring weaknesses fall into three families. (1) Initial-payment success paths (`test_multiply_with_debt_token_initial_payment`, `test_multiply_with_collateral_token_initial_payment`, `test_multiply_with_third_token_initial_payment_swaps_via_convert_steps`) consistently skip the caller-wallet delta — they verify the resulting supply/borrow magnitudes but never confirm the funds actually came from the user, leaving a gap where a regression that sourced the payment from elsewhere would pass. (2) The `multiply_mode_*` and `multiply_two_users` happy paths assert mode/HF only, omitting the supply and borrow magnitudes that distinguish a real strategy from an empty position. (3) Two adversarial tests that hinge on a host-level token error (`test_swap_tokens_rejects_router_pulling_more_than_allowance` and `test_swap_tokens_allowance_remains_zero_after_overpull_rejection`) use bare `is_err()` because `assert_contract_error` only handles controller errors; they should pin `Auth/InvalidAction` from the token contract so that a future regression which rejects the multiply at a different (earlier) layer is caught. The remaining `test_repay_debt_with_collateral_close_position_removes_account` and `test_swap_debt_hf_improvement` are minor — the former under-asserts the post-close debt zeroing, the latter advertises an HF improvement it never compares.
