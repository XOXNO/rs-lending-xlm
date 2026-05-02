# Certora Review — Liquidation

**Files reviewed:**
- `controller/certora/spec/liquidation_rules.rs` (384 lines)
- Production references: `controller/src/positions/liquidation.rs`, `controller/src/helpers/mod.rs`, `controller/src/positions/repay.rs`, `common/src/constants.rs`, `common/src/types.rs`, `common/src/fp.rs`, `controller/certora/spec/summaries/mod.rs`

**Rules examined:** 11 (9 active `#[rule]`s + 2 sanity checks; 3 deleted/commented-out rules acknowledged)

**Findings:** 17 (3 high, 7 medium, 7 low)

---

## Per-rule verdicts

### `liquidation_rules.rs::hf_improves_after_liquidation` (line 32)

- **Verdict: broken (vacuous under the active summary)**
- **Property claimed:** After `process_liquidation` succeeds, the borrower's HF cannot regress (`hf_after >= hf_before`).
- **What's actually asserted:** `cvlr_assert!(hf_after >= hf_before)` at `liquidation_rules.rs:63`, where `hf_before` and `hf_after` are both produced by `calculate_health_factor_for`, which is **summarized** at `controller/src/helpers/mod.rs:118-133`. The summary at `controller/certora/spec/summaries/mod.rs:129-137` returns `nondet()` with only `hf >= 0`. The two calls produce **independent nondet i128s**.
- **Concrete bug it catches / misses:**
  - **Misses everything.** Because the summary returns an unconstrained `i128 >= 0` independently per call, there is no causal link between `hf_before`, the production-code mutation done by `process_liquidation`, and `hf_after`. The solver simply finds a model where the second nondet < first nondet and the assertion *fails*. Conversely, if the prover happens to pass it, it's only because both nondets were sampled equal — which proves nothing.
  - It does not (and cannot, under the current summary) catch: HF regressing because of a buggy bonus formula, miscomputed seizure leaving more debt than collateral can support, or `apply_liquidation_repayments` writing the wrong scaled amount.
- **Issues:**
  1. **High:** the rule depends on the summarized helper preserving a relationship between the two values; the summary at summaries/mod.rs:129 does not. To be sound, this rule must read the production HF at both ends, OR the summary must be relational (e.g., a ghost storing the post-liquidation HF derived from the post-state).
  2. **Medium:** the rule does not bound `debt_amount` (only `> 0`). With `i128::MAX` debt the production path will overflow in `apply_liquidation_repayments` -> `token.transfer` long before the math executes; the rule will not exercise meaningful liquidation paths.
  3. **Low:** comment at line 31 says "see fuzz_supply_borrow_liquidate" — a useful pointer, but the assertion `>=` is so weak that even a buggy implementation that reduces HF to `1` (effectively making the position unliquidatable rather than fixed) would pass when both nondets are 0.
- **Recommendation:**
  - Remove the summary at the call site for HF inside this rule (use a `#[cfg(not(certora_summarize_hf))]` direct call), or add a relational ghost variable that captures the production HF computation.
  - Strengthen the post-condition: require either `hf_after >= hf_before` OR `account.borrow_positions.is_empty()` (full close), AND assert no overflow / no panic was hit.
  - Bound `debt_amount` to a realistic ceiling (e.g., `<= 10^30`).

---

### `liquidation_rules.rs::bonus_bounded` (line 79)

- **Verdict: strong**
- **Property claimed:** The dynamic bonus never exceeds `MAX_LIQUIDATION_BONUS` (1500 BPS), regardless of inputs.
- **What's actually asserted:** `cvlr_assert!(bonus.raw() <= MAX_LIQUIDATION_BONUS)` at line 94 — directly checks the production cap line at `helpers/mod.rs:218` (`Bps::from_raw(bonus.raw().min(MAX_LIQUIDATION_BONUS))`).
- **Concrete bug it catches:** removing the `.min(MAX_LIQUIDATION_BONUS)` clamp; using a wrong constant (e.g., `MAX_LIQUIDATION_BONUS = 5000`); switching to a non-saturating `checked_add` only path that lets `base + bonus_increment` exceed cap.
- **Issues:**
  - **Caveat (medium):** the rule calls the **summarized** `calculate_linear_bonus` (`helpers/mod.rs:222-228`). The summary at `summaries/mod.rs:174-184` returns `bonus ∈ [base_bonus, max_bonus]`. The summary already enforces the bound, so the rule essentially proves a property of the summary, not the production code. To verify production it must either be unsummarized, or the summary must additionally havoc beyond `max_bonus` (which would invalidate other rules). This is a **summary-trusting tautology** masquerading as a property check.
- **Recommendation:** Either disable the summary for this rule (call `calculate_linear_bonus_with_target` directly, which is unsummarized at `helpers/mod.rs:194`), or add a separate rule that exercises `calculate_linear_bonus_with_target` and asserts the same bound — which would be a real verification.

---

### `liquidation_rules.rs::bonus_max_at_deep_underwater` (line 111)

- **Verdict: weak (almost a tautology under the summary)**
- **Property claimed:** At deeply-underwater HF (=0.5 WAD), the bonus equals `max_bonus_bps.min(MAX_LIQUIDATION_BONUS)`.
- **What's actually asserted:** `cvlr_assert!(bonus.raw() == expected_max)` at line 132.
- **Concrete bug it catches / misses:**
  - With the summary in effect, the summary may legally return any `Bps` in `[base, max]` — *not* necessarily `max`. So this assertion will FAIL under the summary (the solver can pick `bonus = base` which is in-range but not `expected_max`). That means either: (a) the rule is currently *broken* under the standard summary configuration, or (b) the rule disables the summary to call the real `calculate_linear_bonus_with_target`.
  - I see no `#[cfg]` to disable the summary. **This rule is broken under the active spec.** It cannot pass.
- **Issues:**
  1. **High:** if the spec is run as written, this rule will produce a counterexample whenever the summary returns `bonus < max`. If the rule has been observed to pass, the configuration must skip this rule or the summary must be different in CI than what's in the source — investigate.
  2. **Low:** the boundary `WAD/2` is an arbitrary "deeply underwater" pick; consider also testing the formal saturation point (`hf <= 1.02*WAD - 0.51*1.02*WAD`).
- **Recommendation:** Bypass the summary by calling `calculate_linear_bonus_with_target` directly, which executes the real interpolation. Otherwise delete the rule.

---

### `liquidation_rules.rs::seizure_proportional` (line 142)

- **Verdict: weak (verifies a local re-implementation)**
- **Property claimed:** Each collateral asset is seized proportionally to its value share.
- **What's actually asserted:** lines 166-173 — `seizure_a >= 0`, `seizure_b >= 0`, `seizure_a + seizure_b <= total + 1`, and `seizure_a >= seizure_b` when `value_a > value_b`.
- **Concrete bug it catches / misses:**
  - **Misses the production code entirely.** This rule re-implements the share-and-seize math inline (`mul_div_half_up` at lines 158-162) and asserts properties of its own arithmetic. It never exercises `calculate_seized_collateral` (`liquidation.rs:312-375`), which is the actual function with the bug surface (loops over supply positions, applies `floor` for the base side, computes `protocol_fee` via `Bps::apply_to`, caps at `actual_amount`).
  - Buggy implementations it would still pass: a production version that always seizes `0` from the smaller asset, that swaps `share_a` and `share_b`, that double-counts the bonus, or that omits the `actual_amount` cap.
- **Issues:**
  1. **Medium:** local re-implementation = tautology bridge. The assertions verify properties of `mul_div_half_up`, not of liquidation seizure.
  2. **Medium:** `seizure_a + seizure_b <= total_seizure_usd_wad + 1` with a `+1` rounding fudge is too generous. The production code uses half-up rounding, and the sum of two half-up rounded values can deviate by up to `±1` per term, so the bound should be `± 2`. More importantly the rule doesn't constrain rounding *direction* — it cannot detect a bug that systematically over-rounds (which would be a real revenue leak).
  3. **Low:** only 2 assets; production allows up to `MAX_SUPPLY_POSITIONS = 4`. A 3- or 4-asset case would test sum-of-shares more thoroughly.
- **Recommendation:** Replace with a rule that calls `process_liquidation` (or `calculate_seized_collateral` directly if it can be made callable from the spec), ghosts the *initial* `actual_amount_wad * price` per asset, and asserts that `Σ seized_usd ≈ repaid_usd × (1 + bonus)` to within rounding tolerance, AND that no asset is seized beyond `actual_amount`.

---

### `liquidation_rules.rs::protocol_fee_on_bonus_only` (line 184)

- **Verdict: weak (re-implements the production formula)**
- **Property claimed:** `protocol_fee = bonus_amount × liquidation_fees_bps / BPS`, never exceeds `bonus_amount`, never goes negative.
- **What's actually asserted:** lines 203-214 — `fee <= bonus_amount`, `fee >= 0`, `fee == 0` when `fees_bps == 0`, `fee < seizure_amount`.
- **Concrete bug it catches / misses:**
  - **Misses production drift.** The rule recomputes the formula at lines 197-200 with `mul_div_half_up`, but the production at `liquidation.rs:357-363` uses `Wad::from_raw(capped_amount).div_floor(env, one_plus_bonus)` (`div_floor`, not `mul_div_half_up`!). The two do not produce the same `base_amount` for boundary inputs. So the rule does NOT verify the production code — it verifies a separate formula that drifts from production.
  - Buggy implementations the rule would pass:
    - Production switching from `div_floor` to half-up rounding (a real change that would affect protocol revenue accounting).
    - Production using `liquidation_fees_bps` from the wrong asset config.
    - Production applying the fee to `seizure_amount` instead of `bonus_portion` (because the rule still computes its own bonus locally and asserts properties of that).
  - **Strong properties it does verify (in-the-small):** `protocol_fee >= 0`, `fee == 0 when bps == 0`, `fee <= bonus`. These are useful but are properties of the local arithmetic, not of liquidation.
- **Issues:**
  1. **High:** local re-derivation diverges from production (`div_floor` vs `mul_div_half_up` at line 198 vs `liquidation.rs:359`). Anyone reading this rule would mistakenly think the production formula is the half-up version.
  2. **Medium:** the rule never reads the actual `liquidation_fees_bps` from `AssetConfig` — it takes it as a free parameter. A bug where production reads the wrong field (e.g., `flashloan_fee_bps`) would not be caught.
  3. **Low:** missing assertion that `base_amount + bonus_amount + protocol_fee` does not exceed `seizure_amount` — the cap in production is `seizure_amount.min(actual_amount)`, and the rule should mirror that.
- **Recommendation:** Replace `mul_div_half_up` at line 198 with `div_floor` to mirror production. Better yet, call `calculate_seized_collateral` directly and compare its `protocol_fee` field against the recomputed expected value.

---

### `liquidation_rules.rs::bad_debt_threshold` (line 224)

- **Verdict: tautology**
- **Property claimed:** Bad-debt cleanup triggers iff `debt > collateral && collateral <= 5*WAD`.
- **What's actually asserted:** lines 234-246 — three implications: `coll > 5*WAD => !qualifies`, `debt <= coll => !qualifies`, `coll == 0 && debt > 0 => qualifies`.
- **Concrete bug it catches / misses:**
  - **Catches nothing.** The variable `qualifies` is *defined locally on line 230* as `debt > coll && coll <= 5*WAD`. The three assertions then check definitional properties of the local boolean expression — pure propositional logic. They never exercise the production predicate at `liquidation.rs:445` or the `clean_bad_debt_standalone` path at `liquidation.rs:478`.
  - Buggy implementations the rule would pass:
    - Production using `>=` instead of `>` at `liquidation.rs:445`.
    - Production using `BAD_DEBT_USD_THRESHOLD = 50 * WAD` (wrong constant).
    - Production failing to call `execute_bad_debt_cleanup` even when the predicate is true.
- **Issues:**
  1. **High:** this is a textbook tautology. Tagged "Rule 8: Bad debt threshold" with confident-sounding language, but proves nothing about the production code.
  2. **Low:** the rule also doesn't reference `BAD_DEBT_USD_THRESHOLD` from `common::constants` — it hard-codes `5 * WAD`. If the constant changes the rule will silently desynchronize.
- **Recommendation:** Either delete (the boundary case is already covered by `boundary_rules.rs::bad_debt_at_exactly_5_usd` at lines 295 and `bad_debt_at_6_usd` at line 323 — verify those run unsummarized), or rewrite to call `clean_bad_debt_standalone` and assert that it panics with `CannotCleanBadDebt` on inputs that should not qualify. Use `BAD_DEBT_USD_THRESHOLD` directly.

---

### `liquidation_rules.rs::bad_debt_supply_index_decreases` (line 257)

- **Verdict: broken (cross-contract call cannot be observed by the controller storage)**
- **Property claimed:** When bad debt is socialized, the supply index must decrease (loss spread to suppliers).
- **What's actually asserted:** `cvlr_assert!(supply_after <= supply_before)` at line 275, with `index_before` and `index_after` read from the controller's `storage::market_index::get_market_index`.
- **Concrete bug it catches / misses:**
  - **The supply index is owned by the pool contract, not the controller.** The pool's `apply_bad_debt_to_supply_index` at `pool/src/interest.rs:115` updates `cache.supply_index` and saves it via `cache.save()` to the *pool's* instance storage (`PoolKey::State` at `common/types.rs:589-593`). The controller has no `market_index::get_market_index` that reads pool state across contracts; the controller only stores `MarketIndex` in its cache for the duration of a transaction.
  - Therefore the storage path the rule reads is either:
    - (a) a controller-side mirror that this rule never updates because the cross-contract call is summarized to nondet, or
    - (b) a non-existent function (the rule may not even compile in `--cfg certora`).
  - In case (a) the rule is vacuous: `index_before == index_after` trivially. In case (b) the rule is a phantom.
  - **Concrete miss:** a buggy `apply_bad_debt_to_supply_index` that *increases* the index (e.g., wrong sign on `reduction_factor`) is not caught.
- **Issues:**
  1. **High:** `crate::storage::market_index::get_market_index` does not appear to exist in the audited tree (the storage module is now `controller/src/storage/market.rs` etc.). The rule may be referencing a stale path that was never updated when storage was modularized in the unstaged changes (`?? controller/src/storage/market.rs`).
  2. **High:** even if the path resolves, the supply index lives in the pool, not the controller. Cross-contract effects are havoced under Certora summaries (see `summaries/mod.rs:88-101` for the pool index summary). The rule is unable to observe the real socialization.
  3. **Medium:** `cvlr_assume!(supply_before >= RAY)` is not enough — the rule should also assume the account qualifies for cleanup (no `cvlr_assume` on the bad-debt predicate, so cleanup may panic before reaching the assertion).
- **Recommendation:** Move this property into a pool-side spec (`pool/src/...`) where the index actually lives, OR add a ghost in `summaries/mod.rs::update_asset_index_summary` that records "after-cleanup" semantics. As written this rule cannot validate the production property.

---

### `liquidation_rules.rs::ideal_repayment_targets_102` (line 293)

- **Verdict: weak (sanity-only)**
- **Property claimed:** Ideal repayment from `estimate_liquidation_amount` produces a value that, when applied, brings HF to ~1.02.
- **What's actually asserted:** lines 330-339 — `ideal > 0`, `ideal <= total_debt`, `ideal <= total_collateral / (1 + bonus) + 1`. Notably **does NOT assert that the post-liquidation HF is near 1.02**, despite being the rule's name.
- **Concrete bug it catches / misses:**
  - The three asserted bounds are good but mostly fall out of `try_liquidation_at_target`'s `min(d_max, total_debt)` at `helpers/mod.rs:365`.
  - **Misses the named property:** the rule never computes `calculate_post_liquidation_hf` and never checks it lands near `1.02`. A buggy `estimate_liquidation_amount` that always returns `min(total_debt, ε)` (some tiny number) would pass: `ε > 0`, `ε <= total_debt`, `ε <= max_repayable + 1`.
  - The simplification at line 316 (`total_collateral_wad = total_debt_wad`) is a structural assumption that hides the relationship between collateral and the bonus formula. It also makes `weighted_collateral < total_debt = total_collateral`, which means the LT-weighted ratio is `< 1` — fine — but `proportion_seized = weighted/total_debt` is being passed as if it were `weighted/total_collateral`, which is what production passes (`liquidation.rs:223-224`). When `total_collateral == total_debt`, these coincide; in any other case, the rule tests math the production never executes.
- **Issues:**
  1. **Medium:** the property name and comment promise "targets HF = 1.02" but the assertion is silent on post-liquidation HF.
  2. **Medium:** `total_collateral_wad = total_debt_wad` (line 316) is a degenerate special case. Real liquidatable accounts have `total_collateral < total_debt` (since HF < 1 *and* LT ≤ 100%, weighted_coll < total_debt; total_collateral can be either side of total_debt).
  3. **Low:** the `+1` rounding tolerance at line 339 is generous given that `mul_div_half_up` and `div` both round half-up; tighter bound is `+0` or `<=` plus a clear off-by-one comment.
- **Recommendation:** Add an assertion that `calculate_post_liquidation_hf(weighted, total_debt, ideal, proportion_seized, bonus) ∈ [1.0*WAD - ε, 1.02*WAD + ε]`. Drop the `total_collateral = total_debt` simplification and let the solver explore.

---

### `liquidation_rules.rs::liquidation_bonus_sanity` (line 347)

- **Verdict: strong (sanity)**
- **Property claimed:** Reachability — there is some valid input where the bonus is positive.
- **What's actually asserted:** `cvlr_satisfy!(bonus.raw() > 0)` at line 361. Sanity rule, properly using `cvlr_satisfy`.
- **Issues:** Under the summary, the bonus is nondet ∈ `[base, max]`; trivially satisfiable. But that's the right shape for a sanity check.
- **Recommendation:** None.

---

### `liquidation_rules.rs::estimate_liquidation_sanity` (line 365)

- **Verdict: strong (sanity)**
- **Property claimed:** Reachability — `estimate_liquidation_amount` returns positive ideal.
- **What's actually asserted:** `cvlr_satisfy!(ideal.raw() > 0)` at line 383. Properly bounded inputs (`total_debt > WAD`, `weighted_col < total_debt`).
- **Issues:**
  - **Low:** `proportion_seized = WAD/2` and `total_collateral = total_debt` are hard-coded. A cleaner sanity rule would let those float to confirm reachability across the input space.
- **Recommendation:** None blocking.

---

### Deleted rules acknowledged in source

- **Rule 2 (`no_over_liquidation`)** at line 67: correctly identified as vacuous (`min(x, y) <= y`). Good catch.
- **Rule 4 (`bonus_zero_at_threshold`)** at line 98: correctly identified as provably wrong (the math at line 99-102 explains why). Good.
- **Rule 10 (`payment_dedup`)** at line 279: correctly identified as a Map tautology. Good.

The deletion comments are useful audit trail and should remain.

---

## Missing invariants

The following critical liquidation safety properties have **no rule** in `liquidation_rules.rs` (and most have no equivalent elsewhere):

1. **HF was strictly < WAD at entry — invariant: `process_liquidation` panics when HF ≥ 1.0.**
   `boundary_rules.rs::liquidation_at_hf_exactly_one` (line 213) and `liquidation_at_hf_just_below_one` (line 235) appear to address the boundary, but they need to be checked against the *actual* `process_liquidation` — confirm they call into production rather than re-asserting the predicate.

2. **Liquidator never receives more collateral than `repaid_usd × (1 + bonus_bps + fees_bps)`.**
   The most important anti-rug invariant. There is no rule asserting `Σ seized_usd <= total_repaid_usd × (1 + max_bonus + max_fees)`. A bug in `calculate_seized_collateral` that doubles the bonus, or that miscomputes `total_seizure_usd = repayment_usd × one_plus_bonus` would not be caught.

3. **Total debt strictly decreases (or account closes).**
   No rule. A buggy `apply_liquidation_repayments` that silently no-ops (e.g., `result.actual_amount = 0`) would pass `hf_improves_after_liquidation` (since both nondets are unrelated).

4. **Total collateral strictly decreases (when seizure > 0).**
   No rule. A bug where `apply_liquidation_seizures` fails to call `withdraw::execute_withdrawal` would not be caught.

5. **`actual_repaid <= debt_outstanding` per asset.**
   This is enforced at `liquidation.rs:261-265` (the `if payment_amount > actual_debt` branch). No rule exists. A bug removing this cap would let liquidators repay more than owed and seize the corresponding collateral at bonus, draining the account's other collateral.

6. **Refunds vector is sound:** `Σ refund_usd + Σ repaid_usd == Σ payment_usd_input` (modulo rounding).
   `process_excess_payment` at `liquidation.rs:377-420` is intricate and unverified. A bug in the proportional refund split could let refunds exceed the original payment (rug the liquidator) or be silently dropped (rug the liquidator the other way).

7. **Isolated debt counter decremented on bad-debt cleanup.**
   `repay::clear_position_isolated_debt` is called in `execute_bad_debt_cleanup` at `liquidation.rs:507`. There is no rule that asserts the global `IsolatedDebt(asset)` counter decreases by the seized USD value. A bug skipping this call would let the isolated-asset cap drift.

8. **`execute_bad_debt_cleanup` removes the account.**
   Asserted only by `liquidation.rs:520` via `positions::account::remove_account`. No Certora rule. A bug leaving the account in place would persist a zombie.

9. **`calculate_seized_collateral` never seizes more than `actual_amount` of any asset.**
   Production caps at line 357 (`seizure_amount.min(actual_amount)`). No rule. A bug here would let liquidators drain more than the account holds.

10. **Bad-debt cleanup zeros all positions** (post-condition of `seize_pool_position` for both deposit and borrow).
    Pool-side property; not in scope, but should have a rule in pool spec.

11. **Liquidation reverts on stale price.**
    The non-isolated path does not enforce safe-price (see `liquidation.rs:55` `ControllerCache::new(env, false)` — `allow_unsafe = false`). No rule asserts that a stale-price cache makes `process_liquidation` panic.

12. **`process_excess_payment` preserves total USD.**
    See item 6.

13. **Self-liquidation:** the production code does not block `liquidator == account.owner`. No rule explores this case to assert it is either rejected or handled correctly (some protocols use self-liquidation for tax/whitelist gaming).

14. **Reentrancy / `require_not_flash_loaning`:** the rule never tests that `process_liquidation` reverts when `FlashLoanOngoing` is set.

---

## Summary integrity

The summaries the liquidation rules depend on are at `controller/certora/spec/summaries/mod.rs`:

- `calculate_health_factor_summary` (line 113) — returns `nondet i128 >= 0`. **Sound but information-free** for the `hf_improves_after_liquidation` rule, because two separate calls return independent nondet values. To verify HF non-regression you need a relational summary (or no summary).
- `calculate_health_factor_for_summary` (line 129) — same shape. Same problem.
- `calculate_account_totals_summary` (line 145) — returns three non-negative WADs with `weighted <= total_collateral`. The order of the tuple in the summary is `(total_collateral, weighted_coll, total_debt)` (line 158-162), but production at `helpers/mod.rs:184` returns `(total_collateral, total_debt, weighted_coll)`. **This is a real bug — a tuple-order mismatch between summary and production.** Any rule that takes the third tuple element as "weighted collateral" via the summary but as "total debt" via production will produce silently wrong analyses.
- `calculate_linear_bonus_summary` (line 174) — returns `Bps ∈ [base, max]`. Sound but **strictly weaker** than production: the summary admits values in the entire interval, while production produces a specific deterministic output. Rules that check exact equality (like `bonus_max_at_deep_underwater` line 132) cannot pass under this summary.

**Critical: confirm the `calculate_account_totals` tuple order.** Production (`helpers/mod.rs:139-186`) returns `(total_collateral, total_debt, weighted_coll)`. Summary (`summaries/mod.rs:145-163`) returns `(total_collateral, weighted_coll, total_debt)`. Any rule consuming this summary will see swapped fields. This affects `check_bad_debt_after_liquidation` (`liquidation.rs:437-442`) which destructures the tuple and any rule path that goes through it.

---

## Severity-tagged action items

### High (blocking)

1. **Fix `calculate_account_totals_summary` tuple order at `summaries/mod.rs:158-162`** to match production `(total_collateral, total_debt, weighted_coll)` from `helpers/mod.rs:184`. This silently breaks every rule that consumes this summary.
2. **Replace `hf_improves_after_liquidation`** (line 32) with either an unsummarized direct call to `calculate_health_factor` or a relational ghost. The current form proves nothing.
3. **Resolve `bad_debt_supply_index_decreases`** (line 257). Either remove (cross-contract observation impossible from controller spec), or move to pool spec, or refactor to call `clean_bad_debt_standalone` and assert observable controller-side effects (e.g., account removal, isolated-debt counter decrement).
4. **Investigate `bonus_max_at_deep_underwater`** (line 111). Under the active summary, the `==` assertion at line 132 is unprovable. Either bypass the summary or delete.
5. **Fix `protocol_fee_on_bonus_only` formula drift** (line 198): production uses `div_floor`, the rule uses `mul_div_half_up`. Switch to `div_floor` to mirror production.

### Medium

6. **Delete or rewrite `bad_debt_threshold`** (line 224). The current form is a propositional tautology. Replace with a rule that calls `clean_bad_debt_standalone` and asserts the panic / non-panic boundary against `BAD_DEBT_USD_THRESHOLD` (the constant, not a literal).
7. **Strengthen `seizure_proportional`** (line 142) to call `calculate_seized_collateral` instead of re-implementing. Test with up to 4 assets.
8. **Add anti-rug invariant rule:** `Σ seized_usd <= Σ repaid_usd × (1 + MAX_LIQUIDATION_BONUS + MAX_FEES)`.
9. **Add per-asset seizure cap rule:** for every entry in the seizure result, `entry.amount <= actual_supply_amount`.
10. **Add refund conservation rule:** `Σ refund_usd + Σ repaid_usd ≈ Σ input_payment_usd` modulo per-asset rounding.
11. **Add account-totals decrease rule:** total collateral strictly decreases (or zeros) AND total debt strictly decreases (or zeros) after `process_liquidation`.
12. **Add stale-price liquidation revert rule.**

### Low

13. **`bad_debt_threshold`** (line 228): use `BAD_DEBT_USD_THRESHOLD` constant instead of `5 * WAD` literal so the rule tracks production.
14. **`hf_improves_after_liquidation`** (line 40): bound `debt_amount` to a realistic range.
15. **`ideal_repayment_targets_102`** (line 293): assert post-liquidation HF lands near 1.02 (matching the rule name).
16. **`seizure_proportional`** (line 168): tighten the rounding tolerance from `+1` to a documented bound based on the half-up step count.
17. **`estimate_liquidation_sanity`** (line 365): let `proportion_seized` and `total_collateral` float to widen reachability coverage.
