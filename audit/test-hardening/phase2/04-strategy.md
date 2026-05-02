# Domain 4 — Strategy (Phase 2 review)

**Phase:** 2 (independent review)
**Files re-read:**
- `test-harness/tests/strategy_tests.rs`
- `test-harness/tests/strategy_bad_router_tests.rs`
- `test-harness/tests/strategy_coverage_tests.rs`
- `test-harness/tests/strategy_edge_tests.rs`
- `test-harness/tests/strategy_happy_tests.rs`
- `test-harness/tests/strategy_panic_coverage_tests.rs`
- `test-harness/src/{user,view,strategy,context,mock_aggregator}.rs`
- `controller/src/strategy.rs`

**Totals:** confirmed=8 refuted=0 refined=4 new=0

---

## Phase 1 entries

### 1. `strategy_bad_router_tests.rs::test_swap_tokens_rejects_router_pulling_more_than_allowance` — Phase 1: broken

**Disposition:** refined

**Reviewer note:** The auditor's diagnosis of the gap (a bare `is_err()` accepts upstream regressions) is correct, but the proposed patch is **factually wrong** about which error is produced. I ran an instrumented copy of the test with the actual `BadAggregator`/OverPull setup and the captured error is:

```
Err(Error(Contract, #9))
```

That is a SAC `transfer_from` insufficient-allowance failure surfaced as a **contract error** (the SAC's own error code, propagated as a `Contract` error), **not** the host-level `Error(Auth, InvalidAction)` the auditor suggests. The auditor's patch would change the test from "passes for any error" to "fails for the actual error", flipping the test from too-loose to broken. The right tightening is to assert the contract error via the harness's existing `assert_contract_error` helper (which already understands the `Ok(Err(...))` shape returned by `try_multiply`).

**Refined patch:**

```diff
--- a/test-harness/tests/strategy_bad_router_tests.rs
+++ b/test-harness/tests/strategy_bad_router_tests.rs
@@ -14,7 +14,7 @@ extern crate std;
 use common::types::{DexDistribution, Protocol, SwapSteps};
 use soroban_sdk::{vec, Address};
 use test_harness::mock_aggregator::{BadAggregator, BadMode};
-use test_harness::{assert_contract_error, errors, eth_preset, usdc_preset, LendingTest, ALICE};
+use test_harness::{assert_contract_error, errors, eth_preset, usdc_preset, LendingTest, ALICE};

 // ---------------------------------------------------------------------------
 // Helpers
@@ -117,13 +117,16 @@ fn test_swap_tokens_rejects_router_pulling_more_than_allowance() {
         &steps,
     );

-    // The transfer_from for 2x amount_in fails inside the token contract.
-    // Any concrete contract error is acceptable evidence that the controller
-    // did not pre-approve more than requested; !is_ok is enough.
-    assert!(
-        result.is_err(),
-        "bad router should have been blocked by the token allowance, got Ok({:?})",
-        result
+    // The SAC's transfer_from rejects the 2x pull with its insufficient-
+    // allowance error (Error(Contract, #9)). Pinning the exact code makes
+    // sure a regression that rejects multiply at an *earlier* layer (e.g. a
+    // bogus validation panic that surfaces a different contract error)
+    // does not silently keep the test green. The error is propagated as a
+    // contract error by the SAC, not as a host-level Auth/InvalidAction.
+    assert_contract_error::<u64>(
+        match result { Ok(Ok(id)) => Ok(id), Ok(Err(e)) => Err(e), Err(e) => Err(e) },
+        9,
     );
 }
```

(Implementation note: `try_multiply` returns `Result<u64, soroban_sdk::Error>` already collapsed by the harness in `test-harness/src/strategy.rs:131-132`, so the `match` reshape above is redundant; a plain `assert_contract_error(result, 9)` works because `result` is already that shape. The `Result<Result<...>, ...>` reshape is shown only to mirror the pattern used by the analogous test in `strategy_tests.rs::test_multiply_rejects_isolated_debt_ceiling_breach` lines 239-246, where the raw `ctrl.try_multiply` is invoked. For this test, the simpler form `assert_contract_error(result, 9)` is sufficient.)

---

### 2. `strategy_edge_tests.rs::test_multiply_with_debt_token_initial_payment` — Phase 1: weak

**Disposition:** confirmed

The 0.5 ETH initial payment is minted to Alice (line 100), passed via `Some((eth, 5_000000))`, and the test asserts the resulting supply (~4500) and borrow (~1.0) but never the wallet decrement. Adding the `token_balance` delta is the right tightening. The auditor's patch is sound and uses existing harness methods (`token_balance` exists at `test-harness/src/view.rs:42`).

---

### 3. `strategy_edge_tests.rs::test_multiply_preserves_existing_collateral_balance` — Phase 1: weak

**Disposition:** confirmed

Lines 234-238 check only `final_supply > 3_500.0`. The multiply opens a new ETH borrow leg that the test never confirms exists. The auditor's patch adds the borrow magnitude and HF check — correct.

---

### 4. `strategy_edge_tests.rs::test_repay_debt_with_collateral_close_position_removes_account` — Phase 1: weak

**Disposition:** confirmed

The test asserts only `!t.account_exists(account_id)` (line 1138). It never confirms (a) the debt was actually repaid (close_position semantics) or (b) the residual collateral was returned to Alice's wallet rather than swept. The auditor's patch adds both checks. The patch's `>= alice_usdc_before` (rather than strict `>`) is fine because the residual after a 1000 USDC partial-collateral repay against a 1 ETH debt with a 1:1 ETH→ETH mock-swap ratio is non-trivial; either way, the bound catches the "swept to controller" regression (which would yield `usdc_after < usdc_before`).

---

### 5. `strategy_edge_tests.rs::test_swap_collateral_no_borrows_skip_hf` — Phase 1: weak

**Disposition:** confirmed

Lines 1247-1259 assert `eth_supply > 0.0` but never that USDC supply shrank. The auditor's patch adds the USDC delta — correct.

---

### 6. `strategy_happy_tests.rs::test_multiply_mode_long` — Phase 1: weak

**Disposition:** confirmed

Lines 117-125 assert only mode and `HF >= 1.0`. The auditor correctly notes that an empty position trivially satisfies `HF >= 1.0` (the controller returns `i128::MAX` scaled), so a regression that skipped the deposit branch in Long mode would pass. Patch adds the supply (~3000) and borrow (~1.0) magnitude checks — correct.

---

### 7. `strategy_happy_tests.rs::test_multiply_mode_short` — Phase 1: weak

**Disposition:** confirmed

Mirror of #6 for Short mode (lines 153-162). Same gap, same fix. Confirmed.

---

### 8. `strategy_happy_tests.rs::test_multiply_two_users` — Phase 1: weak

**Disposition:** refined

The auditor's diagnosis (only `alice_id != bob_id` and HF >= 1.0 are asserted; magnitudes/ownership are not) is correct. The patch is mostly right but uses `t.users.get(ALICE).unwrap().address` directly. While `users` is `pub` on the harness (`test-harness/src/context.rs:87`) and `UserState.address` is `pub` (`context.rs:20`), there is a cleaner public accessor: `t.get_or_create_user(ALICE)` returns the address. The refined patch swaps the direct field access for the documented helper.

Also worth pinning: Bob's borrow magnitude (~2.0 ETH), so the test catches a regression where Bob's leg was a no-op.

**Refined patch:**

```diff
--- a/test-harness/tests/strategy_happy_tests.rs
+++ b/test-harness/tests/strategy_happy_tests.rs
@@ -572,6 +572,30 @@ fn test_multiply_two_users() {

     assert_ne!(alice_id, bob_id, "accounts should be different");

+    let alice_supply = t.supply_balance_for(ALICE, alice_id, "USDC");
+    let bob_supply = t.supply_balance_for(BOB, bob_id, "USDC");
+    assert!(
+        (2999.0..=3001.0).contains(&alice_supply),
+        "Alice should have ~3000 USDC supply, got {}",
+        alice_supply
+    );
+    assert!(
+        (5999.0..=6001.0).contains(&bob_supply),
+        "Bob should have ~6000 USDC supply, got {}",
+        bob_supply
+    );
+    let alice_borrow = t.borrow_balance_for(ALICE, alice_id, "ETH");
+    let bob_borrow = t.borrow_balance_for(BOB, bob_id, "ETH");
+    assert!(
+        (0.99..=1.01).contains(&alice_borrow),
+        "Alice should owe ~1 ETH, got {}",
+        alice_borrow
+    );
+    assert!(
+        (1.99..=2.01).contains(&bob_borrow),
+        "Bob should owe ~2 ETH, got {}",
+        bob_borrow
+    );
+    let alice_addr = t.get_or_create_user(ALICE);
+    let bob_addr = t.get_or_create_user(BOB);
+    assert_eq!(t.get_account_owner(alice_id), alice_addr);
+    assert_eq!(t.get_account_owner(bob_id), bob_addr);
+
     let alice_hf = t.health_factor_for(ALICE, alice_id);
     let bob_hf = t.health_factor_for(BOB, bob_id);
```

---

### 9. `strategy_happy_tests.rs::test_swap_debt_hf_improvement` — Phase 1: weak

**Disposition:** refined

The auditor's diagnosis (test name claims HF improvement but only asserts `hf_after >= 1.0`) is correct. **However, their suggested patch (`assert!(hf_after > hf_before)`) would fail.** I ran an instrumented copy of the test:

```
hf_before = 4.0
hf_after  = 2.6666666666666665
```

The HF gets **worse**, not better. Reason: 10 ETH at $2000 = $20,000 of debt is replaced with 0.5 WBTC at $60,000 = $30,000 of debt. The test name and the comment ("Swapping to a cheaper debt can improve the HF") are misaligned with the actual asset prices configured in the presets (`presets.rs:147` ETH=$2000, `presets.rs:158` WBTC=$60,000). What the test currently verifies is "swap to *more expensive* debt does not blow HF below 1.0", which is a meaningful invariant but not what the name advertises.

The right fix is one of:

(a) Re-purpose: rename the test to `test_swap_debt_keeps_hf_above_one_with_costlier_debt` and assert `hf_after >= 1.0` plus `hf_after < hf_before` (the strict "got worse" direction the existing setup actually exhibits) so a regression that *somehow* improved HF (e.g. forgot to record the new debt) is caught.

(b) Re-purpose to actually improve HF: swap a smaller WBTC notional so the new USD debt is < the old, and then assert `hf_after > hf_before`.

I recommend option (a) because it preserves the existing setup and locks in the observed HF *direction*, which is the strongest assertion against silent bug regressions on this code path.

**Refined patch (option a):**

```diff
--- a/test-harness/tests/strategy_happy_tests.rs
+++ b/test-harness/tests/strategy_happy_tests.rs
@@ -588,11 +588,12 @@
 // ===========================================================================

 // ---------------------------------------------------------------------------
-// test_swap_debt_hf_improvement
-// Swapping to a cheaper debt can improve the HF.
+// test_swap_debt_to_costlier_debt_preserves_minimum_hf
+// Swap 10 ETH ($20k) debt -> 0.5 WBTC ($30k) debt: USD debt grows, so HF
+// shrinks but must stay >= 1.0. Pinning the strict direction catches any
+// regression that silently dropped the new debt or kept the old.
 // ---------------------------------------------------------------------------

 #[test]
-fn test_swap_debt_hf_improvement() {
+fn test_swap_debt_to_costlier_debt_preserves_minimum_hf() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
@@ -614,9 +615,15 @@
     let hf_after = t.health_factor(ALICE);
     assert!(
         hf_after >= 1.0,
         "HF should still be >= 1.0 after swap_debt, got {}",
         hf_after
     );
+    assert!(
+        hf_after < hf_before,
+        "HF must shrink when swapping to costlier debt: before={}, after={}",
+        hf_before,
+        hf_after
+    );
 }
```

---

### 10. `strategy_panic_coverage_tests.rs::test_multiply_with_collateral_token_initial_payment` — Phase 1: weak

**Disposition:** confirmed

500 USDC initial payment minted to Alice (line 304). Test asserts ~3500 supply and ~1.0 borrow, but not the wallet decrement. Auditor's `token_balance` delta patch is correct.

---

### 11. `strategy_panic_coverage_tests.rs::test_multiply_with_third_token_initial_payment_swaps_via_convert_steps` — Phase 1: weak

**Disposition:** confirmed

0.1 WBTC initial payment minted (line 362), but the wallet decrement is never asserted. Patch is correct.

---

### 12. `strategy_panic_coverage_tests.rs::test_swap_tokens_allowance_remains_zero_after_overpull_rejection` — Phase 1: weak

**Disposition:** refined

**Same wrong-error issue as #1.** The auditor wants to pin the rejection layer with `Auth/InvalidAction`, but the actual rejection is `Error(Contract, #9)` from the SAC. This was verified empirically (same OverPull mode, same call path). The auditor's diagnosis is right (a bare `is_err()` lets a regression that rejects multiply earlier silently pass), but the suggested constant is wrong. Pin the SAC contract error #9 instead. The post-rollback allowance check that follows on lines 438-445 is the load-bearing assertion of this test and stays as-is.

**Refined patch:**

```diff
--- a/test-harness/tests/strategy_panic_coverage_tests.rs
+++ b/test-harness/tests/strategy_panic_coverage_tests.rs
@@ -429,7 +429,15 @@ fn test_swap_tokens_allowance_remains_zero_after_overpull_rejection() {
         common::types::PositionMode::Multiply,
         &steps,
     );
-    assert!(result.is_err(), "OverPull must be rejected");
+    // The SAC rejects the 2x pull with its insufficient-allowance error
+    // (Error(Contract, #9)), surfaced through the controller as a contract
+    // error. Pinning the exact code stops a regression that rejects
+    // multiply at an earlier layer (different contract error) from
+    // silently passing the post-rollback allowance check below.
+    assert_contract_error::<u64>(
+        result,
+        9,
+    );

     // After rollback, the controller's ETH allowance on the bad router must
     // be zero. A regression that leaks the pre-approved allowance would
```

---

## Cross-cutting summary

The auditor's pattern analysis (initial-payment success paths skipping wallet deltas; happy-path mode tests asserting mode but not magnitudes; adversarial tests bottoming out at `is_err()`) is accurate. The substantive disagreement is on **which error code** the OverPull rejection produces. The auditor reasoned (plausibly) that the SAC's allowance check would surface as a host-level `Auth/InvalidAction`, but the SAC implementation in this build (`soroban-sdk =26.0.0-rc.1`, see `Cargo.toml:8`) raises `Error(Contract, #9)` — a contract error — when `transfer_from` overshoots the allowance. Confirmed empirically with a temporary instrumented test that printed `Err(Error(Contract, #9))` and then verified `Error::from_contract_error(9) == Error(Contract, #9)`. Phase 2 corrects the two affected patches (#1 and #12) to pin the actual contract error code via the existing `assert_contract_error` helper rather than an XDR-typed host error.

The HF-improvement test (#9) is a similar empirical correction: the existing setup makes HF *worse* (4.0 → 2.67), so the auditor's `hf_after > hf_before` patch would flip the test from too-loose to broken. The refined patch locks in the observed direction (`hf_after < hf_before`) and renames the test to match its actual assertion.

No new findings beyond the auditor's set: the rest of the suite (69 of 81 tests) uses `assert_contract_error` correctly and pins balances/HF to tight numeric ranges.
