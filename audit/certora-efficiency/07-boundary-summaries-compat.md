# Domain 7 — Boundaries, Summaries, Compat (Certora framework efficiency)

**Phase:** Certora formal-verification meta-review (efficiency-focused, post-P0/P1 remediation)
**Audit-frozen tip:** `692932c` (P0+P1 remediation) on top of `6e021fb` (initial summarisation pass)

**Files in scope:**
- `/Users/mihaieremia/GitHub/rs-lending-xlm/controller/certora/spec/boundary_rules.rs` (631 lines, 38 rule fns: 19 assertion rules + 19 sanity satisfies)
- `/Users/mihaieremia/GitHub/rs-lending-xlm/controller/certora/spec/summaries/mod.rs` (254 lines, 9 summaries)
- `/Users/mihaieremia/GitHub/rs-lending-xlm/controller/certora/spec/summaries/pool.rs` (404 lines, 12 pool summaries — *inert; never wired*)
- `/Users/mihaieremia/GitHub/rs-lending-xlm/controller/certora/spec/summaries/sac.rs` (96 lines, 4 SAC summaries — *inert; never wired*)
- `/Users/mihaieremia/GitHub/rs-lending-xlm/controller/certora/spec/compat.rs` (127 lines, 6 entry-point shims)
- `/Users/mihaieremia/GitHub/rs-lending-xlm/controller/certora/spec/mod.rs` (32 lines, module registration)
- `/Users/mihaieremia/GitHub/rs-lending-xlm/controller/certora/HANDOFF.md` (174 lines, engagement-team handoff)

**Production references:**
- `/Users/mihaieremia/GitHub/rs-lending-xlm/controller/src/lib.rs:10-26` (`summarized!` macro definition)
- `/Users/mihaieremia/GitHub/rs-lending-xlm/controller/src/views.rs:111, 143, 260` (active `summarized!` sites)
- `/Users/mihaieremia/GitHub/rs-lending-xlm/controller/src/oracle/mod.rs:25, 372, 492` (active `summarized!` sites)
- `/Users/mihaieremia/GitHub/rs-lending-xlm/controller/src/helpers/mod.rs:55, 118, 139, 222` (active `summarized!` sites)
- `/Users/mihaieremia/GitHub/rs-lending-xlm/controller/src/positions/{supply,borrow,withdraw,repay,liquidation}.rs` (un-wrapped pool/SAC call sites)
- `/Users/mihaieremia/GitHub/rs-lending-xlm/pool/src/lib.rs:132-749` (production pool ABI)
- `/Users/mihaieremia/GitHub/rs-lending-xlm/vendor/cvlr-soroban/cvlr-soroban-macros/src/apply_summary.rs:1-88` (macro semantics)

**Totals:** broken = 3   weak = 9   redundant = 12   inert = 16   nit = 4

---

## Summary verdict

The framework layer has improved meaningfully since the P0+P1 commit (`692932c`): the tuple-order bug in `calculate_account_totals_summary` is fixed, the false `borrow_index >= supply_index` over-constraint is removed, and the HF summary now ties its output to observable input shape (`empty borrows -> i128::MAX`, `empty supply with debt -> 0`).

But the framework has three structural efficiency issues that, taken together, account for most of the lost prover budget:

1. **boundary_rules.rs is ~50% redundant prover load.** Of 19 assertion rules, **5 are strict-stronger duplicates** of `interest_rules` / `math_rules` (rules 1-5: every `borrow_rate_at_exact_*` is subsumed by `borrow_rate_zero_utilization` + `borrow_rate_continuity_at_*` over nondet params). **6 more are pure logical tautologies** (rules 15-17, 19, 20: `cvlr_assume!(x == y)` then `cvlr_assert!(x == y)` re-encoded as predicate). **2 are vacuously refutable** under the input-tied HF summary (rules 6-7: `cvlr_assume!(hf == WAD)` is consistent only in the unconstrained branch; the assertion is then a tautology). The prover spends budget on each one regardless. Net: only 6 of 19 assertion rules give the engagement team signal that no other rule does.

2. **The 12 pool.rs + 4 sac.rs summaries are entirely inert.** `apply_summary!` (`vendor/cvlr-soroban-macros/src/apply_summary.rs:11-33`) wraps **function definitions in the controller crate**. The pool and SAC ABIs are accessed through `pool_interface::LiquidityPoolClient` and `soroban_sdk::token::Client` — generated client wrappers, not controller-defined functions. The 16 newly authored summaries cannot be reached by the macro from any current site. There is no `summarized!(pool::supply_summary, fn apply_pool_supply(...))` wiring (and nothing else either) in `controller/src/positions/{supply,borrow,withdraw,repay}.rs`, `controller/src/cache/mod.rs:152`, `controller/src/router.rs:233-381`, `controller/src/utils.rs:38-41`, or `controller/src/flash_loan.rs:52-62`. **Until wired, every domain rule that traverses a pool or SAC call sees pure prover havoc** — exactly the state HANDOFF.md flags as Pending at line 126. The summaries' contracts are correct (well-typed, sound bounds derived from production line ranges); they just don't change any prover verdict today. Cost: ~700 lines of spec authored that don't help any rule until wrapper functions are added at the call sites.

3. **Compat shim cost is acceptable as authored, but the parameterisation is one-sided.** `compat::multiply` (`compat.rs:41-92`) havocs `account_id`, `initial_payment`, and `convert_steps` to keep both create-vs-load and with-vs-without-initial-payment branches reachable. That is the correct tightness for the rules that already exist (8 callers in `strategy_rules.rs`, all of which want full coverage). The cost is 3 nondets per call → 8 nondet draws × ~per-rule prover work; this is small relative to the full-strategy state-space already in scope. Not a finding by itself, **but**: there are no tight-scope alternates (`multiply_simple` with `account_id = 0`, `initial_payment = None`, `convert_steps = None`) for rules that *only* need the happy path. The fast-prove "supply increases scaled balance after multiply create-new" property pays for the full branching cost even though it doesn't depend on initial_payment or convert_steps.

The boundary-rule redundancy is the highest-leverage cleanup target: it removes prover work without removing coverage. The pool/SAC inert summaries are the highest-leverage *coverage* gap: every property in liquidation, solvency, flash-loan, and strategy rules is silently weakened by the missing wiring, and the audit is at risk of producing pass-by-vacuity verdicts on rules that look strong on the page.

---

## Boundary rules — efficiency map

`boundary_rules.rs` has 19 assertion rules + 19 sanity rules. The sanity rules are reachability checks (`cvlr_satisfy!`) — they cost prover budget but they are intentional and not flagged here.

### Redundant against domain rules (5 rules, ~13% of file)

These rules are *strictly weaker* than an existing domain rule. Removing them eliminates duplicate prover work with zero coverage loss.

| # | Boundary rule (line) | Subsumed by | Why redundant |
|---|---|---|---|
| 1 | `borrow_rate_at_exact_zero` (60) | `interest_rules::borrow_rate_zero_utilization:80` | Both call `calculate_borrow_rate(_, Ray::ZERO, params)` and assert `rate == base/MS_PER_YEAR`. Boundary uses one fixed `boundary_test_params(env)`; interest uses fully nondet `nondet_valid_params(e)`. Universal-quantifier nondet **strictly dominates** existential fixed-params. The boundary rule cannot fail in any model where the interest rule passes. |
| 2 | `borrow_rate_at_exact_mid` (85) | `interest_rules::borrow_rate_continuity_at_mid:149` | Continuity rule already pins both `mid - 1` and `mid` and bounds the gap by 1; boundary rule pins only `mid` against the same expected value. Same dominance argument. |
| 3 | `borrow_rate_at_exact_optimal` (117) | `interest_rules::borrow_rate_continuity_at_optimal:178` | Same as #2. |
| 4 | `borrow_rate_at_100_percent` (147) | `interest_rules::borrow_rate_capped:126` | Capped rule asserts `rate <= cap + 1` for any utilization in `[0, RAY]`; boundary rule asserts the same at `Ray::ONE`. Capped rule strictly dominates. |
| 5 | `compound_interest_at_max_rate_max_time` (171) | `interest_rules::compound_interest_monotonic_in_time:277` + `compound_interest_ge_simple:332` | Boundary asserts `factor > RAY && factor < 100*RAY` at one rate/time pair; the two interest rules over nondet rate/time imply the same property. Sanity at line 187 is fine; the assertion at 171 is redundant. |

**Recommendation:** delete these 5 rules and their sanity twins (10 fns total). Net spec-line reduction: ~140 lines. Net prover-budget saving: 5 SMT queries the prover currently runs that overlap entirely with `interest_rules`.

### Pure logical tautologies (6 rules, ~16% of file)

These rules construct a local predicate, locally assume the inputs into a state where the predicate trivially holds, then assert the predicate. They prove nothing about the protocol. They are uniformly the cheapest queries the prover sees (linear arithmetic over nondets) but they consume submission budget and pollute the verdict report.

| # | Boundary rule (line) | Tautology |
|---|---|---|
| 15 | `tolerance_at_exact_first_bound` (450) | `cvlr_assume!(deviation == first_tolerance)` then `cvlr_assert!(deviation <= first_tolerance)`. Pure assumption-implies-assertion. No call to `oracle::is_within_anchor`. |
| 16 | `tolerance_at_exact_second_bound` (483) | Same shape: `deviation == second_tolerance` ⇒ `deviation > first_tolerance && deviation <= second_tolerance` (uses the prior assumption that `second > first`). |
| 17 | `tolerance_just_beyond_second` (515) | `deviation == second_tolerance + 1` ⇒ `deviation > second_tolerance`. Trivial. |
| 18 | `supply_dust_amount` (553) | Calls `mul_div_half_up(_, 1, RAY, RAY)`. Subsumed by `math_rules::mul_half_up_identity:68` over nondet `a` (which strictly dominates the `a = 1` case). |
| 19 | `borrow_exact_reserves` (578) | `cvlr_assume!(borrow_amount == available_reserves)` then `cvlr_assert!(!(borrow_amount > available_reserves))`. Pure tautology; never invokes `pool::has_reserves` or anything else from production. |
| 20 | `withdraw_more_than_position` (610) | `cvlr_assume!(requested > position_value)`, then `actual = requested.min(position_value)` and asserts `actual == position_value`. Tautology over the `min` operator's definition. |

**Recommendation:** rewrite to invoke production. For example, rule 19 should call `compat::borrow_single` with `amount == available_reserves` (using `cached_pool_sync_data` to discover the reserve) and assert that the call does not panic. Until rewritten, these are noise. HANDOFF.md already classifies them under the "16 tautological rules" bucket, but the post-P0 commit message claims `bad_debt_threshold` was fixed — see next section for whether the boundary-rules version was also rewritten.

### Vacuously refutable under the input-tied HF summary (2 rules)

After the P0 refactor of `calculate_health_factor[_for]_summary` (mod.rs:131-148, 157-172), the summary returns:
- `hf == i128::MAX` if `borrow_positions.is_empty()`
- `hf == 0` if `supply_positions.is_empty() && !borrow_positions.is_empty()`
- `hf >= 0` (otherwise; **fully unconstrained**)

| # | Boundary rule | Behaviour |
|---|---|---|
| 6 | `liquidation_at_hf_exactly_one` (213) | `cvlr_assume!(hf == WAD)`. The `i128::MAX` and `0` branches are infeasible (assumption fails immediately). Only the unconstrained branch survives. Then `cvlr_assert!(hf >= WAD)` is a tautology over the assumption. The rule "passes" but proves the wrong thing — it proves the cvlr_assume was satisfiable, not that production rejects HF == WAD liquidations. The doc comment at line 218 claims it tests "the production guard `if hf >= WAD { panic HealthFactorTooHigh }`" at `liquidation.rs:164`. It does not. |
| 7 | `liquidation_at_hf_just_below_one` (235) | Same shape. `cvlr_assume!(hf == WAD - 1)` then `cvlr_assert!(hf < WAD)` — tautology. |

**Recommendation:** rewrite to call `process_liquidation` and observe the outcome (panic vs success). The current shape proves the cvlr_assume is satisfiable, nothing more.

### Real production-gated rules (6 rules — keep)

These rules call into production and assert against the result of a real computation. They are worth their prover budget.

| # | Rule | Production touched |
|---|---|---|
| 8 | `bonus_at_hf_exactly_102` (258) | Calls `helpers::calculate_linear_bonus` (summarised). The summary returns `bonus ∈ [base, max]` (mod.rs:209-219); the rule asserts `\|bonus - base\| <= 1`. **This rule fails on the prover** — counterexample `bonus = max = 1000` is admitted by the summary. Fixing this requires either (a) tightening the summary at HF == target_hf to return exactly `base` (sound — production line `helpers/mod.rs:202-204` returns `base` when `gap_numerator <= 0`), or (b) un-summarising `calculate_linear_bonus` (it's not actually expensive). |
| 9 | `bad_debt_at_exactly_5_usd` (295) | Calls `views::total_collateral_in_usd` and `total_borrow_in_usd` (both summarised, return `>= 0` nondets). With `cvlr_assume!(total_collateral_usd == 5*WAD)` and `total_debt_usd > total_collateral_usd`, the assertion is then a tautology (predicate is `total_debt > total_collateral && total_collateral <= 5*WAD`, both clauses are direct restatements of the assumptions). **Effectively tautological** despite the production-call ceremony. |
| 10 | `bad_debt_at_6_usd` (323) | Same shape as #9. Tautological. |
| 11 | `mul_at_max_i128` (355) | Calls `mul_div_half_up`; asserts result-near-input identity at `i128::MAX / RAY`. Subsumed by `math_rules::mul_half_up_identity` over nondet `a`, but the boundary case at `i128::MAX / RAY` is not in the `[0, RAY * 1000]` window of the math rule (line 73). **Genuinely additional coverage.** Keep. |
| 12 | `compound_taylor_accuracy` (382) | Calls `compound_interest`; asserts a tight precision band at one specific `(rate, time)`. The math/interest rules give monotonicity and ge-simple but no tight precision band. **Genuinely additional coverage.** Keep. |
| 13 | `rescale_ray_to_wad` (421) | Calls `rescale_half_up(RAY, 27, 18) == WAD`. Subsumed by `math_rules::rescale_upscale_lossless:230` over nondet `x` — but only in the `(7, 18)` direction; the `(27, 18)` downscale is **not** covered there. Keep as a downscale-direction sanity. |
| 14 | `rescale_wad_to_7_decimals` (433) | Calls `rescale_half_up(WAD, 18, 7) == 10^7`. Same: not covered by `math_rules::rescale_roundtrip:260`. Keep. |

Of the 6 "production-gated" rules, **2 are tautological** (#9, #10) and **1 fails on the prover** (#8). Only #11, #12, #13, #14 give the engagement team usable signal.

### Net assessment of `boundary_rules.rs`

- **19 assertion rules.**
- **5 strictly redundant** with domain rules (#1-5).
- **6 pure tautologies** (#15-17, 18, 19, 20).
- **2 vacuously refutable** under HF summary (#6, #7).
- **2 effectively tautological** despite production calls (#9, #10).
- **1 will fail the prover** as written (#8).
- **3 give unique signal** (#11, #12, #13/#14 as a pair).

The file's signal-to-budget ratio is 3-to-19 (~16%). The rest is either redundant or tautological work. HANDOFF.md (line 99-114) already classifies this in aggregate as "16 tautological + 4 vacuous" but does not flag the redundancy with `interest_rules` and `math_rules`, and does not flag #8's prover-fail risk.

---

## Summary contracts (existing) — tightness audit

The summaries listed here are the ones currently wired via `summarized!` in production (verified by `grep -n "summarized!" controller/src/`):
- views.rs:111, 143, 260 → `total_collateral_in_usd_summary`, `total_borrow_in_usd_summary`, `ltv_collateral_in_usd_summary`
- oracle/mod.rs:25, 372, 492 → `token_price_summary`, `is_within_anchor_summary`, `update_asset_index_summary`
- helpers/mod.rs:55, 118, 139, 222 → `calculate_health_factor_summary`, `calculate_health_factor_for_summary`, `calculate_account_totals_summary`, `calculate_linear_bonus_summary`

| Summary | Bound tightness | Cost | Verdict |
|---|---|---|---|
| `token_price_summary` (mod.rs:57-69) | **Tight on price (>0); tight on decimals (≤27); tight on staleness (≤now+60).** | Low (3 cvlr_assume on i128/u32/u64). | Sound. Could add `asset_decimals == cached_market_config(asset).oracle_config.asset_decimals` to make decimals deterministic — current nondet decimals widen the rescaling state space in every downstream rule that converts to wad. **Weak (1)**: rules that destructure `feed.asset_decimals` see all 28 possible values. |
| `is_within_anchor_summary` (76-84) | **Maximally weak (nondet bool).** | Trivial. | Sound but uninformative. Every domain rule that branches on this gets both branches as live; if a rule's invariant happens to hold only in one branch, the rule fails. **Weak (2)**: an oracle rule asserting "first-tier path returns safe price" cannot use this summary's contract. The oracle rules that need first/second tier discrimination already call the unsummarised `crate::oracle::is_within_anchor::is_within_anchor` (preserved by `apply_summary!` at vendor/cvlr-soroban-macros/src/apply_summary.rs:3-10), so this is a non-issue in practice. |
| `update_asset_index_summary` (95-110) | **Tight on monotonicity floors** (`supply >= WAD`, `borrow >= RAY`); the over-constraint `borrow >= supply` was removed by P0 (commit 692932c). | Low. | Sound. Could be tightened with a monotonicity-against-prior bound for non-seize paths, similar to `pool::nondet_market_index_monotone` (pool.rs:55-60). Without this, every rule comparing pre/post indexes around a non-seize operation cannot use the helper. **Missing (1)**: no `update_asset_index_summary_monotone` variant for the non-seize callers in `cache/mod.rs:153`. |
| `calculate_health_factor_summary` (131-148) | **Input-tied: empty borrows → MAX, empty supply with debt → 0, otherwise unconstrained ≥ 0.** P0 refactor. | Low. | Sound but **weak in the central case** (most-rule path). Rules of the form `hf_after >= hf_before` (`health_rules::supply_cannot_decrease_hf:93`, `liquidation_rules::hf_improves_after_liquidation:33`) get two independent nondet draws bounded only by `>= 0`. **Vacuously refutable** for the prover: `hf_before = 100, hf_after = 0` violates the assertion in any state where both maps are non-empty. **Weak (3)**, **broken (1)** depending on framing — the rules will fail under this summary unless they pin the input-state shape (e.g. assume both are non-empty AND the computed value monotonically improves), which they don't. |
| `calculate_health_factor_for_summary` (157-172) | Same as above. | Low. | Same. **Weak (4)**. The `#[cfg(feature = "certora")]`-only nature is fine; the summary contract is the issue. |
| `calculate_account_totals_summary` (180-198) | **Tight on order (collateral, debt, weighted) — fixed by P0 commit 692932c**; tight on `weighted ≤ collateral`. | Low. | **Sound.** Tuple-order fix is correctly applied. Could be tightened with `total_debt >= 0` (already there) plus an upper-bound link to `total_borrow_in_usd_summary` (currently summary returns independent nondet for `total_debt_raw`); without that link, two summaries called in the same rule produce inconsistent USD totals. **Missing (2)**: no cross-summary consistency. |
| `calculate_linear_bonus_summary` (209-219) | `bonus ∈ [base, max]`. | Low. | **Too weak.** Production at HF ≥ target_hf returns *exactly* `base` (helpers/mod.rs:202-204); the summary admits any value in the band. `boundary_rules::bonus_at_hf_exactly_102:258` (rule 8) asserts `\|bonus - base\| ≤ 1` and **will fail the prover** because the summary admits `bonus = max`. **Broken (2)**: tighten by branching on `hf >= target_hf` → return `base`, else nondet in band. |
| `total_collateral_in_usd_summary` (230-234) | `total >= 0`. | Trivial. | **Weak (5).** Production guarantees `== 0` when `try_get_account_meta` is None or supply map is empty (views.rs:114-120). Rules that branch on these zero cases get a nondet ≥ 0. Tighten with `if try_get_account(env, account_id).is_none() || account.supply_positions.is_empty() { cvlr_assume!(total == 0); }`. |
| `total_borrow_in_usd_summary` (237-241) | Same as above. | Trivial. | Same. **Weak (6).** |
| `ltv_collateral_in_usd_summary` (249-253) | `total >= 0`. | Trivial. | **Weak (7).** The doc comment (lines 244-247) correctly identifies the production invariant — "result is bounded by `total_collateral_in_usd`" — but the summary does not encode it. Tighten with `cvlr_assume!(total <= total_collateral_in_usd_summary(env, account_id).raw())` — though this requires careful sequencing because both summaries return independent nondets. The cleaner fix is to share the underlying ghost: introduce a single-assignment ghost `total_collateral_ghost` and have both summaries reference it. |

### Net assessment

- **2 broken** (`calculate_linear_bonus_summary` admits all values in band; HF summary makes hf-improves rules vacuously refutable).
- **7 weak** (acceptable in isolation but lose information that production guarantees).
- **0 unsound over-constraints** (the P0 commit fixed the only one).

The summaries are **strictly less harmful than the un-summarised originals** — that is the design goal — but they do not give downstream rules enough leverage to prove non-trivial properties. Most of the weakness can be tightened without prover budget cost (cvlr_assume on returned scalars is cheap; the cost lives in cross-summary ghost coordination, not per-summary bounds).

---

## Summary wiring status

For each new summary in `pool.rs` and `sac.rs` (authored in commit 692932c), this section answers two questions: (a) is the summary contract correct? (b) is the summary currently active in any rule? **All answers to (b) are NO.**

### `pool.rs` (12 summaries)

| Summary | Production fn (`pool/src/lib.rs:line`) | Active? | Wiring needed | Effect today |
|---|---|---|---|---|
| `supply_summary` (76-93) | `LiquidityPool::supply` (132-162) | **NO** | Wrap `apply_pool_supply` (`controller/src/positions/supply.rs:363-382`) with `crate::summarized!(crate::spec::summaries::pool::supply_summary, fn apply_pool_supply(...) -> SupplyMarketUpdate {...})`. The current `apply_pool_supply` returns `SupplyMarketUpdate {market_index, credited_amount}`, not `PoolPositionMutation` — the summary signature would need to match the wrapper, not the bare ABI. Either (i) change wrapper return type to `PoolPositionMutation` + bookkeeping at the caller, or (ii) write a new `pool_supply_call` wrapper that returns `PoolPositionMutation` directly and gets summarised. | None. Domain rules (`solvency_rules::supply_increases_supplied`, `health_rules::supply_cannot_decrease_hf`, `position_rules::*supply*`) see havoc on the cross-contract result. The summary contract is correct but inert. |
| `borrow_summary` (105-123) | `LiquidityPool::borrow` (164-203) | **NO** | Wrap an analogous wrapper around `pool_client.borrow` (positions/borrow.rs:262-263). | Same as above. `solvency_rules::borrow_decreases_reserves`, `health_rules::hf_safe_after_borrow`, `flash_loan_rules::*` all see havoc. |
| `withdraw_summary` (135-160) | `LiquidityPool::withdraw` (205-285) | **NO** | Wrap around `pool_client.withdraw` at `controller/src/positions/withdraw.rs:136-137`. | Same. |
| `repay_summary` (173-196) | `LiquidityPool::repay` (287-350) | **NO** | Wrap around `pool_client.repay` at `controller/src/positions/repay.rs:104-106`. | Same. |
| `update_indexes_summary` (203-205) | `LiquidityPool::update_indexes` (352-365) | **NO** | Wrap a thin wrapper around the `pool_client.update_indexes(&...)` calls at `controller/src/router.rs:237`, `controller/src/utils.rs:143`, and the `cache/mod.rs:152-153` `pool_client.get_sync_data()` (different fn). | Same — `index_rules::*` reasoning about pre-vs-post indexes sees havoc. |
| `add_rewards_summary` (216) | `LiquidityPool::add_rewards` (367-387) | **NO** | Wrap around `pool_client.add_rewards` at `controller/src/router.rs:324`. | Same. |
| `flash_loan_begin_summary` (227) | `LiquidityPool::flash_loan_begin` (389-413) | **NO** | Wrap around the call at `controller/src/flash_loan.rs:53`. | Same. `flash_loan_rules::*` (4 rules) all see havoc on begin/end semantics. |
| `flash_loan_end_summary` (238) | `LiquidityPool::flash_loan_end` (415-456) | **NO** | Wrap around the call at `controller/src/flash_loan.rs:62`. | Same. |
| `create_strategy_summary` (250-278) | `LiquidityPool::create_strategy` (458-508) | **NO** | Wrap around `pool_client.create_strategy` at `controller/src/positions/borrow.rs:55-56`. | Same. `strategy_rules::*` (20 rules) lose the `amount_received == amount - fee` guarantee. |
| `seize_position_summary` (294-308) | `LiquidityPool::seize_position` (510-545) | **NO** | Wrap around `pool_client.seize_position` at `controller/src/positions/liquidation.rs:525-527`. | Same. `liquidation_rules::*` (10 rules) lose the `position.scaled == 0 after seize` post-condition. |
| `claim_revenue_summary` (319-323) | `LiquidityPool::claim_revenue` (547-600) | **NO** | Wrap around `pool_client.claim_revenue` at `controller/src/router.rs:282`. | Same. |
| `get_sync_data_summary` (343-391) | `LiquidityPool::get_sync_data` (736-749) | **NO** | Wrap the `cached_pool_sync_data` accessor in `controller/src/cache/mod.rs:152-153`, OR add a controller-side wrapper that the cache calls. | Same. Every rule that uses `cache.cached_pool_sync_data` (broadly `solvency_rules`, `interest_rules::supplier_rewards_conservation`) sees the cache return havoc. |

### `sac.rs` (4 summaries)

| Summary | Production fn | Active? | Wiring needed | Effect today |
|---|---|---|---|---|
| `transfer_summary` (41-43) | `soroban_sdk::token::Client::transfer` | **NO** | Wrap `transfer_and_measure_received` (`controller/src/utils.rs:30-52`) with `crate::summarized!`. The summary signature would need to match `transfer_and_measure_received` (returns i128 `received`) rather than the bare SAC `transfer` (no return). Alternatively introduce a thin `pool_token_transfer(env, asset, from, to, amount)` wrapper around `token.transfer` and summarise that. | None. Every transfer in supply/borrow/withdraw/repay/strategy paths sees a havoc'd post-balance, including the 17 `token.transfer` / `token.balance` call sites in `controller/src/strategy.rs:417-919`. |
| `balance_summary` (60-64) | `soroban_sdk::token::Client::balance` | **NO** | Same as above — needs a controller-side `pool_token_balance` wrapper. | Same. |
| `approve_summary` (77-85) | `soroban_sdk::token::Client::approve` | **NO** | Wrap around the `approve` call sites (none under `controller/src/` in current revision; verify if there is one before wiring). | Same. |
| `allowance_summary` (91-95) | `soroban_sdk::token::Client::allowance` | **NO** | Same. | Same. |

### Net wiring status

**0 / 16 new summaries are active.** All are correctly authored — bounds derived from production line ranges, signatures mirror the ABI, doc comments accurate. They are **inert** until controller-side wrappers exist (one per call site) that `apply_summary!` can wrap. The wiring effort is mechanical but non-trivial: ~14 new wrapper functions, each with a 5-10 line `summarized!` block, distributed across `controller/src/{positions,cache,router,utils,flash_loan}/...`.

HANDOFF.md (line 126) classifies this as "Pending: Post-engagement remediation". For an audit run today, the engagement team will see havoc on every cross-contract reply; rules that *appear* to verify pool semantics are actually verifying the prover's nondet bound, which is uniformly weaker than what production guarantees. The risk is **pass-by-vacuity**: rules that should fail because of a pool-side regression do not fail because the pool's reply is unconstrained.

---

## Compat shim cost analysis

The recently parameterised shims add nondet inputs to keep production branches reachable. This section weighs the cost.

### `compat::multiply` (compat.rs:41-92) — 4 nondets

- `account_id: u64` (nondet) — admits both create-new (`== 0`) and load-existing (`> 0`) branches in `process_multiply`.
- `take_initial: bool` (nondet) → `initial_payment: Option<Payment>` — admits with-vs-without initial payment.
- `take_convert: bool` (nondet) → `convert_steps: Option<SwapSteps>` — admits with-vs-without convert step.
- (Plus the rule-provided `mode` mapped through a 4-way match.)

**Cost:** ~3-4 doublings of the prover state space at the `multiply` call site. With 8 calling rules in `strategy_rules.rs`, the total prover work is multiplicative — but each branch is a coherent call into `process_multiply`, not a separate logic explosion.

**Acceptable when:** the calling rule depends on at least one of the parameterised branches (most strategy rules do — they want full coverage of create-new vs load-existing).

**Wasteful when:** the rule only needs the happy path. For example, `multiply_creates_position_with_collateral_and_debt` (strategy_rules.rs:33-70) has `cvlr_assert!(deposit_pos.is_some())` and `borrow_pos.is_some()` — the assertion holds across all four parameter branches but the prover explores all four.

### Recommendation: tight-scope alternates

Add `multiply_simple(env, caller, e_mode_category, collateral_token, debt_to_flash_loan, debt_token, mode, steps)` as a sibling of `multiply` that hard-codes:
- `account_id = 0`
- `initial_payment = None`
- `convert_steps = None`

For rules that genuinely need branch coverage (e.g. `multiply_load_existing_account_works`), keep using `compat::multiply`. For the happy-path coverage rules (which are most of them), use `multiply_simple`. Net prover-budget saving: roughly 4× per simple-path rule.

The same argument applies to `compat::repay_debt_with_collateral` (compat.rs:97-117), which havocs `close_position`. Most rules only care about the don't-close branch; a `repay_debt_with_collateral_simple` would be a fast happy-path alternate.

`compat::liquidate` (124-126) is correctly minimal — no nondet inputs to havoc.

`compat::supply_single`, `borrow_single`, `withdraw_single`, `repay_single` (1-32) are pure ABI adapters around the multi-asset entry points; they pass a single-element vector. No nondet cost. **Correct as-is.**

---

## HANDOFF.md drift

Three load-bearing facts in `controller/certora/HANDOFF.md` are out of date as of `692932c`. An engagement engineer reading the doc will misroute their analysis.

| Line | Claim | Reality | Impact |
|---|---|---|---|
| 125 | `Delete or repurpose summaries/mod.rs \| Pending` | The file is wired and active (10 callers in `views.rs:111,143,260` + `oracle/mod.rs:25,372,492` + `helpers/mod.rs:55,118,139,222`). It is not "empty placeholder" (line 155 of HANDOFF.md). | An engineer reading HANDOFF skips investigation of summaries' soundness, missing the broken `calculate_linear_bonus_summary` and the weak HF summary contract. |
| 126 | `Add apply_summary! wrappers at pool / oracle / SAC call sites \| Pending` | Oracle wrappers exist (3 sites). Pool and SAC wrappers do **not** exist anywhere in `controller/src/`. The 12 pool summaries + 4 SAC summaries authored in 692932c are inert. | An engineer assumes pool calls are summarised. Liquidation rules apparently relying on `seize_position_summary`'s `scaled == 0` guarantee are silently relying on prover havoc instead. |
| 149 | `model.rs # ghost variables (currently unused)` | The file does not exist. `ls controller/certora/spec/model.rs` returns `No such file`. | Cosmetic, but the file-tree section of HANDOFF.md is wrong. |
| 155 | `summaries/ # mod.rs # empty placeholder (post-engagement repurpose)` | `summaries/` now contains `mod.rs` (254 lines, 9 summaries), `pool.rs` (404 lines, 12 summaries), `sac.rs` (96 lines, 4 summaries). | Same as line 125. |
| 99-114 | Rule-quality table cites 16 tautological + 4 vacuous + 9 weak rules. | After the P0 fixes for `bad_debt_threshold` (liquidation rule), `protocol_fee_on_bonus_only` (liquidation), `signed_mul_away_from_zero` (math), `withdraw_rejects_zero_amount` (deleted), the count is stale. The boundary-rule subset of the tautological count is also stale (the boundary rules listed above were not rewritten in the P0 pass). | Engagement-team triage is misled about which rules are signal vs noise. |

These drifts are nits individually but, taken together, they make HANDOFF.md unreliable as a guide for the engagement team.

---

## Findings (numbered)

### Broken

1. **`calculate_linear_bonus_summary` admits all values in `[base, max]`; `boundary_rules::bonus_at_hf_exactly_102` (rule 8) asserts `|bonus - base| ≤ 1`.** Counterexample `bonus = max = 1000` is admitted by the summary; the rule fails the prover. Tighten the summary by branching on `hf` vs target_hf (1.02 WAD): if `hf >= target_hf`, return exactly `base`; else nondet in band. (`summaries/mod.rs:209-219`, `boundary_rules.rs:258-274`).
2. **HF summary makes `hf_after >= hf_before` rules vacuously refutable.** `health_rules::supply_cannot_decrease_hf:93-111` and `liquidation_rules::hf_improves_after_liquidation:33-77` and `boundary_rules::liquidation_at_hf_exactly_one:213` all sample two independent nondet HF values bounded only by `>= 0`. Prover counterexample: `hf_before = 100, hf_after = 0`. The rules cannot pass. Tighten the summary with a per-account ghost so two calls on the same account return the same value, and add a monotonicity coupling for state transitions that must improve HF. (`summaries/mod.rs:131-148, 157-172`).
3. **HANDOFF.md tells the engagement team that pool/SAC wrappers exist when they do not.** The 16 newly authored summaries have zero call sites and zero effect. Either complete the wiring (~14 wrapper functions across 5 files) before the engagement run, or update HANDOFF.md to clearly mark all liquidation, solvency, flash-loan, and strategy rule verdicts as "modulo pool/SAC havoc" so the engagement team can interpret the pass/fail correctly. (`controller/certora/HANDOFF.md:126`, `controller/certora/spec/summaries/pool.rs`, `controller/certora/spec/summaries/sac.rs`).

### Weak

4. **`token_price_summary` admits all 28 values for `asset_decimals`.** Production guarantees `asset_decimals == cached_market_config(asset).oracle_config.asset_decimals`, deterministic per-asset. Tighten by reading from cache. (`summaries/mod.rs:57-69`).
5. **`update_asset_index_summary` lacks a monotone-against-prior bound.** Non-seize callers always see indexes that are `>= prior`. The pool.rs helper `nondet_market_index_monotone` (pool.rs:55-60) has the right shape; expose a controller-side variant. (`summaries/mod.rs:95-110`).
6. **`calculate_account_totals_summary` has independent nondets for the three tuple slots.** Without cross-summary ghosting, `total_borrow_in_usd_summary(account)` and `calculate_account_totals_summary(account).1` (debt) draw independently and can disagree by orders of magnitude in the same rule. Tighten with shared ghost. (`summaries/mod.rs:180-198`).
7. **`total_collateral_in_usd_summary` and `total_borrow_in_usd_summary` lose the zero-account case.** Production returns `0` when account meta is missing or the position map is empty (`views.rs:114-120, 146-152`). The summary returns `>= 0` nondet. Tighten with `if account_id is missing -> 0`. (`summaries/mod.rs:230-241`).
8. **`ltv_collateral_in_usd_summary` does not encode the `<= total_collateral` invariant** that its own doc comment identifies (`summaries/mod.rs:244-247`). (`summaries/mod.rs:249-253`).
9. **`compat::multiply` has no tight-scope alternate.** Every `multiply` rule pays for branch exploration of `account_id`, `initial_payment`, `convert_steps` even when the assertion holds across all branches. Add `multiply_simple` for happy-path rules. (`compat.rs:41-92`).
10. **`compat::repay_debt_with_collateral` has no tight-scope alternate.** Same as #9 for `close_position`. (`compat.rs:97-117`).
11. **HANDOFF.md rule-quality counts are stale post-P0** (line 99-114 still cites 16 tautological + 4 vacuous + 9 weak; some of these were rewritten or deleted in 692932c). (`controller/certora/HANDOFF.md:99-114`).
12. **`is_within_anchor_summary` returns nondet bool** with no per-input determinism. Two calls in the same rule with identical inputs can return different values. Production is deterministic. (`summaries/mod.rs:76-84`).

### Redundant (delete or rewrite)

13. **`borrow_rate_at_exact_zero` (boundary_rules.rs:60)** subsumed by `interest_rules::borrow_rate_zero_utilization`.
14. **`borrow_rate_at_exact_mid` (boundary_rules.rs:85)** subsumed by `interest_rules::borrow_rate_continuity_at_mid`.
15. **`borrow_rate_at_exact_optimal` (boundary_rules.rs:117)** subsumed by `interest_rules::borrow_rate_continuity_at_optimal`.
16. **`borrow_rate_at_100_percent` (boundary_rules.rs:147)** subsumed by `interest_rules::borrow_rate_capped`.
17. **`compound_interest_at_max_rate_max_time` (boundary_rules.rs:171)** subsumed by `interest_rules::compound_interest_monotonic_in_time` + `compound_interest_ge_simple`.
18. **`tolerance_at_exact_first_bound` (boundary_rules.rs:450)** pure tautology; never invokes oracle production.
19. **`tolerance_at_exact_second_bound` (boundary_rules.rs:483)** pure tautology.
20. **`tolerance_just_beyond_second` (boundary_rules.rs:515)** pure tautology.
21. **`supply_dust_amount` (boundary_rules.rs:553)** subsumed by `math_rules::mul_half_up_identity`.
22. **`borrow_exact_reserves` (boundary_rules.rs:578)** pure tautology; never invokes pool production.
23. **`withdraw_more_than_position` (boundary_rules.rs:610)** pure tautology over `min`.
24. **`bad_debt_at_exactly_5_usd` and `bad_debt_at_6_usd` (boundary_rules.rs:295, 323)** effectively tautological — predicate is restated from cvlr_assume.

### Nit

25. **`boundary_rules::liquidation_at_hf_exactly_one` doc comment claims to test the production guard at `liquidation.rs:157`** (`boundary_rules.rs:206-211`). The production guard is now at `liquidation.rs:164`. Update the line ref.
26. **`pool::supply_summary` doc comment cites `pool/src/lib.rs:139, 142, 144-148, 160`** (`summaries/pool.rs:67-75`); current production is at `137-162`. Re-verify after each pool refactor.
27. **`pool::seize_position_summary` doc comment cites `pool/src/lib.rs:521-525, 530, 535, 539`** (`summaries/pool.rs:282-307`); current production is at `510-545`. Re-verify.
28. **The `summarized!` macro defined in `controller/src/lib.rs:13-26`** has no test in either feature mode. Adding a fixture-style spec rule that exercises the `apply_summary!` wrapping (e.g. `assert that production-fn under cfg(certora) calls the spec summary`) would catch wiring regressions early.

---

## Recommended remediation order (efficiency-ordered)

1. **Wire pool + SAC summaries** (broken #3). Without this, every rule downstream of a cross-contract call is silently weakened. ~14 wrapper fns + `summarized!` blocks. Highest-leverage coverage gain.
2. **Tighten `calculate_linear_bonus_summary`** (broken #1). One-line fix; unblocks `boundary_rules::bonus_at_hf_exactly_102`.
3. **Add per-account HF ghost coupling** (broken #2). Medium effort; unblocks ~5 health/liquidation rules that compose two HF reads.
4. **Delete redundant boundary rules #13-21** (5 redundant + 6 tautological + 2 effectively tautological = 13 rule fns + 13 sanity twins = 26 fns). Reduces prover budget by ~13 SMT queries + reduces verdict-table noise. ~280 spec lines removed.
5. **Update HANDOFF.md** (nit/weak #11) to reflect the post-P0 state. ~20-line edit.
6. **Add `multiply_simple` and `repay_debt_with_collateral_simple` compat alternates** (weak #9, #10). Fast happy-path rules switch to the alternates; ~4× speedup on those rules.
7. **Tighten the 7 "weak" summary contracts** (#4-8, #12). One cvlr_assume each; no prover-budget cost; meaningful improvement to downstream rule strength.

Steps 1-3 are correctness; steps 4-7 are efficiency. The audit is at risk on steps 1-3 today.
