# Certora Review — Solvency / Health / Position

**Files reviewed:**
- `controller/certora/spec/solvency_rules.rs` (1005 lines, 21 substantive rules + 5 sanity)
- `controller/certora/spec/health_rules.rs` (129 lines, 4 substantive rules + 2 sanity)
- `controller/certora/spec/position_rules.rs` (150 lines, 5 substantive rules + 1 sanity)
- Cross-referenced against:
  - `controller/certora/spec/summaries/mod.rs`
  - `controller/certora/spec/compat.rs`
  - `controller/src/helpers/mod.rs`, `validation.rs`, `cache/mod.rs`, `lib.rs`
  - `controller/src/positions/{supply,borrow,repay,withdraw,liquidation,account}.rs`
  - `controller/src/storage/{account,certora}.rs`
  - `controller/src/utils.rs`, `controller/src/views.rs`
  - `pool-interface/src/lib.rs`
  - `common/src/{constants,types}.rs`

**Rules examined:** 30 (sanity rules included but not graded).
**Findings:** 21 substantive issues — 4 high, 9 medium, 8 low. 6 rules are fundamentally **broken** by summarization or by mis-stated invariant.

---

## Top-level structural finding (affects health rules globally)

`controller/src/helpers/mod.rs:55` and `:118` wrap **both** `calculate_health_factor` and `calculate_health_factor_for` with `crate::summarized!(...)`. Under `cfg(feature = "certora")`, the macro at `controller/src/lib.rs:13-17` rewrites every callsite of those functions to invoke their summaries (`calculate_health_factor_summary`, `calculate_health_factor_for_summary` at `summaries/mod.rs:113-137`). Each summary returns an independent nondet `i128` constrained only by `>= 0`.

Consequence: every rule in `health_rules.rs` that asserts a property over post-state `calculate_health_factor_for(...)` is asserting a property of an **independent fresh nondet draw**, not of the value any production codepath actually computed. The HF check inside `validation::require_healthy_account` (`validation.rs:75-83`) calls the same summary; on the path the prover explores it draws `hf` separately — the prover only continues if that intermediate draw `>= WAD`, but the post-state draw the rule asserts on is uncorrelated with that intermediate one.

This means rules `hf_safe_after_borrow`, `hf_safe_after_withdraw`, `liquidation_requires_unhealthy_account`, and `supply_cannot_decrease_hf` all **prove the wrong thing**. Each is, structurally, “over all nondet `hf2 >= 0` such that some earlier nondet `hf1` was `>= WAD`, prove `hf2 >= WAD`.” That has counterexamples (`hf2 = 0`), so either:
- The rules are reported as failing by the prover (which would be visible, but the harness surrounds them with sanity rules that *do* pass, suggesting the harness either reports failure or — more likely — the rules are silently weakened by the summary in a way that makes them vacuous because the production HF check inside `require_healthy_account` always “passes” under summary nondeterminism, leaving the post-state draw unrestricted but never falsified by any path).

Either way, **none of the four health rules currently provides a sound guarantee that production HF math respects HF >= WAD post-condition**, because the rule never exercises that math. The same observation applies to the use of `calculate_account_totals` in liquidation — `process_liquidation` at `liquidation.rs:158-166` reads HF via the summarized `calculate_health_factor`, and `helpers::calculate_account_totals` at `liquidation.rs:168-173` is itself summarized at `helpers/mod.rs:140`. The `liquidation_requires_unhealthy_account` rule is therefore trivially satisfied on any nondet draw where the summary returned `hf >= WAD` and on no path actually exercises the real HF arithmetic.

This is the **highest-severity finding in the entire domain** and it dominates everything below. See the per-rule verdicts for concrete consequences.

---

## Per-rule verdicts

### `solvency_rules.rs::pool_reserves_cover_net_supply` (line 36)
**Verdict:** weak.
**Property claimed:** Pool token reserves plus borrowed amount cover what suppliers deposited (`reserves + borrowed >= supplied`).
**What's actually asserted:** `pool_client.reserves() + pool_client.borrowed_amount() >= pool_client.supplied_amount()` (line 47).
**Bug it catches / misses:** Catches a buggy pool that under-tracks reserves OR over-tracks supplied. Misses the time-varying case: this is a **pure invariant** at one cross-contract snapshot — there is no transition tested, so it cannot detect a bug introduced by *some particular operation*. Also the values returned by `pool_client.reserves()`, `borrowed_amount()`, `supplied_amount()` are pure havoc to the prover (no summary contract pins them together — `summaries/mod.rs` does not summarize the cross-contract `LiquidityPoolClient` calls). The prover may pick mutually-inconsistent values.
**Issues:** Cross-contract calls return unsummarized havoc values. No precondition pins their relationship, so the prover could pick `reserves=0, borrowed=0, supplied=10` and falsify — but there is no operation under test, so the rule is checking only that *whatever the prover happens to havoc into those slots* satisfies the inequality. With nothing constraining the slots, the prover can refute by counter-example, or the rule will be verified vacuously if the prover treats cross-contract returns as constrained by the contract code (which it cannot).
**Recommendation:** rewrite. Either (a) summarize the four pool views with a joint contract that establishes the invariant by construction (then this rule becomes a tautology and should be deleted), or (b) write a transition rule: take a snapshot before/after each mutating op, assert the invariant over both states.

### `solvency_rules.rs::revenue_subset_of_supplied` (line 57)
**Verdict:** weak (same flaw as previous).
**Property claimed:** Protocol revenue never exceeds supplied (`revenue <= supplied`).
**What's actually asserted:** `pool_client.protocol_revenue() <= pool_client.supplied_amount()` (line 66).
**Bug it catches / misses:** Same structural problem — both sides are unconstrained havoc. Cannot identify a specific bug.
**Issues:** No summary contract on `protocol_revenue` and `supplied_amount` linking the two values; no operation under test.
**Recommendation:** rewrite as a transition rule (e.g. that an `accrue_interest`/borrow/repay never increases `revenue - supplied`).

### `solvency_rules.rs::borrowed_lte_supplied` (line 79)
**Verdict:** weak (same flaw).
**Property claimed:** `borrowed <= supplied + revenue` — borrowed can exceed supplied only by accumulated reserve-factor revenue.
**What's actually asserted:** `pool_client.borrowed_amount() <= pool_client.supplied_amount() + pool_client.protocol_revenue()` (line 89).
**Issues:** Same as above. Three independent havocked values; the prover can construct a refutation or accept it vacuously.
**Recommendation:** rewrite as transition rule, or summarize the three pool views jointly.

### `solvency_rules.rs::claim_revenue_bounded_by_reserves` (line 102)
**Verdict:** broken.
**Property claimed:** Claimed revenue never exceeds the pool's pre-call reserves.
**What's actually asserted:** `claimed <= pre_reserves` where `claimed = Controller::claim_revenue(...).get(0)` (line 112).
**Bug it catches / misses:** Cannot catch a bug because the cross-contract `claim_revenue` is unsummarized havoc. The prover may model it as returning anything in `i128` range, including `claimed > pre_reserves`. Without a summary that ties `claim_revenue`'s return to `min(reserves, treasury)` (which is the actual pool guarantee at `pool/src/lib.rs:467-477`, per the rule comment), the rule will report violations on every run — unless the prover instead havocs both `pre_reserves` and `claimed` consistently within the same call, which is not how Soroban cross-contract semantics work in CVLR.
**Issues:** Missing summary for `LiquidityPoolClient::reserves()` and `LiquidityPoolClient::claim_revenue()`. The rule as written either fails spuriously or trivially.
**Recommendation:** rewrite. Add a summary in `summaries/mod.rs` for `pool_client.claim_revenue()` that returns nondet `c` with `cvlr_assume!(0 <= c && c <= reserves)` where `reserves` is captured via a paired summary on `pool_client.reserves()`.

### `solvency_rules.rs::utilization_zero_when_supplied_zero` (line 127)
**Verdict:** weak.
**Property claimed:** When `supplied_ray == 0`, `capital_utilisation() == 0`.
**What's actually asserted:** `pool_client.capital_utilisation() == 0` after `cvlr_assume!(sync.state.supplied_ray == 0)` (line 135).
**Issues:** `pool_client.get_sync_data()` and `pool_client.capital_utilisation()` are both unsummarized cross-contract calls. The assume on `supplied_ray == 0` constrains one havocked value; the assertion is on a separate havocked value with no relationship. The rule is structurally the same as the pure-invariant rules above — checking a relationship between two independent havocs.
**Recommendation:** rewrite. Either summarize `capital_utilisation()` jointly with the sync data (same pool snapshot), or test this as a unit property over `pool::src` where the function body is in scope.

### `solvency_rules.rs::isolation_debt_never_negative_after_repay` (line 145)
**Verdict:** strong.
**Property claimed:** Isolated-debt tracker stays non-negative across a repay.
**What's actually asserted:** `get_isolated_debt(asset) >= 0` after a single-asset repay, given the pre-state was non-negative (line 157).
**Bug it catches / misses:** Catches a regression in `adjust_isolated_debt_usd` that lets the tracker go negative (e.g. dropping the `max(0, ...)` clamp at `controller/src/utils.rs:61-92`). Catches a bug that subtracts more than the current tracker on overpayment. The pre-condition is appropriate (induction step), the post-condition is the actual invariant.
**Issues:** None major. Note: only catches the *non-negative* invariant, not the tighter "decrease equals the repaid USD" relation. Acceptable for a smoke-grade rule.
**Recommendation:** confirm.

### `solvency_rules.rs::borrow_respects_reserves` (line 169)
**Verdict:** broken.
**Property claimed:** A successful borrow only completes when `pre_reserves >= amount`.
**What's actually asserted:** `pre_reserves >= amount` after `borrow_single` (line 186).
**Bug it catches / misses:** Cannot catch any bug. The pool's `has_reserves(amount)` check is inside the cross-contract pool body, which is unsummarized havoc. CVLR will model the cross-contract `borrow` call as either "havoc the world and return" (rule passes vacuously) or "the call panics nondeterministically" (also passes vacuously on the path where it panicked). There is no causal link between `pre_reserves` (pre-call snapshot) and the post-call assertion that the borrow body actually used the reserves guard.
**Issues:** Pool's reserves guard lives in the pool contract; CVLR cannot inspect it from controller-side rules. The rule as written is testing controller-side semantics (`borrow_single` doesn't fail in some particular havoc draw), not the pool's reserves guard.
**Recommendation:** delete from controller-side suite. Move to a pool-side rule once `pool/certora/` exists. Alternatively, add a summary for `pool_client.borrow(...)` that documents the post-condition `pre_reserves >= amount` as an `cvlr_assume!`, but that turns the rule into a tautology asserting the assumption.

### `solvency_rules.rs::ltv_borrow_bound_enforced` (line 196)
**Verdict:** broken.
**Property claimed:** After a successful borrow, total debt USD <= LTV-weighted collateral USD.
**What's actually asserted:** `total_borrow <= ltv_collateral` after `borrow_single` (line 211).
**Bug it catches / misses:** Both `total_borrow_in_usd` and `ltv_collateral_in_usd` are summarized at `controller/src/views.rs:143,260`. The summaries return independent nondet `>= 0` values (`summaries/mod.rs:201-218`). The post-state assertion is on two independent fresh nondet draws — provably refutable (e.g. `total_borrow=2, ltv_collateral=1`).
**Issues:** Summary unsoundness for the LTV invariant. The summaries discard the relationship that production code maintains.
**Recommendation:** rewrite. Either (a) tighten `summaries/mod.rs:ltv_collateral_in_usd_summary` and `total_borrow_in_usd_summary` to be coupled (impossible without ghost state pinning the same account), or (b) replace this rule with one that does NOT call the summarized views — instead inline the production calculation against the unsummarized account state, or (c) drop the summary on these views and let the prover handle the iteration cost.

### `solvency_rules.rs::supply_index_above_floor_after_supply` (line 222)
**Verdict:** broken.
**Property claimed:** `supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW` is preserved across a supply.
**What's actually asserted:** Post-state `supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW` given pre-state was (line 241).
**Bug it catches / misses:** `pool_client.get_sync_data()` is unsummarized, so the pre-state and post-state values are independent havocs. The pre-state assume constrains pre; the post assertion is on a fresh independent nondet — counterexamples are trivial. Cannot detect a real index-floor regression.
**Issues:** No summary contract for the pool sync state. The same flaw afflicts every rule that compares pre/post pool sync data via the cross-contract client.
**Recommendation:** rewrite. The floor invariant lives inside `pool/src/interest.rs`; verify it as a pool-side rule. Or summarize `pool_client.get_sync_data()` consistently per call.

### `solvency_rules.rs::supply_index_monotonic_across_borrow` (line 254)
**Verdict:** broken (same flaw as above).
**Property claimed:** Supply index never decreases across a borrow.
**What's actually asserted:** `post.supply_index_ray >= pre.supply_index_ray` (line 271).
**Issues:** Pre and post are independent havocs from cross-contract `get_sync_data()`. No causal relationship is enforced.
**Recommendation:** rewrite as a pool-side rule.

### `solvency_rules.rs::supply_rejects_zero_amount` (line 287)
**Verdict:** strong.
**Property claimed:** `Controller::supply` with amount=0 reverts.
**What's actually asserted:** `cvlr_satisfy!(false)` after the call (line 298) — proves the call must panic.
**Bug it catches / misses:** Catches removal of the `amount > 0` guard in `aggregate_payment_amount` at `utils.rs:87-89` or `validation::require_amount_positive`. The asset is `e.current_contract_address()` so the call additionally fails on missing-market — that doesn't weaken the rule for the zero-rejection property because EITHER failure mode satisfies the assertion. But it means the rule is also trivially satisfied if the missing-market check moves earlier than the zero check; the rule does not pinpoint which guard fired.
**Issues:** Minor — does not prove the *zero-amount* guard specifically reverts; proves only "this call reverts." Use a configured asset to actually exercise the zero check.
**Recommendation:** tighten — pre-configure the asset so the only revert path is the zero-amount one.

### `solvency_rules.rs::borrow_rejects_zero_amount` (line 306)
**Verdict:** strong (same caveat as supply).
**Property claimed:** `Controller::borrow` with amount=0 reverts.
**Issues:** Same as above; reverts may be from missing-market rather than zero-amount.
**Recommendation:** tighten with a configured asset.

### `solvency_rules.rs::withdraw_rejects_zero_amount` (line 325)
**Verdict:** broken.
**Property claimed:** `Controller::withdraw` with amount=0 reverts.
**What's actually asserted:** `cvlr_satisfy!(false)` after the call (line 336).
**Bug it catches / misses:** **The claimed property is false.** Production code at `controller/src/positions/withdraw.rs:96` explicitly maps `amount == 0` to `WITHDRAW_ALL_SENTINEL = i128::MAX` — zero is the documented "withdraw all" sentinel, not an error. The rule passes only because (a) the asset `e.current_contract_address()` is unsupported and `cached_pool_address` panics, or (b) the position lookup returns `None` and `PositionNotFound` panics. Both are unrelated revert reasons. The rule's name and intent contradict the production contract.
**Issues:** Wrong invariant. Withdraw `amount=0` is a feature, not a bug.
**Recommendation:** delete. Replace with a positive rule like `withdraw_zero_means_withdraw_all`: assume a supply position exists, call withdraw with amount=0, assert the post-state position is empty.

### `solvency_rules.rs::repay_rejects_zero_amount` (line 344)
**Verdict:** strong (same caveat as supply).
**Recommendation:** tighten.

### `solvency_rules.rs::supply_position_limit_enforced` (line 368)
**Verdict:** strong.
**Property claimed:** Supplying a NEW asset when at `max_supply_positions` reverts.
**What's actually asserted:** `cvlr_satisfy!(false)` after the call (line 401).
**Bug it catches / misses:** Catches removal or weakening of the limit check in `validation::validate_bulk_position_limits` at `validation.rs:115-162`. Pre-conditions (current count >= limit, asset truly new) are sound and capture the necessary state without begging the question.
**Issues:** None major. Note: the rule's `for` loop using `for i in 0..current_list.len()` and tracking `already_exists` with a boolean works correctly in CVLR's loop unrolling but adds unbounded iteration cost; consider using `current_list.contains(new_asset.clone())` if the SDK exposes it (the SDK Vec does).
**Recommendation:** confirm. (Optional polish: replace manual loop with `Vec::contains` for prover speed.)

### `solvency_rules.rs::borrow_position_limit_enforced` (line 410)
**Verdict:** strong.
**Same as supply variant.**

### `solvency_rules.rs::supply_scaled_conservation` (line 450)
**Verdict:** weak.
**Property claimed:** Position's scaled amount increases by the correct scaled amount; pool supplied increases.
**What's actually asserted:** `scaled_delta > 0` (line 487) and `supplied_after > 0` (line 492).
**Bug it catches / misses:** Catches a regression that fails to credit the user's scaled position at all. **Misses** the actual conservation property — the rule does not assert that `scaled_delta` equals `calculate_scaled_supply(amount)` (within rounding), only that it's positive. The doc comment at line 449 promises `+/-1 rounding`; the assertion does not enforce that. A buggy implementation that credits 1 ray for any amount > 0 would pass.
**Issues:** Postcondition far weaker than the named property. Tautology-adjacent.
**Recommendation:** rewrite. Capture pre-state `supplied_ray` from `pool_client.get_sync_data()` and assert `post.supplied_ray - pre.supplied_ray >= scaled_delta - 1 && <= scaled_delta + 1`.

### `solvency_rules.rs::borrow_scaled_conservation` (line 501)
**Verdict:** weak (same flaw).
**Property claimed:** Pool borrowed_ray increases by the correct scaled amount.
**What's actually asserted:** `scaled_delta > 0` (line 533) and `borrowed_after > 0` (line 540).
**Issues:** Same as above; doesn't enforce conservation.
**Recommendation:** rewrite same way.

### `solvency_rules.rs::repay_scaled_conservation` (line 549)
**Verdict:** weak.
**Property claimed:** Pool borrowed_ray decreases by repaid scaled amount; user position decreases.
**What's actually asserted:** `pos_after < pos_before` (line 584) and `borrowed_after < borrowed_before` (line 588).
**Issues:** `borrowed_before` and `borrowed_after` are independent unsummarized cross-contract havocs (pool_client.borrowed_amount()). The "pool borrowed decreased" assertion compares two havocs with no causal relationship, so the rule cannot reliably catch a regression that fails to reduce pool borrowed. The user-side assertion `pos_after < pos_before` IS sound because it reads on-chain controller storage, not cross-contract state.
**Recommendation:** rewrite. Drop the pool_client comparison or summarize it with a paired pre/post contract.

### `solvency_rules.rs::borrow_index_gte_supply_index` (line 602)
**Verdict:** weak.
**Property claimed:** `borrow_index_ray >= supply_index_ray` for any initialized asset.
**What's actually asserted:** Direct comparison from `market_index` storage read (line 610).
**Bug it catches / misses:** Catches an invariant violation in stored market_index. The pre-condition `liquidation_threshold_bps > 0` proxies "asset initialized" — adequate. However, `get_market_index` in storage/certora.rs:105-114 calls `pool_client.get_sync_data()` cross-contract, so values are pure havoc and the assertion is comparing two independent nondets. The summary at `summaries/mod.rs:88-101` does enforce `borrow_index_ray >= supply_index_ray` via `cvlr_assume!`, but the production `market_index::get_market_index` is NOT routed through that summary — it does a direct cross-contract read. So the assumption is not visible to this rule.
**Issues:** The asserted invariant is what `update_asset_index_summary` already assumes — but only when reached through `crate::oracle::update_asset_index`, not through `storage::market_index::get_market_index`. Cross-summary inconsistency.
**Recommendation:** rewrite to use the summarized accessor (`crate::oracle::update_asset_index` via `cache.cached_market_index`) so the index pair comes from the same summarized snapshot.

### `solvency_rules.rs::supply_index_grows_slower` (line 621)
**Verdict:** broken.
**Property claimed:** `supply_growth <= borrow_growth` across a supply (reserve factor cut).
**What's actually asserted:** `supply_after - supply_before <= borrow_after - borrow_before` (line 656).
**Bug it catches / misses:** Same fundamental flaw — `supply_before`, `borrow_before`, `supply_after`, `borrow_after` come from `storage::market_index::get_market_index` which routes through unsummarized cross-contract `get_sync_data()`. Four independent havoc values; the relationship is not enforced. Cannot catch a regression in interest accrual math.
**Recommendation:** rewrite as a pool-side rule, or use `cache.cached_market_index` (summarized) for both pre and post and assume the summary's joint contract.

### `solvency_rules.rs::index_cache_single_snapshot` (line 712)
**Verdict:** strong.
**Property claimed:** Repeated `cached_market_index(asset)` calls return the same snapshot within a transaction.
**What's actually asserted:** `index1.supply_index_ray == index2.supply_index_ray && index1.borrow_index_ray == index2.borrow_index_ray` (lines 723-724).
**Bug it catches / misses:** Catches a regression that drops the cache hit-path in `cache::cached_market_index` at `cache/mod.rs:124-140` and re-fetches every call, returning a stale-vs-fresh pair on the second call. This is real — the index-stale-snapshot attack vector is well-defined.
**Issues:** None significant. Even with the summary, the cache code-path is in scope (the cache is in controller, not pool). Soundness intact.
**Recommendation:** confirm.

### `solvency_rules.rs::supply_withdraw_roundtrip_no_profit` (line 750)
**Verdict:** strong.
**Property claimed:** `mul_div_half_up(scale, then unscale)` recovers at most amount + 1.
**Bug it catches / misses:** Catches a regression in `mul_div_half_up` that introduces >1 unit drift. Correct direction (round up gain).
**Issues:** This is really a math-rules property, not solvency. Acceptable here as defense-in-depth.
**Recommendation:** confirm.

### `solvency_rules.rs::borrow_repay_roundtrip_no_profit` (line 789)
**Verdict:** strong.
**Same shape as the supply variant.**
**Recommendation:** confirm.

### `solvency_rules.rs::price_cache_invalidation_after_swap` (line 822)
**Verdict:** strong.
**Property claimed:** `clean_prices_cache()` empties the price cache; subsequent fetch repopulates.
**Bug it catches / misses:** Catches removal of the cache invalidation at `cache.clean_prices_cache()` (cache/mod.rs:90-92), or a regression that fails to clear the entry for a specific asset.
**Issues:** Note this is properly an oracle-domain rule; reasonable to keep here as a stale-state defense.
**Recommendation:** confirm.

### `solvency_rules.rs::mode_transition_blocked_with_positions` (line 865)
**Verdict:** weak.
**Property claimed:** Account in e-mode with borrows cannot supply an isolated asset (would require switching modes).
**What's actually asserted:** `cvlr_satisfy!(false)` after the supply call (line 903).
**Bug it catches / misses:** The assumption set is large (e_mode_category > 0, !is_isolated, has borrow positions, asset is_isolated_asset). The actual production blockage path goes through `validate_isolated_collateral` and `ensure_e_mode_compatible_with_asset`, both of which are also reachable via paths that don't depend on having borrow positions. Whether the test exercises the *mode-transition* path or just the *isolated-asset-supplied-into-non-isolated-account* path is unclear; the rule doesn't distinguish.
**Issues:** Mixes two invariants. Pre-conditions over-constrain (could drop the borrow-positions condition and still hit the same revert).
**Recommendation:** tighten — split into two rules: one for "isolated asset cannot be supplied to non-isolated account when other supplies exist", one for "e-mode account cannot accept isolated supply". The current single rule conflates them.

### `solvency_rules.rs::compound_interest_bounded_output` (line 922)
**Verdict:** strong.
**Property claimed:** Compound factor < 100,000 * RAY for bounded rate and time.
**Recommendation:** confirm. (Math-rules domain, not strictly solvency.)

### `solvency_rules.rs::compound_interest_no_wrap` (line 954)
**Verdict:** strong.
**Property claimed:** Compound factor >= RAY for non-negative rate and time.
**Recommendation:** confirm.

---

### `health_rules.rs::hf_safe_after_borrow` (line 18)
**Verdict:** broken (see top-level finding).
**Property claimed:** After a successful borrow, HF >= WAD.
**What's actually asserted:** `hf >= WAD` where `hf = calculate_health_factor_for(...)` (line 29).
**Bug it catches / misses:** Catches **no real bug**. `calculate_health_factor_for` is summarized at `helpers/mod.rs:118-133` to return any nondet `i128 >= 0`. The borrow flow at `borrow.rs:99` calls the same summarized function inside `require_healthy_account` — the prover models that as a fresh nondet draw on each invocation, so the path-restriction "borrow only completes if internal HF >= WAD" does not propagate to the post-state assertion's draw.

A buggy `calculate_health_factor` that returns `WAD - 1` for all undercollateralized accounts (i.e., always allows the borrow) would still pass this rule because the rule is not exercising the production HF math at all.
**Issues:** Summary erases the safety invariant the rule is meant to verify.
**Recommendation:** rewrite. Either (a) drop the `summarized!` wrapper from `calculate_health_factor` (accept TAC blow-up cost), or (b) introduce a correlated-summary mechanism (ghost state holding the last-computed HF for a given account, asserted to be returned consistently across calls within the same transaction).

### `health_rules.rs::hf_safe_after_withdraw` (line 38)
**Verdict:** broken (same root cause).
**Bug it catches / misses:** Identical to `hf_safe_after_borrow` — the assertion is over a fresh nondet draw with no causal link to the production math.
**Recommendation:** rewrite (same fix).

### `health_rules.rs::liquidation_requires_unhealthy_account` (line 60)
**Verdict:** broken.
**Property claimed:** `process_liquidation` reverts when HF >= WAD.
**What's actually asserted:** `cvlr_satisfy!(false)` after the call (line 83), with pre-condition `hf_before >= WAD`.
**Bug it catches / misses:** The pre-state `hf_before` is a nondet draw `>= 0`; the rule constrains it to `>= WAD`. Inside `process_liquidation`, the production HF check at `liquidation.rs:158-166` calls `calculate_health_factor` (summarized) — that's a NEW independent nondet draw. The two draws are uncorrelated. There exist paths where the pre-state draw was `>= WAD` AND the in-production draw was `< WAD`, on which the liquidation completes successfully and the rule fails. The rule will report failures spuriously, OR — if the prover models cross-call summary draws as session-stable for a given context — the rule may be vacuously verified.

A buggy `calculate_health_factor` that always returns `WAD / 2` regardless of inputs would NOT be caught — the production check would pass (in summary land), the rule's pre-state draw is unrelated, and `cvlr_satisfy!(false)` proves only that the call panics on the path the prover explores.
**Issues:** Same summary-decoupling pathology.
**Recommendation:** rewrite. Drop the summary on `calculate_health_factor` for at least this rule, or factor out the HF computation so it can be replaced with a deterministic ghost-state read.

### `health_rules.rs::supply_cannot_decrease_hf` (line 92)
**Verdict:** broken.
**Property claimed:** Supplying collateral never decreases HF.
**What's actually asserted:** `hf_after >= hf_before` where both come from `calculate_health_factor_for` (line 110).
**Bug it catches / misses:** Both `hf_before` and `hf_after` are independent nondet draws from `calculate_health_factor_for_summary`. The assertion compares two independent nondets; the prover can pick `hf_before = i128::MAX, hf_after = 0` and refute. **A buggy supply that REDUCES the user's collateral position would not be caught**, because the bug shows up in real HF math which the rule never exercises.
**Issues:** Summary kills the property. Note also that the rule uses TWO separate `ControllerCache::new` invocations (`cache` and `cache2`, lines 102 and 107), so even if HF were not summarized, the per-cache index reads would still be independent nondets — the pre/post correlation requires either a single shared cache (impossible since supply mutates state and a stale cache is wrong) or shared ghost state.
**Recommendation:** rewrite. Drop the summary. Use one cache pre-supply and a fresh cache post-supply; assert `hf_after >= hf_before` against the actual production computation.

---

### `position_rules.rs::supply_increases_position` (line 22)
**Verdict:** strong.
**Property claimed:** Supply credits the user's deposit position.
**What's actually asserted:** `pos_after > pos_before` over `storage::positions::get_scaled_amount(...)` (line 41).
**Bug it catches / misses:** Catches a regression in `update::update_or_remove_position` that fails to write the new scaled value, or in the pool-returned position that miscredits. The position-side reads use direct controller storage (no summary), so the assertion is sound.
**Issues:** None. The rule does not enforce that the delta equals the supplied amount, only that it's positive — but a separate conservation rule could cover that.
**Recommendation:** confirm.

### `position_rules.rs::borrow_increases_debt` (line 49)
**Verdict:** strong.
**Same shape, same soundness.**
**Recommendation:** confirm.

### `position_rules.rs::full_repay_clears_debt` (line 77)
**Verdict:** strong.
**Property claimed:** Repaying strictly more than the outstanding scaled debt zeros the position.
**What's actually asserted:** `pos_after == 0` (line 92).
**Bug it catches / misses:** Catches a regression in `pool_client.repay`'s overpayment-clamping at `repay.rs:115` (`actual_amount = amount.min(outstanding_before)`), or in `update::update_or_remove_position` failing to remove a zeroed entry. Pre-conditions (pos_before > 0, amount > pos_before, amount <= WAD) are sound. The bound on `amount <= WAD` is documented as a path-pruning artifact.
**Issues:** Note: the comparison `amount > pos_before` compares the requested *token amount* to the stored *scaled_amount_ray*. These are in different units. If `borrow_index_ray >> RAY`, then `pos_before` (scaled) could be much smaller than the actual debt (raw), but the assumption `amount > pos_before` doesn't necessarily imply `amount > actual_debt`. So the rule may incorrectly assume "this is a full overpayment" when in fact it isn't enough. In that case, `pos_after` won't be zero, and the rule WILL fail — but for the wrong reason (under-payment, not bug). This is a subtle unit confusion in the precondition.
**Recommendation:** tighten. Compare `amount` to the actual debt (`pos_before * borrow_index / RAY`, approximated via `pool_client.borrowed_amount()` or the cache's market_index).

### `position_rules.rs::withdraw_decreases_position` (line 99)
**Verdict:** strong.
**Bug it catches / misses:** Catches a regression that lets withdraw leave the deposit position unchanged or grow it.
**Issues:** Pre-condition `pos_before > 0` ensures we are withdrawing against a real position. Note: the rule does not distinguish between full and partial withdraw; if the pool returns the position with `scaled = 0` (full withdraw), `pos_after == 0` < `pos_before` satisfies the assertion. Good.
**Recommendation:** confirm.

### `position_rules.rs::repay_decreases_debt` (line 125)
**Verdict:** strong.
**Same shape as withdraw_decreases_position.**
**Recommendation:** confirm.

---

## Missing invariants (rules we should have but don't)

The following solvency / health / position invariants are **not** verified by any rule in the three files reviewed:

1. **`scaled_amount_ray >= 0` always (supply and borrow).** No rule enforces non-negativity of stored scaled amounts. A regression that lets a position go negative through subtraction underflow would not be caught.

2. **Account close requires both maps empty.** `cleanup_account_if_empty` at `account.rs:48-52` requires both supply and borrow maps to be empty before deletion, and `process_withdraw` at `withdraw.rs:67-73` is the only path that calls `remove_account`. No rule verifies that:
   - `remove_account` on a non-empty account is unreachable (the existing storage path makes it possible to call `remove_account_entry` directly — would be caught by a rule asserting "no path removes account meta while side maps non-empty").
   - The trailing `set_supply_positions` at `withdraw.rs:72` writes the correct map.

3. **Position parameters snapshot at open time.** `liquidation_threshold_bps`, `liquidation_bonus_bps`, `liquidation_fees_bps`, and `loan_to_value_bps` are written once when a position is opened (`get_or_create_borrow_position` at `borrow.rs:343-362`, `get_or_create_deposit_position` at `supply.rs:224-243`). The supply path *refreshes* LTV / bonus / fees on each supply (`supply.rs:266-274`) but NOT threshold (keeper-only). No rule verifies:
   - Threshold field on a deposit position is unchanged across a supply.
   - All four fields on a borrow position are unchanged across a borrow.

4. **AccountMeta TTL bumped on every side-map write.** `write_side_map` at `account.rs:33-54` bumps meta TTL whenever a side map is written. No rule verifies this — a regression that drops the meta-bump would orphan accounts on TTL expiry.

5. **`create_account` produces empty maps.** `account::create_account` at `account.rs:11-40` constructs an account with empty supply and borrow positions. A rule like "post-`create_account`, both `get_supply_positions` and `get_borrow_positions` are empty" is missing.

6. **Liquidation cannot run on HF >= WAD.** `liquidation_requires_unhealthy_account` claims to verify this but is broken (above). No working version exists.

7. **Position count never exceeds `max_supply_positions` / `max_borrow_positions` (post-state).** `supply_position_limit_enforced` and `borrow_position_limit_enforced` verify the *pre-state limit blocks new opens*, but no rule asserts the post-state invariant `count(supply) <= max_supply_positions` directly. A regression that bypasses the limit guard via a different code-path would not be caught.

8. **`total_collateral_weighted_usd >= total_debt_usd` for any account with HF >= WAD.** This is the actual liquidation-threshold invariant and is the one a "solvency" rule should anchor on. The summarized `calculate_account_totals` returns `(total_collateral, total_debt, weighted_coll)` with `cvlr_assume!(weighted_coll <= total_collateral)` (`summaries/mod.rs:157`), but the relationship `weighted_coll >= total_debt iff HF >= WAD` is not asserted anywhere.

9. **HF monotonicity under repay.** Repay can only improve HF (or keep it the same). No rule.

10. **HF monotonicity under withdraw of pure-supply account (no debt).** Trivially true (HF stays at `i128::MAX`), but worth pinning so a regression that errantly computes HF for a no-debt account doesn't slip through.

11. **Idempotency of supply with amount=0 in batch.** `aggregate_payment_amount` at `utils.rs:81-99` rejects `amount == 0` for supply/borrow/repay; the rule `supply_rejects_zero_amount` covers single-element batches, but not multi-asset batches with one zero. A regression that aggregates `[(A, 5), (B, 0)]` through without panicking would not be caught.

12. **e-mode / isolation exclusivity post-construction.** `validate_e_mode_isolation_exclusion` runs at create-time. No rule verifies that for any persisted account `is_isolated && e_mode_category_id != 0` is impossible.

13. **Account ownership invariant.** No rule verifies that `account.owner` matches what `create_account` set (no path mutates owner; a regression that does should be caught).

14. **Liquidation post-condition: HF improves or terminates the account.** `liquidation.rs:283-310` guarantees that after a successful liquidation, either HF rises OR the account moves to bad-debt cleanup. No rule verifies this monotonicity.

15. **Withdraw with debt requires HF >= WAD (covered by `hf_safe_after_withdraw` but broken via summarization).** Need a working version.

---

## Summary integrity

For each summary in `summaries/mod.rs` used by this domain's rules:

| Summary | Used by (in domain) | Nondet contract | Bound match | Issue |
|---|---|---|---|---|
| `calculate_health_factor_summary` (line 113) | `validation::require_healthy_account` (transitively from every borrow/withdraw rule), `liquidation::execute_liquidation` | Returns `i128 >= 0` | **Too loose.** The real function clamps overflow to `i128::MAX` and returns `WAD` units; never returns negative. The lower bound is fine but the contract loses ALL relationship to inputs. | **HIGH SEVERITY.** Decouples health rules from production math (root cause of all four broken health rules). |
| `calculate_health_factor_for_summary` (line 129) | `health_rules::*` directly | Returns `i128 >= 0` | Same as above. | Same. |
| `calculate_account_totals_summary` (line 145) | `liquidation::execute_liquidation`, `liquidation::check_bad_debt_after_liquidation` | Three values >= 0; weighted_coll <= total_collateral | Real function: weighted_coll <= total_collateral (matches), no other relationships pinned. The relationship `total_debt > total_collateral` (bad debt threshold) is invariant in production but not in summary — a buggy production that returns inconsistent totals would not be caught. | **MEDIUM.** Allows the prover to pick degenerate triples. |
| `calculate_linear_bonus_summary` (line 174) | Liquidation rules (out of scope here; relevant to seizure invariants) | `bonus in [base, max]` | Matches the real function's range. | Acceptable. |
| `total_collateral_in_usd_summary` (line 195) | Not used in this domain (used in views). | `>= 0`. | Loose; no relationship to per-asset values. | Low (out of domain). |
| `total_borrow_in_usd_summary` (line 202) | `solvency_rules::ltv_borrow_bound_enforced` | `>= 0`. | **Too loose** for the LTV bound check — independent from `ltv_collateral_in_usd_summary`. | **HIGH** for `ltv_borrow_bound_enforced` (it becomes vacuous). |
| `ltv_collateral_in_usd_summary` (line 214) | `solvency_rules::ltv_borrow_bound_enforced` | `>= 0`. | Same as above. | Same. |
| `update_asset_index_summary` (line 88) | Index reads via `cached_market_index` | `supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW`, `borrow_index_ray >= RAY`, `borrow_index_ray >= supply_index_ray`. | Matches production guarantees. **However**, `storage::market_index::get_market_index` (used by `borrow_index_gte_supply_index`, `supply_index_grows_slower`) bypasses this summary — direct cross-contract read from `LiquidityPoolClient::get_sync_data`, which is unsummarized havoc. | **MEDIUM.** Summary inconsistency between two paths reading the same logical data. |
| `token_price_summary` (line 50) | All HF/LTV computations | `price_wad > 0`, `asset_decimals <= 27`, `timestamp <= now/1000 + 60`. | Matches production. | Acceptable. |

**Cross-contract pool client calls (`reserves`, `borrowed_amount`, `supplied_amount`, `protocol_revenue`, `capital_utilisation`, `get_sync_data`, `claim_revenue`) are NOT summarized.** Every solvency rule that compares two such values is structurally testing two independent havocs. This is the second-largest soundness gap in the domain after the HF summarization issue.

---

## Severity-tagged action items

### High (block release)

1. **Fix or remove HF summarization for health rules.** Rules `hf_safe_after_borrow`, `hf_safe_after_withdraw`, `liquidation_requires_unhealthy_account`, `supply_cannot_decrease_hf` are all structurally broken because `calculate_health_factor[_for]` is replaced with an independent nondet at every callsite. Either (a) drop the `summarized!` wrapper on these two functions and accept the prover cost, (b) introduce ghost state that pins the per-account HF across calls within one transaction, or (c) document that these rules are placeholders and replace them with rules that exercise the unsummarized `position_value` / `weighted_collateral` primitives directly.

2. **`solvency_rules::withdraw_rejects_zero_amount` asserts a false invariant.** Production at `controller/src/positions/withdraw.rs:96` treats `amount=0` as "withdraw all". Delete the rule and replace with `withdraw_zero_means_withdraw_all`.

3. **`solvency_rules::ltv_borrow_bound_enforced` is vacuous** because `total_borrow_in_usd` and `ltv_collateral_in_usd` summaries return independent nondets. Rewrite to inline the LTV check against the unsummarized supply / borrow position maps, or tighten the summaries to share a ghost.

4. **Cross-contract pool views (`reserves`, `supplied_amount`, `borrowed_amount`, `protocol_revenue`, `claim_revenue`, `get_sync_data`, `capital_utilisation`) are unsummarized.** Every rule that depends on inter-relating two such values (rules `pool_reserves_cover_net_supply`, `revenue_subset_of_supplied`, `borrowed_lte_supplied`, `claim_revenue_bounded_by_reserves`, `utilization_zero_when_supplied_zero`, `borrow_respects_reserves`, `supply_index_above_floor_after_supply`, `supply_index_monotonic_across_borrow`, `borrow_index_gte_supply_index`, `supply_index_grows_slower`, `repay_scaled_conservation`'s pool half) is unsound on this account. Add joint summaries in `summaries/mod.rs` that expose the pool's invariants (`supplied + revenue >= borrowed`, etc.) as `cvlr_assume!` post-conditions.

### Medium

5. **`solvency_rules::{supply,borrow}_scaled_conservation` enforce only sign, not magnitude.** Tighten to assert the scaled delta is within `+/-1` of `mul_div_half_up(amount, RAY, index)` (the named "+/-1 rounding" property is not actually checked).

6. **`solvency_rules::repay_scaled_conservation` pool-side assertion is unsound.** Drop or summarize.

7. **`position_rules::full_repay_clears_debt` pre-condition has unit confusion.** Comparing `amount` (token decimals) against `pos_before` (scaled_amount_ray, RAY decimals). With high `borrow_index_ray`, the precondition `amount > pos_before` does not imply `amount > actual_debt`. Tighten the precondition to compare against the actual debt amount.

8. **Add a non-negativity invariant rule for `scaled_amount_ray`** (both supply and borrow positions). This is a foundational invariant the rules assume but never assert.

9. **Add an account-close invariant.** `cleanup_account_if_empty` is the only sanctioned remove path; verify that `remove_account` is unreachable while either side map is non-empty.

10. **Add HF monotonicity rules under repay (improves) and under no-debt withdraw (stays at i128::MAX).**

11. **Add position-count post-state invariants.** Currently only the *blocking* rule is verified; the post-state count <= max is not.

12. **Split `mode_transition_blocked_with_positions`** into the two distinct invariants it conflates.

13. **`borrow_index_gte_supply_index` reads through the unsummarized cross-contract path.** Route through `cache::cached_market_index` so the summary's joint contract applies.

### Low

14. **`solvency_rules::{supply,borrow,repay}_rejects_zero_amount` use the controller address as the asset**, so the revert may come from missing-market rather than zero-amount. Tighten by configuring the asset.

15. **`solvency_rules::supply_position_limit_enforced` / `borrow_position_limit_enforced` use a manual "already_exists" loop** (`for i in 0..len { ... }`). Replace with `Vec::contains` if the SDK Vec exposes it (it does), to reduce prover cost.

16. **`solvency_rules::supply_withdraw_roundtrip_no_profit` and `borrow_repay_roundtrip_no_profit`** belong in math_rules; consider moving for clean separation.

17. **`solvency_rules::compound_interest_*`** belong in interest_rules; consider moving.

18. **Add e-mode / isolation exclusivity rule for persisted accounts.**

19. **Add `create_account_starts_empty` rule.**

20. **Add `position_params_snapshot_invariant` rule.** Threshold/bonus/fees/LTV stability across borrow; threshold stability across supply.

21. **Add `account_meta_ttl_bumped_on_side_write` rule.** Although TTL semantics may not be observable directly, the side-effect (`bump_user` invocation) can be verified via a ghost-state hook.

---

## Closing note

The position-domain rules (`position_rules.rs`) are the strongest in this set — five of five substantive rules are structurally sound because they read direct controller storage and assert simple monotonicity properties. The solvency-domain rules are mixed: pure-math rules (rounding, compound interest) are sound; pool-state rules are largely unsound due to unsummarized cross-contract calls. The health-domain rules are uniformly broken by the summary on `calculate_health_factor[_for]`.

If only one item from this review is acted on, it should be **#1 (fix HF summarization)**, because it invalidates every health rule and every solvency rule that depends on `require_healthy_account` reverting on subhealthy paths.
