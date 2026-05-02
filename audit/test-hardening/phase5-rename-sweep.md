# Phase 5 — Second-pass misleading-name sweep

**Files scanned:** 49 integration files (test-harness/tests/) + 27 inline-source files (controller/src, pool/src, common/src).
**Misleading names found:** 2
**Stale comments / banners found:** 3
**Stale doc-comments found:** 0

This pass focused on patterns the first sweep missed: stale arithmetic comments inside test bodies, mis-numbered or stale section banners, claim-vs-verification mismatches, plural-vs-singular slips, doc-comment lies, and inline-source tests (which were skipped entirely in phase 4).

## Misleading names

### `test-harness/tests/strategy_edge_tests.rs:442::test_multiply_rejects_mode_4`

**Pattern:** 4 (claim-vs-verification mismatch) and 1 (stale comment inside body).
**Current:** `test_multiply_rejects_mode_4`
**Proposed:** `test_multiply_rejects_normal_mode`
**Why:** The body at line 455 passes `PositionMode::Normal` (which encodes as `0`, per `common/src/types.rs:29`) — not mode 4. The inline comment at line 449 says "mode = 4 is out of range (valid: 1, 2, 3)", but the actual rejection is for `Normal` (= 0). The test verifies that `multiply` rejects the `Normal` variant; "mode 4" is fictional under the current `PositionMode` enum (which has variants `Normal=0, Multiply=1, Long=2, Short=3`). A reader looking for the "out-of-range mode 4" path would not find it here.
**Patch:**

```diff
--- a/test-harness/tests/strategy_edge_tests.rs
+++ b/test-harness/tests/strategy_edge_tests.rs
@@ -439,9 +439,9 @@ fn test_multiply_siloed_debt_conflict_does_not_apply() {
 // ---------------------------------------------------------------------------

 #[test]
-fn test_multiply_rejects_mode_4() {
+fn test_multiply_rejects_normal_mode() {
     let mut t = LendingTest::new()
         .with_market(usdc_preset())
         .with_market(eth_preset())
         .build();

     let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
-    // mode = 4 is out of range (valid: 1, 2, 3).
+    // PositionMode::Normal is reserved for non-strategy accounts; multiply requires
+    // Multiply, Long, or Short.
     let result = t.try_multiply(
         ALICE,
         "USDC",
         1.0,
         "ETH",
         common::types::PositionMode::Normal,
         &steps,
     );
     assert_contract_error(result, errors::INVALID_POSITION_MODE);
 }
```

### `controller/src/tests.rs:483::test_position_limit_enforcement`

**Pattern:** 4 (claim-vs-verification mismatch) and 1 (stale comment).
**Current:** `test_position_limit_enforcement`
**Proposed:** `test_position_limit_reaches_configured_max`
**Why:** Body at lines 501-526 stores exactly two positions (matching the `max_supply_positions = 2` limit set on line 493) and asserts `account.supply_positions.len() == 2` plus `len() >= limits.max_supply_positions`. Both assertions are tautological given the loop body. The inline comment at line 518 reads "The limit check must now fail for a third position", but no third position is ever attempted — `update_or_remove_position` is never called for a third asset, and `validate_bulk_position_limits` (the actual enforcement function in `controller/src/validation.rs:115`) is never invoked in the test. The harness equivalent `test_supply_position_limit_exceeded` (`test-harness/tests/supply_tests.rs:256`) does verify enforcement by attempting a third supply and asserting the rejection error; this controller-side test does not.
**Patch:**

```diff
--- a/controller/src/tests.rs
+++ b/controller/src/tests.rs
@@ -478,9 +478,9 @@
 // -----------------------------------------------------------------------
-// Test: position limit enforcement
+// Test: position storage reaches configured max (no enforcement assertion)
 // -----------------------------------------------------------------------
 #[test]
-fn test_position_limit_enforcement() {
+fn test_position_limit_reaches_configured_max() {
     let t = TestSetup::new();
     let client = t.client();
     let owner = Address::generate(&t.env);
@@ -515,11 +515,11 @@
         }
         storage::set_account(&t.env, id, &account);

-        // The limit check must now fail for a third position.
+        // After storing two positions the supply_positions map is at the
+        // configured limit. End-to-end enforcement (rejecting a third asset)
+        // is covered by test-harness supply_tests::test_supply_position_limit_exceeded.
         let account = storage::get_account(&t.env, id);
         assert_eq!(account.supply_positions.len(), 2);
         let limits = storage::get_position_limits(&t.env);
         assert!(
             account.supply_positions.len() >= limits.max_supply_positions,
             "should be at limit"
         );
     });
 }
```

## Stale comments / banners

### `test-harness/tests/flash_loan_tests.rs:220`

The section banner (line 220) reads `// 10. test_flash_loan_fee_calculation`, but the function below is named `test_flash_loan_fee_config_matches_default_preset` (line 224). The function was renamed (see the in-body comment at lines 225-233 explaining the rewrite from a tautological fee-arithmetic test to a config-pinning test) but the banner stayed pointing at the old name.

**Patch:**

```diff
--- a/test-harness/tests/flash_loan_tests.rs
+++ b/test-harness/tests/flash_loan_tests.rs
@@ -217,7 +217,7 @@ fn test_flash_loan_reentrancy_blocks_liquidation() {
 }

 // ---------------------------------------------------------------------------
-// 10. test_flash_loan_fee_calculation
+// 10. test_flash_loan_fee_config_matches_default_preset
 // ---------------------------------------------------------------------------

 #[test]
 fn test_flash_loan_fee_config_matches_default_preset() {
```

### `test-harness/tests/decimal_diversity_tests.rs:144`

The banner reads `// 4. All five decimal types in one account`, but the function `test_mixed_decimal_types_single_account` (line 148) only registers four markets: `usdc_6dec`, `wbtc_8dec`, `sol_9dec`, `dai_18dec` (lines 150-153). The 7-decimal `xlm_7dec` preset (defined at line 65) is not used in this test. The body covers four decimal classes, not five.

**Patch:**

```diff
--- a/test-harness/tests/decimal_diversity_tests.rs
+++ b/test-harness/tests/decimal_diversity_tests.rs
@@ -141,7 +141,7 @@ fn test_supply_9dec_borrow_8dec() {
 }

 // ---------------------------------------------------------------------------
-// 4. All five decimal types in one account
+// 4. Four decimal types (6/8/9/18) in one account
 // ---------------------------------------------------------------------------

 #[test]
 fn test_mixed_decimal_types_single_account() {
```

### `pool/src/cache.rs:190::test_load_uses_zeroed_defaults_when_state_is_missing`

This is a misleading test name driven by a stale-claim pattern. Body at lines 196-205 asserts that on a missing-state load: `supplied=ZERO`, `borrowed=ZERO`, `revenue=ZERO`, `last_timestamp=0` — but `borrow_index=Ray::ONE` and `supply_index=Ray::ONE` (NOT zero). The name "zeroed_defaults" lies about the indexes, which default to the multiplicative identity (1 RAY) — the value that makes a fresh pool consistent. A reader who trusts the name would expect every field to be zero and miss the index initialization invariant.

**Pattern:** This is on the borderline between "misleading name" and "stale comment", but since the inline-source test was skipped entirely in phase 4, it is reported here as a stale-claim banner rather than a separate misleading-name entry. Either the rename or a one-line clarifying comment above the test fixes the trap.

**Patch (rename):**

```diff
--- a/pool/src/cache.rs
+++ b/pool/src/cache.rs
@@ -187,7 +187,7 @@ mod tests {
     }

     #[test]
-    fn test_load_uses_zeroed_defaults_when_state_is_missing() {
+    fn test_load_uses_neutral_defaults_when_state_is_missing() {
         let t = TestSetup::new();

         t.as_contract(|| {
             t.env.storage().instance().remove(&PoolKey::State);
             let cache = Cache::load(&t.env);

             assert_eq!(cache.supplied, Ray::ZERO);
             assert_eq!(cache.borrowed, Ray::ZERO);
             assert_eq!(cache.revenue, Ray::ZERO);
             assert_eq!(cache.borrow_index, Ray::ONE);
             assert_eq!(cache.supply_index, Ray::ONE);
             assert_eq!(cache.last_timestamp, 0);
             assert_eq!(cache.current_timestamp, 1_000_000);
             assert_eq!(cache.params.asset_id, t.params.asset_id);
         });
     }
```

## Stale doc-comments

None found. No `///` doc-comment above any `#[test]` function in scope was observed to contradict the function body. Test files use plain `//` section banners (which are flagged separately above), and the few `///` doc-comments that do appear (e.g. `test-harness/tests/revenue_tests.rs:60-63` describing the new revenue flow, `test-harness/tests/pool_revenue_edge_tests.rs:25-28`, `test-harness/tests/chaos_simulation_tests.rs:43-50`) match their bodies.

## Notes on what was checked but not flagged

- All 11 already-fixed renames listed in the prompt were observed under their new names; none were re-flagged.
- HF/debt/collateral arithmetic comments inside test bodies in `borrow_tests.rs`, `liquidation_tests.rs`, `views_tests.rs`, `withdraw_tests.rs`, `liquidation_coverage_tests.rs`, `liquidation_math_tests.rs`, `bad_debt_index_tests.rs`, `isolation_tests.rs`, `strategy_happy_tests.rs`, `stress_simulation_tests.rs`, and `chaos_simulation_tests.rs` were spot-checked against assertions; the ones inspected matched (e.g. `// HF = 5920/6000 = 0.9867` paired with `assert!(hf_f64 < 1.0 && hf_f64 > 0.95)` in `liquidation_math_tests.rs:46,118`). One slightly inconsistent body comment in `liquidation_coverage_tests.rs:137-141` (claims ideal=$88 then says "fulfills the ideal (~$148)") was noted but not flagged because the test logic does not depend on the arithmetic and the assertions only check directional debt change.
- `test_apply_bad_debt_emits_insolvent_event_on_severe_reduction` (`pool/src/interest.rs:370`) — name claims an event is emitted but the body only asserts the supply index dropped >10x; no `env.events()` inspection. Borderline emit-claim; left unflagged because Soroban event-emission tests are uncommon in this codebase and the body does exercise the severe-reduction code path that emits the event in production. A future rename to `_collapses_supply_index_on_severe_reduction` would be tighter, but it is not actively misleading the way the two `Misleading names` entries are.
- `test_pool_borrow_rate_increases_with_borrows` (`test-harness/tests/utils_tests.rs:206`) uses `>=` not `>`; the name says "increases". Borderline — accepted because the base rate is non-zero so the only way `>=` holds non-trivially is when the rate strictly increases, and the loose comparison guards against rate-curve rounding.
- `test_seize_position_bad_debt` (`pool/src/lib.rs:1215`), `test_seize_position_deposit_dust` (`pool/src/lib.rs:1262`), `test_set_aggregator_stores_wasm_contract_address` and `test_set_accumulator_stores_wasm_contract_address` (`controller/src/config.rs:994,1014`) have somewhat overspecified names ("WASM" is not actually verified, "bad debt" / "dust" are mechanism descriptions rather than assertions) but the bodies exercise the named code paths cleanly. Skipped — minor specificity nits, not active misleads.
- `test_update_params_rejects_optimal_utilization_above_one` (`pool/src/lib.rs:1376`) actually triggers when `optimal_utilization >= RAY` (1.0), not strictly above. Name says "above_one"; the test passes `RAY` exactly. Borderline; left unflagged because the off-by-equality is small and the panic site (`pool/src/lib.rs:631`) confirms the `>=` semantics so a future reader reading both files will not be deceived.
