# Certora Efficiency — Liquidation

**Files in scope:**
- `controller/certora/spec/liquidation_rules.rs` (415 lines, 9 active `#[rule]`s + 2 sanity)
- `controller/confs/liquidation.conf` (`loop_iter: 12`, `-maxBlockCount 300000`)
- Production: `controller/src/positions/liquidation.rs`, `controller/src/helpers/mod.rs:55-411`
- Summaries: `controller/certora/spec/summaries/mod.rs`, `controller/certora/spec/summaries/pool.rs`

**Soundness audit reference:** `audit/certora-review/02-liquidation.md` (3 high / 7 medium / 7 low). This pass is orthogonal: it asks **"will the prover finish, and is the work it does load-bearing?"**, not "does the property hold?".

**Headline:** at least 4 of the 9 active rules are TAC-budget-canaries (likely time out or overflow `-maxBlockCount 300000`). The active set is also unbalanced: 4 rules invoke `process_liquidation` / `clean_bad_debt_standalone` (the heaviest entry points in the contract), while the cheap question — "does the *predicate* hold?" — has no dedicated focused rule. Splitting heavy multi-step rules into per-leg rules at single-asset / single-account granularity is the highest-leverage change.

---

## Efficiency rubric (per-rule)

Legend per rubric item:
- **N**: nondets (count, bounded?)
- **S**: state symbolic? (account-shape, map-shape)
- **A**: single-asset focus?
- **L**: unbounded loops over position maps?
- **P**: alignment with new pool summaries?
- **F**: action-focused vs state-invariant
- **B**: decision branches (bad-debt path / normal / refund)
- **R**: storage read budget
- **V**: actual verification value (post-summary)

---

### `hf_improves_after_liquidation` — `liquidation_rules.rs:32-64`

| Item | Status |
|---|---|
| **N** | 4 (`liquidator: Address`, `account_id: u64`, `debt_asset: Address`, `debt_amount: i128`); only `debt_amount > 0` bound. `debt_amount` can reach `i128::MAX` |
| **S** | Fully symbolic: `account_id` indexes any account in storage; `account.supply_positions`, `account.borrow_positions` are unbounded maps |
| **A** | No — the `payments` Vec has 1 entry but `process_liquidation` then loops over (a) `debt_payment_plan.iter()` (line 49), (b) `merged.len()` inside `calculate_repayment_amounts` (line 246), (c) `account.supply_positions.iter()` inside `calculate_seized_collateral` (line 328), (d) `repaid.len()` (line 105), (e) `seized.len()` (line 132), (f) the `borrow_positions` loop in `calculate_health_factor` (line 87) at both ends |
| **L** | Six unbounded loops (above). With `loop_iter: 12` and 4-asset position maps, that is 12 unrollings per loop the prover has to chase symbolically |
| **P** | Only partially — pool summaries (`repay_summary`, `withdraw_summary`, `seize_position_summary` in `summaries/pool.rs`) are **not yet wired** at the production call sites (`HANDOFF.md:126` lists this as Pending). Until wired, every iteration of `apply_liquidation_repayments` triggers a real `LiquidityPoolClient::new(...).repay()` cross-contract call — pure havoc, but the controller still pays for the call-site setup. When the summaries are wired, each Vec iteration still drains `loop_iter` budget |
| **F** | Action-focused (calls `process_liquidation`) — heavy |
| **B** | Three decision branches inside the call: bad-debt cleanup (line 81), `process_excess_payment` while-loop (line 385), `seize_position` Deposit/Borrow split |
| **R** | Reads ~6 storage cells per asset × up to 4 assets, plus `cached_*` accessors for pool address / market index / asset config / price feed (4 cache slots × 4 assets = 16 cached lookups) |
| **V** | Per the soundness audit (Finding 2.1, line 24), the post-condition is currently **vacuous** because both `hf_before` and `hf_after` are independent nondets returned by `calculate_health_factor_for_summary`. So the rule pays full price for *no* verification value |

**Verdict: TAC budget canary, zero verification value.** Likely to time out **or** falsely pass on independent-nondet inputs.

---

### `bonus_bounded` — `liquidation_rules.rs:80-95`

| Item | Status |
|---|---|
| **N** | 3 (`hf_wad`, `base_bonus_bps`, `max_bonus_bps`); all bounded |
| **S** | None — pure-function rule, no storage access |
| **A** | Trivially yes (no asset) |
| **L** | None |
| **P** | Calls `helpers::calculate_linear_bonus`, which is summarized at `summaries/mod.rs:209-219`. The summary already enforces the bound; the assertion is therefore a property of the summary, not production. (See soundness audit Finding 2.2) |
| **F** | State-invariant on a pure function |
| **B** | None |
| **R** | Zero |
| **V** | Under the summary, near-tautology. **Cheap to run** but verifies the abstraction, not the code. Counterpart `boundary_rules.rs::bonus_at_hf_exactly_102` (line 258) calls `calculate_linear_bonus` against a concrete `hf=1.02 WAD` and asserts `bonus == base ± 1` — that one *is* load-bearing because the equality is sharper than the summary allows |

**Verdict: cheap, low-value.** Keep as fast smoke check but flag as summary-trusting.

---

### `bonus_max_at_deep_underwater` — `liquidation_rules.rs:111-133`

| Item | Status |
|---|---|
| **N** | 2 (`base_bonus_bps`, `max_bonus_bps`); bounded |
| **S** | None |
| **A** | Yes (no asset) |
| **L** | None |
| **P** | Same problem as `bonus_bounded` — under the active `calculate_linear_bonus_summary`, the return value is `nondet ∈ [base, max]`. The rule asserts `==` equality with `max_bonus`, which the summary cannot satisfy. **The rule is unprovable as written** (soundness audit Finding 2.3 high) |
| **F** | State-invariant |
| **B** | None |
| **R** | Zero |
| **V** | Negative — the prover wastes work finding the counterexample (or worse, a misconfiguration silently passes) |

**Verdict: cheap-to-prove-impossible.** Will produce CEX every run unless the summary is bypassed (`calculate_linear_bonus_with_target` is unsummarized, line 194 of `helpers/mod.rs`).

---

### `seizure_proportional` — `liquidation_rules.rs:142-174`

| Item | Status |
|---|---|
| **N** | 3 (`total_seizure`, `asset_a_value`, `asset_b_value`); bounded by `> 0` and `<= total_collateral` |
| **S** | None — fully synthetic |
| **A** | Two assets only (production allows 4) |
| **L** | None — straight-line arithmetic |
| **P** | N/A — does not call any production function |
| **F** | Local arithmetic re-derivation (does NOT call `calculate_seized_collateral`) |
| **B** | One conditional (`if asset_a > asset_b`) |
| **R** | Zero |
| **V** | Verifies properties of `mul_div_half_up`, not of liquidation seizure (soundness Finding 2.4 medium). **Four `mul_div_half_up` calls** = four I256 ops in the TAC, which is moderate but bounded |

**Verdict: cheap, low-value.** Drops in <30s; doesn't catch real bugs in `calculate_seized_collateral`.

---

### `protocol_fee_on_bonus_only` — `liquidation_rules.rs:193-227`

| Item | Status |
|---|---|
| **N** | 3 (`seizure_amount`, `bonus_bps`, `liquidation_fees_bps`); bounded |
| **S** | None |
| **A** | Synthetic (no asset) |
| **L** | None |
| **P** | N/A — does not call production |
| **F** | Local arithmetic re-derivation, plus the `mul_div_half_up` vs `div_floor` formula drift bug from soundness audit Finding 2.5 |
| **B** | Two conditionals (`fees_bps == 0`, plus implicit) |
| **R** | Zero |
| **V** | Three I256 ops (`mul_div_half_up + mul_div_floor + mul_div_half_up`). Verifies a separate formula from production (soundness audit High #5) |

**Verdict: cheap, low-value, soundness-broken.** Even after fixing the `div_floor` mismatch, it remains a re-implementation rule.

---

### `bad_debt_threshold` — `liquidation_rules.rs:245-277`

| Item | Status |
|---|---|
| **N** | 1 (`account_id`); fully symbolic |
| **S** | Fully symbolic — reads `storage::get_account(account_id)` and calls `calculate_account_totals` over the (unbounded) supply/borrow maps |
| **A** | No — iterates `account.supply_positions` and `account.borrow_positions` |
| **L** | Two unbounded loops in `calculate_account_totals` (`supply.iter()` + `borrow.iter()`) and again inside `clean_bad_debt_standalone` → `execute_bad_debt_cleanup` which iterates **both** `supply_positions.keys()` and `borrow_positions.keys()` |
| **P** | Pool summaries unwired: `seize_pool_position` reaches into a real `LiquidityPoolClient::new(...).seize_position(...)` per asset. With `seize_position_summary` (lines 294-308) wired, each iteration still drains `loop_iter` budget |
| **F** | Action-focused (calls `clean_bad_debt_standalone`) — **heaviest entry point** in the spec because it iterates supply, then borrow, doing isolation-debt updates and pool seizures |
| **B** | Three branches inside: empty-borrows panic, qualification panic, then per-asset deposit-or-borrow seize branch |
| **R** | One `get_account` (3 storage cells) + `calculate_account_totals` (4 cached lookups × up to 4 supply assets + 4 borrow assets = 32 cache hits) + per-asset `seize_position` call data |
| **V** | The post-condition is identical to the predicate at `liquidation.rs:478` — assert a tautology of "if this branch was taken, the gating condition held". The ONLY non-tautological piece is "the call did not panic" (i.e., reached the assertion), which can be tested with a much smaller rule. **All the heavy iteration is wasted work** |

**Verdict: TAC budget canary.** Single most expensive rule in the file. The verification value can be obtained by a 5-line predicate-only rule (see proposed action-focused rewrites below).

---

### `bad_debt_supply_index_decreases` — `liquidation_rules.rs:287-306`

| Item | Status |
|---|---|
| **N** | 1 (`account_id`) + the implicit `current_contract_address()` |
| **S** | Fully symbolic |
| **A** | Single-asset (the contract's own address) — but supports the wrong question. The supply index lives in the pool, not the controller (soundness Finding 2.7 high). The controller-side `storage::market_index::get_market_index` (`storage/certora.rs:105`) reads via `LiquidityPoolClient::new(...).get_sync_data()` — a cross-contract havoc. So `index_before` and `index_after` are independent nondets, identical situation to `hf_improves_after_liquidation` |
| **L** | Same as `bad_debt_threshold` (calls `clean_bad_debt_standalone`) |
| **P** | The cross-contract havoc means the assertion is always satisfied by the solver picking equal nondets. Wiring `get_sync_data_summary` (line 343) doesn't help: the summary still returns independent nondets per call |
| **F** | Action-focused |
| **B** | Same as `bad_debt_threshold` |
| **R** | Same as `bad_debt_threshold` plus 2 cross-contract `get_sync_data` calls (each fully havoced) |
| **V** | Zero — the relational invariant "index decreased" cannot be observed from the controller spec |

**Verdict: TAC budget canary, zero verification value.** Identical heavy cost to `bad_debt_threshold` for an unobservable property.

---

### `ideal_repayment_targets_102` — `liquidation_rules.rs:323-370`

| Item | Status |
|---|---|
| **N** | 5 (`total_debt`, `weighted_collateral`, `hf`, `base_bonus`, `max_bonus`); all bounded; `total_debt <= 1M*WAD` cap (line 333) |
| **S** | None — synthetic helper invocation, no storage |
| **A** | Synthetic, no asset |
| **L** | None |
| **P** | Calls `estimate_liquidation_amount` (`helpers/mod.rs:234`), which is **NOT** summarized — it executes the full two-target fallback math with `try_liquidation_at_target`, `calculate_post_liquidation_hf`, and several I256 ops. Loop-free but I256-heavy |
| **F** | State-invariant on a pure helper |
| **B** | Several internal branches inside `estimate_liquidation_amount` (primary target / fallback target / unrecoverable path) — visible to the prover as path-explosion fan-out |
| **R** | Zero |
| **V** | Misnamed — does not actually assert HF lands at 1.02 (soundness Finding 2.8 medium) — but the bounds it does assert are real properties of the pure function |

**Verdict: moderate cost, moderate value.** The cap at $1M and the synthetic shape keep it tractable. The fallback-path branching (3 paths) plus several `mul_div` I256 ops likely fits inside `-maxBlockCount 300000` but pushes against it.

---

### `liquidation_bonus_sanity` — `liquidation_rules.rs:377-392`

| Item | Status |
|---|---|
| **N** | 3, bounded |
| **S** | None |
| **L** | None |
| **V** | Pure reachability (`cvlr_satisfy!`); summary makes it trivially satisfiable |

**Verdict: cheap, fine.**

---

### `estimate_liquidation_sanity` — `liquidation_rules.rs:394-414`

| Item | Status |
|---|---|
| **N** | 3, bounded |
| **S** | None |
| **L** | None |
| **V** | Reachability check on the unsummarized `estimate_liquidation_amount`. Hardcoded `proportion_seized = WAD/2`, `total_collateral = total_debt`, `base = 500`, `max = 1000` — all of which fix the path through the function. Fast |

**Verdict: cheap, fine.**

---

## Cross-cutting observations

### 1. Heavy/light split is wrong

The four heaviest rules (`hf_improves_after_liquidation`, `bad_debt_threshold`, `bad_debt_supply_index_decreases`, plus the action-focused parts implicit in `seizure_proportional` if it were ever rewritten to call production) all chain at least **two** unbounded map iterations. With `loop_iter: 12` and 4-position caps, each iteration becomes 12 symbolic copies — and `process_liquidation` chains six such loops back-to-back. The prover's TAC graph blows up multiplicatively.

The four heaviest rules and the four lightest re-implementation rules (`bonus_bounded`, `seizure_proportional`, `protocol_fee_on_bonus_only`, `bonus_max_at_deep_underwater`) sit at opposite extremes: either fully symbolic / multi-loop / action-focused, or fully synthetic / loop-free / property-on-local-arithmetic. There is no middle tier of "single-asset, single-account, one-loop-iteration" rules that exercise production but stay small.

### 2. Pool summaries not yet wired

`HANDOFF.md:126` flags this as Pending. Currently `apply_liquidation_repayments` at line 113 calls `repay::execute_repayment`, which calls `LiquidityPoolClient::new(...).repay(...)` — a cross-contract havoc. The prover treats every field of the returned `PoolPositionMutation` as fully independent nondet. So:
- `result.actual_amount` is unconstrained → could be > `amount`, breaking soundness reasoning
- `result.market_index.supply_index_ray` could violate `>= SUPPLY_INDEX_FLOOR_RAW`
- `result.position.scaled_amount_ray` could go *up* on a repay

These violations don't help proves, they help **disprove** rules — meaning the rules pass for the wrong reason (vacuous CEX path is unreachable in the prover's model).

Wiring the summaries (a) tightens the post-conditions to production-actual bounds, (b) still iterates the Vec symbolically. Net effect: rules become sound but stay slow. **Wiring is necessary but not sufficient for efficiency.**

### 3. Vec iteration over symbolic-length `repaid` / `seized`

`apply_liquidation_repayments` (line 105) and `apply_liquidation_seizures` (line 132) loop `0..repaid.len()` and `0..seized.len()` respectively. The lengths are determined by `execute_liquidation` → `calculate_repayment_amounts` (loops `0..merged.len()`) and `calculate_seized_collateral` (loops `account.supply_positions.iter()`). Under symbolic execution with 4-asset caps and `loop_iter: 12`, each of these turns into 12 `Vec::get(i)` calls, each with `unwrap()` decision branches. **This is the proptest-style canary the prompt warns about.** Restricting these to single-entry would cut TAC commands by 11× per loop.

### 4. Predicate vs. action conflation in `bad_debt_threshold`

The current rule (lines 245-277) calls the entire `clean_bad_debt_standalone` flow just to reach the post-state where the gating predicate must have held. The interesting *predicate* — `total_debt > total_collateral && total_collateral <= BAD_DEBT_USD_THRESHOLD` — is a 2-comparison boolean. A rule that captures the predicate alone (no entry-point invocation, no map iteration, no pool seize) delivers the same coverage at <1% of the cost.

### 5. `hf_improves_after_liquidation` cannot be salvaged at current granularity

Even after fixing the soundness bug (relational ghost or unsummarized HF), the rule still chains `get_account` + `process_liquidation` + `calculate_health_factor_for` × 2. The three position-map iterations alone push against the budget. The right granularity is **per-leg**: one rule for the repayment-leg debt-decrease, one for the seizure-leg collateral-decrease, one for the bad-debt branch.

---

## Proposed action-focused rule-set

Each proposed rule below is bounded to **one asset, one account, one step** and avoids the unbounded-loop trap. Together they cover the same surface as the current 9 rules but at an order of magnitude lower TAC cost.

### Cheap pure-math tier (add four)

These should run in seconds each; they replace the four re-implementation rules with rules that actually call production.

#### `R1: linear_bonus_monotone_in_hf`
- Calls **`calculate_linear_bonus_with_target`** directly (unsummarized at `helpers/mod.rs:194`).
- Two nondets `hf1 < hf2`, fixed `base`, `max`, `target`. Asserts `bonus(hf1) >= bonus(hf2)` (monotone-decreasing in HF).
- Catches: sign flip, base/max swap, target wrong constant.
- Cost: 2 calls × pure-math body ≈ <30s.

#### `R2: linear_bonus_at_target_returns_base`
- Calls `calculate_linear_bonus_with_target` with `hf >= target`. Asserts `bonus == base`.
- Already exists at `boundary_rules.rs::bonus_at_hf_exactly_102` for the `hf == 1.02` case; replicate for `hf > 1.02` to widen coverage.
- Cost: 1 call ≈ <10s.

#### `R3: protocol_fee_matches_production`
- Synthetic inputs (`seizure_amount`, `bonus_bps`, `fees_bps`).
- Compute `base = mul_div_floor(seizure, WAD, one_plus_bonus)` (matches `liquidation.rs:359`).
- Compute `expected_fee = Bps::from_raw(fees_bps).apply_to(env, seizure - base)` (matches `liquidation.rs:362`).
- Assert `expected_fee >= 0`, `expected_fee <= bonus_amount`, and (most importantly) that `expected_fee` matches what `Bps::apply_to` returns — catches drift between the rule and `apply_to` semantics.
- Cost: 3 fp_core ops ≈ <30s.

#### `R4: seizure_two_assets_proportional_against_production`
- Build a 2-asset `supply_positions` Map and call `calculate_seized_collateral` directly.
- Assert each entry `.amount <= actual_amount` (the production cap at `liquidation.rs:357`).
- Assert the per-asset value share matches `(asset_value / total_collateral)` within rounding.
- Bounded: 2 assets, fixed bonus, fixed price.
- Cost: 1 call to `calculate_seized_collateral` with a 2-entry map ≈ <2 min.

### Single-asset action tier (add four; bound `loop_iter` to 1)

These call production but pin map sizes to 1 so the iterations don't multiply.

#### `R5: hf_improves_single_asset_full_repay`
- One supply asset, one borrow asset, both nondet positions. `payments` Vec has 1 entry with `amount >= debt_outstanding` (forces full repayment).
- Calls `process_liquidation` directly.
- Asserts: post-state `account.borrow_positions.is_empty()` (full close path).
- Use a **`process_liquidation`-only conf** with `loop_iter: 1` (since position maps have 1 entry each).
- Cost: 1 iteration × every loop ≈ tractable in 5-10 min once pool summaries are wired.

#### `R6: hf_improves_single_asset_partial_repay`
- Same shape as R5 but `payment.amount` is a tiny fraction. Asserts post-state HF strictly increased (relational ghost OR unsummarized HF call at both ends — see soundness audit High #2).
- This is the rule `hf_improves_after_liquidation` *should* be.

#### `R7: bad_debt_predicate_only`
- Replaces the heavyweight `bad_debt_threshold` rule.
- Synthetic: two i128s `total_collateral_usd`, `total_debt_usd`. No map, no `get_account`, no `clean_bad_debt_standalone` call.
- Body:
  ```rust
  let qualifies =
      total_debt_usd > total_collateral_usd
      && total_collateral_usd <= BAD_DEBT_USD_THRESHOLD;
  cvlr_assert!(qualifies == /* recompute */);
  ```
- Wait — that's still a tautology. **Do NOT do that.** The right form is a "panic-or-not" rule: assume the negation of the predicate, call `clean_bad_debt_standalone`, assert unreachable (under panic semantics it must have panicked). But that path needs the entry-point call.
- **Better: skip predicate-only and use R8 (boundary panic check) which is what `boundary_rules.rs::bad_debt_at_6_usd` already does well at line 323.** Mark `bad_debt_threshold` for deletion (already covered).

#### `R8: bad_debt_panics_when_unqualified`
- Mirrors `boundary_rules.rs::bad_debt_at_6_usd` pattern: assume `total_collateral_usd == 6 * WAD` and `total_debt_usd > 6 * WAD`, call `clean_bad_debt_standalone`, assert that the call reverted (`expected_panic` or equivalent).
- Cost: same as `boundary_rules.rs::bad_debt_at_6_usd` (already passes per existing conf).

#### `R9: seizure_amount_capped_at_actual`
- One supply asset with `actual_amount = K`. Build a synthetic `account` with this position. Call `calculate_seized_collateral` with a `repayment_usd` large enough to want more than `K`.
- Assert: returned `entry.amount <= K`.
- Catches the cap removal bug (no equivalent rule exists today). See soundness audit missing-invariant #9.
- Cost: 1 supply-position iteration ≈ <2 min.

### Cross-cutting tier (one rule for the entire file)

#### `R10: process_liquidation_reverts_when_hf_above_one`
- Empty payment Vec rejected at line 45; non-empty Vec but pre-state HF == WAD: `process_liquidation` must panic at line 165.
- Already partially in `boundary_rules.rs::liquidation_at_hf_exactly_one` (line 213) but that asserts only the predicate post-`calculate_health_factor_for`, not that `process_liquidation` reverts. Wire a panic-asserting variant.

### Rules to delete / move

| Rule | Action | Reason |
|---|---|---|
| `hf_improves_after_liquidation` | Replace with R5+R6 | Vacuous under summary AND TAC canary |
| `bonus_bounded` | Keep, mark "summary-trusting" | Cheap; flag in comment |
| `bonus_max_at_deep_underwater` | Delete | Unprovable under active summary |
| `seizure_proportional` | Replace with R4 | Re-impl rule with no production link |
| `protocol_fee_on_bonus_only` | Replace with R3 | Formula drift bug; re-impl |
| `bad_debt_threshold` | Delete | Heavy AND tautological; covered by `boundary_rules` |
| `bad_debt_supply_index_decreases` | Move to pool spec | Cross-contract observation impossible from controller |
| `ideal_repayment_targets_102` | Keep, strengthen | Tractable; assert post-HF lands near 1.02 (soundness Finding 2.8) |
| `liquidation_bonus_sanity` | Keep | Cheap reachability |
| `estimate_liquidation_sanity` | Keep | Cheap reachability |

Net change: delete 5 (or move), add 6, keep 4 → 10 rules total, but the heavy cost is concentrated in 2 rules (R5+R6) instead of 4.

---

## Severity-tagged actions

### High (blocking; will cause timeouts or TAC overflow at current `-maxBlockCount 300000`)

1. **Delete `bad_debt_supply_index_decreases`** (`liquidation_rules.rs:287-306`) from `confs/liquidation.conf`. Calls `clean_bad_debt_standalone` (the heaviest entry point: 2-loop totals + per-asset seize loop + isolation update) for an assertion that is **always trivially satisfied** by the cross-contract havoc. Expected savings: ~1/3 of the conf's wall time.

2. **Replace `hf_improves_after_liquidation`** (`liquidation_rules.rs:32-64`) with **R5 (full-repay close)** and **R6 (partial-repay HF up)**, each with `account.supply_positions` and `account.borrow_positions` of size 1. Set `loop_iter: 1` for these rules in a dedicated sub-conf. The current rule chains six unbounded loops × `loop_iter: 12` each — an immediate canary.

3. **Delete `bad_debt_threshold`** (`liquidation_rules.rs:245-277`). It's a tautology AND the heaviest entry-point invocation. The same coverage is in `boundary_rules.rs::bad_debt_at_exactly_5_usd` (line 295) and `bad_debt_at_6_usd` (line 323), both already in the boundary conf and tractable.

4. **Wire pool summaries before re-running the file.** Until `repay_summary`, `withdraw_summary`, `seize_position_summary`, `claim_revenue_summary` are wrapped at production sites via `apply_summary!` (HANDOFF.md:126), every `process_liquidation` rule pays full price for cross-contract havoc *and* gets weaker post-conditions. Wiring is a precondition for R5/R6 being tractable.

### Medium

5. **Delete `bonus_max_at_deep_underwater`** (`liquidation_rules.rs:111-133`). Under the active `calculate_linear_bonus_summary` (`summaries/mod.rs:209-219`) the equality at line 132 is unprovable. Either delete or rewrite to call `calculate_linear_bonus_with_target` directly (the unsummarized helper at `helpers/mod.rs:194`) — at that point it becomes equivalent to `boundary_rules.rs::bonus_at_hf_exactly_102` extended to deep-underwater HF.

6. **Replace `seizure_proportional`** (`liquidation_rules.rs:142-174`) with **R4 (call `calculate_seized_collateral` directly with a 2-entry supply Map)**. Add **R9 (per-asset cap at `actual_amount`)** while you're there — this fills missing-invariant #9 from the soundness audit at near-zero marginal cost.

7. **Replace `protocol_fee_on_bonus_only`** (`liquidation_rules.rs:193-227`) with **R3 (call `Bps::apply_to` against `liquidation.rs:357-363` reference math)**. Even after fixing `mul_div_half_up` → `div_floor` to match production, the rule remains a re-derivation. Calling production primitives is cheaper *and* sounder.

8. **Add a dedicated `liquidation-light.conf`** with `loop_iter: 1` for the new R3/R4/R9 single-step rules. The existing `loop_iter: 12` is needed only for R5/R6 if the position maps are not pinned to size 1.

### Low

9. **Tighten input bounds on `hf_improves_after_liquidation` (R5/R6) `debt_amount`.** The current rule has only `debt_amount > 0` (`liquidation_rules.rs:40`), which lets the prover pick `i128::MAX` and hit overflow paths irrelevant to liquidation correctness. Cap at `1_000_000 * WAD * 10^7` (matches the `1M USD * 7-decimal token` realistic protocol ceiling).

10. **`ideal_repayment_targets_102`**: assert post-HF lands near 1.02 to make the rule match its name (soundness Finding 2.8). The cost is one extra `calculate_post_liquidation_hf` call — already a pure helper, no extra loops.

11. **Mark `bonus_bounded` with `// summary-trusting` comment** so future readers don't mistake it for a production-verifying rule. It's cheap; keep it as a smoke check but be honest about what it covers.

12. **Verify `liquidation.conf` `-maxBlockCount 300000` is the right ceiling once R5/R6 land.** Heavy single-asset rules with pool summaries wired likely fit inside 200k; deletion of the 3 high-severity rules above should drop the conf-wide ceiling. Re-tune after the first clean run.

---

## Estimated TAC budget (informal, post-changes)

| Rule | Current TAC weight | Post-change TAC weight |
|---|---|---|
| `hf_improves_after_liquidation` | ~6 unbounded loops × 12 unrolls + 2 cross-contract havocs | R5+R6 each: 1-asset pinned, summarized pool ≈ 1/12 |
| `bad_debt_threshold` | 2 totals loops + per-asset seize loop | Deleted |
| `bad_debt_supply_index_decreases` | Same as above + 2 cross-contract reads | Deleted |
| `bonus_max_at_deep_underwater` | 0 (CEX immediate) | Deleted |
| `seizure_proportional` | 4 fp_core ops | R4: 1 production call, 2-asset map ≈ 5× heavier but real |
| `protocol_fee_on_bonus_only` | 3 fp_core ops | R3: 3 fp_core ops + 1 `Bps::apply_to` ≈ same |
| `ideal_repayment_targets_102` | 1 `estimate_liquidation_amount` | +1 `calculate_post_liquidation_hf` |

Net: drop the three TAC canaries (~70% of conf wall time today), pay back ~30% in R4/R5/R6 with sound coverage. Conf should fit comfortably inside `-maxBlockCount 200000` after the changes.
