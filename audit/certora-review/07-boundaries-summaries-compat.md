# Domain 7 — Boundaries, Summaries, Compat (Certora framework integrity)

**Phase:** Certora formal-verification meta-review
**Files in scope:**
- `controller/certora/spec/boundary_rules.rs` (630 lines, 38 rule fns)
- `controller/certora/spec/summaries/mod.rs` (218 lines, 9 summaries)
- `controller/certora/spec/compat.rs` (84 lines, 6 entry-point shims)
- `controller/certora/spec/mod.rs` (31 lines, module registration)
- `controller/certora/HANDOFF.md` (engagement-team handoff design intent)

**Production references:**
- `controller/src/lib.rs:10-26` (`summarized!` macro, conditional `apply_summary!` indirection)
- `controller/src/storage/account.rs:1-220` (slim `AccountMeta`, side-map storage)
- `controller/src/storage/{certora,instance,market,pools,debt,emode,ttl}.rs`
- `controller/src/cache/mod.rs:33-156` (`ControllerCache`, `cached_pool_sync_data`)
- `controller/src/oracle/mod.rs:25-64, 372-392, 492-508` (summarised entry points)
- `controller/src/helpers/mod.rs:55-228` (summarised helpers)
- `controller/src/views.rs:111-270` (summarised view aggregates)
- `controller/src/positions/{supply,borrow,withdraw,repay,liquidation}.rs` (unsummarised pool cross-contract calls)
- `pool/src/lib.rs:132-749` (production pool implementation)
- `pool-interface/src/lib.rs:1-85` (`LiquidityPoolInterface` trait)
- `vendor/cvlr-soroban/cvlr-soroban-macros/src/apply_summary.rs:1-88` (macro semantics)
- `common/src/types.rs:172-184, 347-360, 552-573` (`AccountMeta`, `PoolPositionMutation`, `ControllerKey`)

**Totals:** broken=12  weak=6  nit=4  missing=14 (coverage gaps + spec/HANDOFF drift)

---

## Summary verdict

The Certora framework infrastructure (boundary rules + summaries + compat) is the keystone of every other domain rule's validity, and it has structural soundness defects that make the entire formal-verification track misleading.

The headline findings:

1. **Tuple ordering bug in `calculate_account_totals_summary`.** Production returns `(total_collateral, total_debt, weighted_coll)` (`helpers/mod.rs:184`); the summary returns `(total_collateral, weighted_coll, total_debt)` (`summaries/mod.rs:158-162`). Every rule that calls `calculate_account_totals` under `--features certora` reads `weighted_coll` where production yields `total_debt`, and vice-versa. The bound `cvlr_assume!(weighted_coll_raw <= total_collateral_raw)` (line 157) is *applied to the wrong field*. This is a UB-level summary defect: `liquidation_collateral_available` (`views.rs:251`) returns `total_debt` to the prover instead of `weighted_coll`; any liquidation rule that depends on the third tuple element observes the second.

2. **Boundary rules 6, 7, 9, 10 are documented as "real production helper" calls but invoke a stub returning `nondet >= 0`.** The 2-paragraph note at `boundary_rules.rs:204-211` explicitly claims the rewrite "constrains the real cached HF via cvlr_assume and then assert[s] the liquidation-guard predicate against it." Under `apply_summary!` (`vendor/cvlr-soroban-macros/src/apply_summary.rs:21-23`), calls to `crate::helpers::calculate_health_factor_for(...)` are rewritten to `crate::spec::summaries::calculate_health_factor_for_summary(...)` which returns `let hf: i128 = nondet(); cvlr_assume!(hf >= 0); hf`. The "rewritten" rules are *exactly* as tautological as the original local-constant versions. The comment is a soundness-claim regression: a reviewer reading the file is told the helper is invoked; the prover sees a havoc'd value.

3. **`bonus_at_hf_exactly_102` (rule 8) calls a summary that returns `nondet ∈ [base_bonus, max_bonus]`** (`summaries/mod.rs:174-184`). The rule asserts `|bonus - base_bonus| <= 1`. The summary explicitly admits `bonus = max_bonus = 1000` as a valid return. The rule is unsound-as-written: either it fails on the prover (counterexample `bonus = 1000`) — in which case the engagement team gets a misleading verdict — or the prover quirks the rule into "passing" via some encoding peculiarity, and a real protocol violation would not be caught.

4. **Zero pool cross-contract summaries.** `pool-interface/src/lib.rs:10-85` declares 22 methods on `LiquidityPoolInterface`. `summaries/mod.rs` summarises *zero* of them. The 14 production cross-contract `pool_client.<method>(...)` call sites in `controller/src/{cache,positions,router,utils,flash_loan,storage/certora}/...` are passed to the prover as raw external invocations. The framework relies on prover havoc semantics; this is *sound* but loses every rate, conservation, and re-entry property the pool *does* guarantee. Every rule that traces through `Controller::supply/borrow/withdraw/repay/multiply/liquidate/claim_revenue` (which is *most* health, solvency, liquidation, flash-loan, strategy, and isolation rules) sees an unconstrained pool reply that can return arbitrary `actual_amount`, arbitrary `position`, arbitrary `market_index`. The rules either accidentally over-constrain via `cvlr_assume!` outside the production path, or pass vacuously.

5. **HANDOFF.md is out-of-date in three load-bearing places.** Line 125: "Delete or repurpose `summaries/mod.rs` | Pending" — the file is not empty; it is wired and active. Line 126: "Add `apply_summary!` wrappers at pool / oracle / SAC call sites | Pending" — wrappers exist for oracle/helpers/views but **not** for pool or SAC; the `summarized!` macro is imported in three controller files. Line 149: `model.rs # ghost variables (currently unused)` — the file does not exist (`ls controller/certora/spec/` returns no `model.rs`). The HANDOFF document tells the engagement team the framework is in a state it isn't; an engagement engineer reading the doc will misroute their analysis.

6. **Boundary rules 11, 15, 16, 17, 19, 20 prove arithmetic identities, not protocol invariants.** They rebuild the production predicate locally (`let qualifies = total_debt > total_collateral && total_collateral <= bad_debt_threshold`, `let would_panic = borrow_amount > available_reserves`, `let in_first_tier = deviation <= first_tolerance`, etc.) and assert that local predicate against locally-assumed values. None of these wire to the actual `process_liquidation`, `pool::borrow`, `oracle::tolerance_check`, or `withdraw_position` invocation that *implements* the predicate in production. A regression that flips `<= ` to `<` in production would not surface in any of these rules.

7. **Storage refactor (slim `AccountMeta` + side maps) is structurally compatible with boundary rules** because no boundary rule directly reads `ControllerKey`, `AccountMeta`, `SupplyPositions`, or `BorrowPositions`. It does, however, surface a gap: there is no boundary or solvency rule that asserts `AccountMeta` lifecycle invariants under the new layout — e.g., "after `remove_account_entry`, neither `SupplyPositions(id)` nor `BorrowPositions(id)` exist", or "`bump_account` is idempotent and never resurrects deleted side maps". The new TTL-bumping side-effect at `storage/account.rs:50-53` (writing a side map bumps `AccountMeta` TTL) creates an invariant — *no live side map outlives `AccountMeta`* — that no rule attempts to verify.

8. **`compat.rs` entry-point shims silently compress the e-mode-category argument.** `multiply` (lines 32-63) discards the `account_id: u64` parameter that production accepts at `controller::Controller::multiply` (the `0` literal at line 53 means *every multiply rule starts a new account*, never reuses an existing one). This means strategy / liquidation rules that compose `multiply` followed by `liquidate` on the same `account_id` are unreachable through `compat`.

A reviewer following HANDOFF.md's prioritisation (boundary last, weak rules acceptable, "16 tautological rules" already documented) would *not* surface findings 1-3 above. Those findings break invariants the HANDOFF document treats as solid framework infrastructure.

---

## Summary contract table

Bounds in **bold** match the production post-condition; *italic* bounds are weak (sound but lose information); ✗ bounds disagree with production (unsound or buggy); ⚠ indicates the function is wired but never invoked from any production call site that *would* surface a regression.

### Internal helpers / views currently summarised

| Production fn (path) | Summary fn (`summaries/mod.rs:line`) | Returned-value bounds | Soundness | Notes |
|---|---|---|---|---|
| `oracle::token_price` (`oracle/mod.rs:25-64`) | `token_price_summary` (50-62) | `price_wad > 0` ✓; `asset_decimals <= 27` ✓; `timestamp <= now/1000 + 60` ✓ | ✓ sound | Misses: `asset_decimals == cached_market_config(asset).oracle_config.asset_decimals` (production guarantees it; summary nondets it). Misses: returned timestamp `== now/1000` (production sets it deterministically). Domain rules `oracle_rules::price_staleness_enforced`, `health_rules::*` falsely "pass" because the summary already enforces a weaker form of the asserted post-condition. |
| `oracle::is_within_anchor` (`oracle/mod.rs:372-392`) | `is_within_anchor_summary` (69-77) | `bool` (nondet) | ✓ sound | Maximally weak. Every oracle tier-discrimination rule that uses `is_within_anchor` sees an oracle that can claim "in-band" or "out-of-band" arbitrarily. The first/second-tolerance rules in `oracle_rules.rs` and `boundary_rules.rs` Rule 15-17 don't invoke this function — they re-implement the predicate locally. So the summary's weakness is invisible. But any future rule that *does* invoke it gets no information. |
| `oracle::update_asset_index` (`oracle/mod.rs:492-508`) | `update_asset_index_summary` (88-101) | `supply_index_ray >= SUPPLY_INDEX_FLOOR_RAW (= WAD)` ✓; `borrow_index_ray >= RAY` ✓; `borrow_index_ray >= supply_index_ray` ✓ | ⚠ partially sound | Bound `borrow_index_ray >= supply_index_ray` (line 96) does *not* hold in production. Production allows `supply_index > borrow_index` after a bad-debt socialisation event (`pool::seize_position` at `pool/src/lib.rs:521-525` calls `apply_bad_debt_to_supply_index` which can push `supply_index` up sharply). This is **over-constraining**: any rule that invokes the summary while reasoning about a post-bad-debt state will reason about a state production cannot reach. False negatives possible in `index_rules`, `liquidation_rules` (post-seizure HF chains). |
| `helpers::calculate_health_factor` (`helpers/mod.rs:55-115`) | `calculate_health_factor_summary` (113-122) | `hf >= 0` *(weak)* | ✓ sound | The bound `hf >= 0` is so weak that *no* health-factor rule that consumes it (`health_rules::hf_safe_after_borrow`, `liquidation_at_hf_exactly_one`, etc.) constrains anything the prover can use. Production also guarantees `hf == i128::MAX` when `borrow_positions.is_empty()` (line 64) — uncaptured. Production guarantees `hf >= 0` follows from `weighted_coll * WAD / total_borrow` with non-negative numerator and positive denominator (lines 86-113) — uncaptured detail. Every domain rule that asserts `hf_after >= hf_before`, `hf >= WAD`, or `hf < WAD` after a state transition is **vacuously refutable**: prover picks `hf_before = 100`, `hf_after = 0`. |
| `helpers::calculate_health_factor_for` (`helpers/mod.rs:117-133`) | `calculate_health_factor_for_summary` (124-137) | `hf >= 0` *(weak)* | ✓ sound | `#[cfg(feature = "certora")]`-only function exists *only* for the prover (`grep` confirms zero callers outside `certora/spec/`). Same weakness as above. **All 6 callers in `health_rules.rs`, 1 in `liquidation_rules.rs`, 1 in `strategy_rules.rs`, 2 in `boundary_rules.rs:215, 237` invoke the nondet summary, not the production wrapper.** The boundary-rule comment at `boundary_rules.rs:204-211` is misleading. |
| `helpers::calculate_account_totals` (`helpers/mod.rs:139-186`) | `calculate_account_totals_summary` (145-163) | `(coll, weighted, debt)` returned in **wrong tuple order** vs production `(coll, debt, weighted)` ✗ | ✗ **broken** | See finding 1 above. The bound `weighted_coll_raw <= total_collateral_raw` (line 157) is applied to the *second* tuple slot. Production callers at `liquidation.rs:168, 437, 470` and `views.rs:251` destructure as `(coll, debt, weighted)`. Under summarisation, every such destructure binds the wrong values. **Critical soundness defect**: the LTV-≤-collateral invariant is lost; the debt-vs-collateral inequality is randomised. |
| `helpers::calculate_linear_bonus` (`helpers/mod.rs:222-228`) | `calculate_linear_bonus_summary` (174-184) | `bonus ∈ [base_bonus, max_bonus]` ✓ | ⚠ admits all rules' assertions to fail | Production at HF == target_hf returns *exactly* `base_bonus` (the gap is zero). The summary admits *any* value in `[base, max]`. `boundary_rules::bonus_at_hf_exactly_102` (rule 8) asserts `|bonus - base| <= 1` — refutable by `bonus = max`. Either the rule fails (then HANDOFF's "passing rules" claim is wrong) or the prover encodes some special-case that hides this — either way the rule does not formally verify the intended boundary. |
| `views::total_collateral_in_usd` (`views.rs:111-141`) | `total_collateral_in_usd_summary` (195-199) | `total >= 0` *(weak)* | ✓ sound | Production also guarantees `== 0` when account meta absent (line 114-116) and `== 0` when supply map empty (line 118-120). Rules that branch on these zeros (e.g. `bad_debt_at_exactly_5_usd` assumes `== 5*WAD`) lose the zero-branch information. Acceptable for the boundary rule because of the explicit `cvlr_assume`, but vacuously-passing for any rule that doesn't pin the value. |
| `views::total_borrow_in_usd` (`views.rs:143-173`) | `total_borrow_in_usd_summary` (202-206) | `total >= 0` *(weak)* | ✓ sound | Same as above. |
| `views::ltv_collateral_in_usd` (`views.rs:260-270`) | `ltv_collateral_in_usd_summary` (214-218) | `total >= 0` *(weak)* | ⚠ misses production invariant | The doc comment at `summaries/mod.rs:208-213` *correctly identifies* a strong production invariant — "the result is bounded by `total_collateral_in_usd`" — but the summary does **not** encode it (no `cvlr_assume!(total <= total_collateral_in_usd(account))`). A future rule asserting `ltv_collateral <= total_collateral` cannot use the summary's contract. |

### Pool cross-contract calls — *zero summaries currently exist*

These should exist per the rubric. Each entry below shows the call site, the production return contract, the prover-relevant bounds that should be encoded, and the domain rule(s) that pass-or-fail solely based on the absence.

| Production fn | Call sites | Returned-value bounds (production guarantees) | Soundness today | Affected rules |
|---|---|---|---|---|
| `pool::supply` | `positions/supply.rs:370` | `actual_amount == amount` (input pass-through, `pool/src/lib.rs:160`); `position.scaled_amount_ray = old + amount/index` (monotone increase, `lib.rs:144-148`); `market_index.borrow_index >= RAY`, `market_index.supply_index >= WAD` | ✗ no summary; havoc | `solvency_rules::supply_increases_supplied`, `position_rules::*supply*`, `health_rules::supply_cannot_decrease_hf` (rule 4) — pool can return any `actual_amount` so the post-state position is unconstrained. |
| `pool::borrow` | `positions/borrow.rs:263` | `actual_amount == amount` (`lib.rs:201`); `position.scaled_amount_ray = old + amount/borrow_index` (monotone increase); `cache.has_reserves(amount)` was checked before return (`lib.rs:177`) | ✗ no summary; havoc | `solvency_rules::*borrow*`, `health_rules::hf_safe_after_borrow` — production *guarantees* the borrow only succeeds when reserves cover it. Without a summary, the prover cannot use this. |
| `pool::repay` | `positions/repay.rs:106` | `actual_amount == min(amount, current_debt)` (`lib.rs:338`); `position.scaled_amount_ray = old - scaled_repay >= 0` (`lib.rs:319-322`); refund `= amount - current_debt` if `amount >= current_debt` else 0 | ✗ no summary; havoc | `solvency_rules::repay_decreases_borrowed`, `liquidation_rules::*` — refund-bounding invariant invisible. |
| `pool::withdraw` | `positions/withdraw.rs:137` | `actual_amount = gross_amount = min(requested, current_supply_actual)` (`lib.rs:226-243, 283`); position decremented by exact scaled amount; reserves checked (`lib.rs:256-258`) | ✗ no summary; havoc | `solvency_rules::*withdraw*`, `boundary_rules::withdraw_more_than_position` (rule 20) — the boundary rule re-implements the bound locally instead of invoking pool. A regression that drops the `min` cap is invisible to both. |
| `pool::seize_position` | `positions/liquidation.rs:527` | `position.scaled_amount_ray == 0` post-call (`lib.rs:530, 535`); pool's `borrowed`/`revenue` adjusted symmetrically; `supply_index` may rise (bad-debt socialisation) | ✗ no summary; havoc | `liquidation_rules::*seize*`, `solvency_rules::*bad_debt*` — the prover sees an unconstrained `AccountPosition` reply, missing the "scaled goes to zero" invariant that gates downstream HF. |
| `pool::create_strategy` | `positions/borrow.rs:56` (multiply path) | `actual_amount == amount`; `amount_received == amount - fee` (`lib.rs:506`); `0 <= fee <= amount` (`lib.rs:473`); reserves cover full amount | ✗ no summary; havoc | `strategy_rules::*` — the entire flash-then-borrow accounting (fee routes to revenue, scaled debt = amount/borrow_index) is havoc'd. `flash_loan_rules` cannot verify "fee == 0 round-trips with no debt change". |
| `pool::flash_loan_begin` / `flash_loan_end` | `flash_loan.rs:53, 62` | `begin`: pool snapshots `pre_balance`, transfers `amount` to receiver. `end`: receiver returns `amount + fee`; pool verifies `balance_after >= pre_balance + fee` (`lib.rs:447-449`); fee added to revenue | ✗ no summary; havoc | `flash_loan_rules::*` — the begin/end pairing invariant ("fee credited to revenue iff end succeeds") is uncaptured. |
| `pool::update_indexes` | `router.rs:237`, `utils.rs:143` | Returns `MarketIndex` with `supply_index_ray >= old`, `borrow_index_ray >= old` (monotone, `pool::interest::global_sync` is monotone) | ✗ no summary; havoc | `index_rules::*monotonic*` — production guarantees monotonicity through `global_sync`; without a summary the prover cannot use it. |
| `pool::add_rewards` | `router.rs:324` | Side-effect only: `supply_index` increases proportional to `amount/supplied`; panics if `supplied == 0` (`lib.rs:375-377`) | ✗ no summary; havoc | `interest_rules::*reward*` — the rewards-conservation invariant is lost. |
| `pool::claim_revenue` | `router.rs:282` | Returns `amount_to_transfer = min(reserves, treasury_actual)`; pool's revenue and supplied decrease by proportional `scaled_to_burn` (`lib.rs:570-579`); panics if `revenue > supplied` (`lib.rs:557-561`) | ✗ no summary; havoc | `solvency_rules::revenue_conservation` — the revenue-burns-from-supplied invariant is uncaptured. |
| `pool::get_sync_data` | `cache/mod.rs:153`, `positions/borrow.rs:378`, `positions/supply.rs:414`, `storage/certora.rs:79, 107, 122` | Returns `(params, state)` where `state.supplied_ray, borrowed_ray, revenue_ray, borrow_index_ray, supply_index_ray, last_timestamp`. Production guarantees `supplied >= 0`, `borrowed >= 0`, `borrow_index >= RAY`, `supply_index >= WAD`, `last_timestamp <= block_ts`. | ✗ no summary; havoc | Every rule that traverses cache: `health_rules::*`, `solvency_rules::*`, `interest_rules::*`, `boundary_rules::borrow_exact_reserves` (rule 19) — the boundary rule asserts `borrow_amount <= available_reserves` against locally-assumed values; the production check at `pool/src/lib.rs:177-179` is never invoked. |
| `pool::keepalive` | `router.rs:381` | Side-effect: extends instance TTL; no return contract beyond panic-or-not | ✗ no summary; havoc | TTL/keepalive rules in `fuzz_ttl_keepalive.rs` exist on the test side, none on the Certora side. |
| `pool::reserves`, `capital_utilisation`, `deposit_rate`, `borrow_rate`, `protocol_revenue`, `supplied_amount`, `borrowed_amount`, `delta_time` | (read-only views, called from controller views/strategy paths) | Pure derivations of `(params, state)`; deterministic given a synced state | ✗ no summary; havoc | View-level rules (none currently in spec) would have no anchor. |

### SAC (token) cross-contract calls — *zero summaries currently exist*

| Production fn | Call sites | Soundness today | Notes |
|---|---|---|---|
| `token::Client::transfer` | numerous in `flash_loan.rs`, `pool/src/lib.rs`, `positions/*.rs` | ✗ no summary; havoc on token call (transfer return is `()`, but balance side-effect is unmodeled). | The framework doesn't try to model token side-effects; this is the standard "external token contract" abstraction that Certora users typically encode via a `tokens_balance` ghost. None exists here. Affects: any rule reasoning about caller-balance conservation across a tx. |
| `token::Client::balance` | `pool/src/lib.rs:405, 443` (flash-loan pre/post snapshot), `cache::get_reserves_for` | ✗ no summary; havoc | Flash-loan repayment-verification rules need this to be linked to prior `transfer`s. Currently unlinked. |

---

## boundary_rules.rs

### `boundary_rules.rs::borrow_rate_at_exact_zero` and sanity

**Lines:** 59-74
**Severity:** strong
**Rubric items failed:** none

**Why:** Pure-arithmetic rule on `calculate_borrow_rate`, which is *not* summarised (verified by absence from `summaries/mod.rs`). The assertion `rate == base_rate / MILLISECONDS_PER_YEAR` invokes the production rate-model function via `common::rates::calculate_borrow_rate` (`boundary_rules.rs:21, 63`). Sound. One of the few boundary rules that actually exercise production.

**Notes:** `boundary_test_params(env)` reuses `env.current_contract_address()` as `asset_id` (line 45) — sound shortcut documented at lines 32-34.

---

### `boundary_rules.rs::borrow_rate_at_exact_mid` and sanity

**Lines:** 84-106
**Severity:** strong
**Rubric items failed:** none

**Why:** Same as above. Tests Region-1→Region-2 continuity at `U == mid`. Allows ±1 ulp for half-up rounding, which is appropriate for the production formula.

---

### `boundary_rules.rs::borrow_rate_at_exact_optimal` and sanity

**Lines:** 116-137
**Severity:** strong

**Why:** Region-2→Region-3 continuity. Sound.

---

### `boundary_rules.rs::borrow_rate_at_100_percent` and sanity

**Lines:** 146-161
**Severity:** strong

**Why:** Caps at `max_borrow_rate / MILLISECONDS_PER_YEAR`. Sound.

---

### `boundary_rules.rs::compound_interest_at_max_rate_max_time` and sanity

**Lines:** 170-192
**Severity:** strong

**Why:** Asserts `compound_interest(RAY/year, year) > RAY` and `< 100*RAY`. Production function (`common::rates::compound_interest`) is not summarised. Sanity tightens to `(2*RAY, 3*RAY)` — verifies `e^1 ≈ 2.718*RAY` falls in band. Sound.

---

### `boundary_rules.rs::liquidation_at_hf_exactly_one` and sanity

**Lines:** 212-227
**Severity:** **broken**
**Rubric items failed:** [2 — soundness of nondet bounds; 6 — boundary rules; HANDOFF drift]

**Why:**
1. **Calls a summary, claims to call production.** The comment block at lines 204-211 explicitly states: *"the rewritten rules constrain the real cached HF via cvlr_assume and then assert the liquidation-guard predicate against it."* The body at line 215 calls `crate::helpers::calculate_health_factor_for(&e, &mut cache, account_id)`. Under `apply_summary!` (active under the `certora` feature, which is the build context for *all* `.conf` files per `HANDOFF.md:58`), this call is rewritten to `crate::spec::summaries::calculate_health_factor_for_summary(...)` which returns `let hf: i128 = nondet(); cvlr_assume!(hf >= 0); hf` (`summaries/mod.rs:129-137`). The cache argument is dropped (`_cache`); the account_id is dropped (`_account_id`); no production logic is exercised.
2. **Tautology with extra steps.** Body reduces to: `hf := nondet >= 0; assume hf == WAD; assert hf >= WAD`. Provable by inspection. The rule was already a tautology *before* the rewrite (`hf < WAD` on `let hf = WAD`); after the rewrite it remains a tautology (`assume(hf == WAD) → assert(hf >= WAD)`). The HANDOFF document has no record of the tautology persisting.
3. **Does not catch the regression the comment claims.** *"a broken guard in production would have still passed"* — the new rule has the same property. If `process_liquidation`'s gate were changed to `if hf >= WAD - 10 { panic }` (a real risk regression), this rule would still pass: the summary returns `hf == WAD` per the assume, and the assertion `hf >= WAD` does not invoke `process_liquidation`.

**Affected domain rules:** any rule importing `crate::helpers::calculate_health_factor_for` — listed earlier under finding 2.

**Fix sketch:** invoke `crate::helpers::calculate_health_factor_for::calculate_health_factor_for(&e, ...)` (the unsummarised inner module path that `apply_summary!` preserves at `vendor/cvlr-soroban-macros/src/apply_summary.rs:3-9`). And then either invoke `process_liquidation` directly with a healthy account and assert it panics, or assert the predicate `process_liquidation` *would* check.

---

### `boundary_rules.rs::liquidation_at_hf_just_below_one` and sanity

**Lines:** 234-249
**Severity:** **broken**

**Why:** Same defect as rule 6. Body becomes `hf := nondet >= 0; assume hf == WAD - 1; assert hf < WAD`. Tautology; production guard never invoked.

---

### `boundary_rules.rs::bonus_at_hf_exactly_102` and sanity

**Lines:** 257-281
**Severity:** **broken** (or **vacuous-and-failing**, depending on prover encoding)
**Rubric items failed:** [3 — over/under-summarisation]

**Why:**
1. Calls `crate::helpers::calculate_linear_bonus(&e, hf=1.02 WAD, base=500, max=1000)`. Summary at `summaries/mod.rs:174-184` returns `let bonus_raw: i128 = nondet(); cvlr_assume!(bonus_raw >= 500); cvlr_assume!(bonus_raw <= 1000)`.
2. Production at `helpers/mod.rs:222-228` (gated by `#[cfg(feature = "certora")]`) returns `calculate_linear_bonus_with_target(env, hf, base, max, target=1.02 WAD)`. At `hf == target`, the body at `helpers/mod.rs:194-219` enters the `gap_numerator <= ZERO` branch (line 202) and returns `base` *exactly*.
3. The summary admits `bonus = max = 1000`. The rule asserts `|bonus - 500| <= 1`. Refutable.

Either:
- The Certora prover refutes the rule (verdict: fail), in which case HANDOFF.md's listing of "102 strong rules" overstates the passing rate.
- The prover treats this assert-on-summary-bounds as a soundness exemption — in which case the rule passes vacuously and a real regression in `calculate_linear_bonus_with_target` (e.g., the gap formula is off by a factor) would be invisible.

**Affected domain rules:** any liquidation rule that depends on the target-HF→base-bonus relationship.

**Fix sketch:** invoke the unsummarised `crate::helpers::calculate_linear_bonus::calculate_linear_bonus(...)` path. Or invoke `calculate_linear_bonus_with_target` directly (the inner non-summarised helper at `helpers/mod.rs:194`).

---

### `boundary_rules.rs::bad_debt_at_exactly_5_usd` and sanity

**Lines:** 294-315
**Severity:** **broken**
**Rubric items failed:** [3, 6]

**Why:**
1. Calls `views::total_collateral_in_usd` and `views::total_borrow_in_usd`, which are summarised to `nondet >= 0`. Both summaries discard `account_id` (`_account_id`).
2. The rule assumes `total_collateral_usd == 5*WAD` and `total_debt_usd > total_collateral_usd`, then asserts `qualifies := total_debt > total_coll && Wad(total_coll) <= 5*WAD` is true.
3. Body reduces to: `assume(coll == 5*WAD ∧ debt > coll) → assert(debt > coll ∧ coll <= 5*WAD)`. Tautology. The production predicate at `liquidation.rs:430` is *not* invoked. A regression that changes `<=` to `<` in production is invisible.

**Fix sketch:** call `crate::positions::liquidation::clean_bad_debt_standalone` (the unsummarised producer of the predicate) on a constructed account and assert the function dispatches to the bad-debt branch iff the predicate holds.

---

### `boundary_rules.rs::bad_debt_at_6_usd` and sanity

**Lines:** 322-342
**Severity:** **broken**

**Why:** Same defect as rule 9. Tautology.

---

### `boundary_rules.rs::mul_at_max_i128` and sanity

**Lines:** 354-372
**Severity:** strong

**Why:** Pure-arithmetic test on `mul_div_half_up(i128::MAX/RAY, RAY, RAY) ≈ i128::MAX/RAY`. The production function is in `common::fp_core` and is not summarised. The bounded ±1 ulp tolerance handles half-up rounding. Sound.

**Notes:** This rule actually exercises the I256 boundary that the summaries claim to abstract. It is one of the few boundary rules that delivers what its comment promises.

---

### `boundary_rules.rs::compound_taylor_accuracy` and sanity

**Lines:** 381-412
**Severity:** strong

**Why:** Pure-arithmetic test on `compound_interest`. Asserts `factor >= 1.01*RAY` and `< 1.0101*RAY`. Sound.

---

### `boundary_rules.rs::rescale_ray_to_wad`

**Lines:** 420-424
**Severity:** strong (no sanity counterpart; pure assertion)

**Why:** `rescale_half_up(RAY, 27, 18) == WAD`. Pure arithmetic on a non-summarised function. Sound.

---

### `boundary_rules.rs::rescale_wad_to_7_decimals`

**Lines:** 432-437
**Severity:** strong

**Why:** `rescale_half_up(WAD, 18, 7) == 10^7`. Sound.

---

### `boundary_rules.rs::tolerance_at_exact_first_bound` and sanity

**Lines:** 449-474
**Severity:** **weak**
**Rubric items failed:** [6 — boundary rules]

**Why:**
1. Pure-arithmetic identity. Body assumes `deviation == first_tolerance` and asserts `deviation <= first_tolerance` (true by reflexivity) and `!(deviation > first_tolerance && deviation <= second_tolerance)` (which simplifies to `!(false && ...) = true`).
2. **Does not invoke production.** The actual tier-discrimination logic lives in `oracle/mod.rs::is_within_anchor` and `calculate_final_price` (`oracle/mod.rs:114-157, 372-392`). Neither is called from this rule. A regression that flips the production branch to `<` would not be caught.

**Fix sketch:** invoke `is_within_anchor` (currently summarised to nondet bool — also problematic) or, better, the unsummarised `calculate_final_price` directly with `(agg, safe, config)` and assert the returned price equals `safe` when `is_within_anchor(...,first)` holds.

---

### `boundary_rules.rs::tolerance_at_exact_second_bound` and sanity

**Lines:** 482-506
**Severity:** weak

**Why:** Same defect as rule 15.

---

### `boundary_rules.rs::tolerance_just_beyond_second` and sanity

**Lines:** 514-539
**Severity:** weak

**Why:** Same defect. Asserts `deviation > second_tolerance` after assuming `deviation == second_tolerance + 1` — refl tautology.

---

### `boundary_rules.rs::supply_dust_amount` and sanity

**Lines:** 552-568
**Severity:** strong

**Why:** Pure-arithmetic test of `mul_div_half_up(1, RAY, RAY) > 0`. Sound. Verifies the dust-doesn't-zero-out invariant.

**Note:** A stronger version would also assert that the *production* `supply` path produces a non-zero scaled position when given `amount == 1`. That requires a `pool::supply` summary (currently absent) and would catch a regression that changed the rounding mode.

---

### `boundary_rules.rs::borrow_exact_reserves` and sanity

**Lines:** 577-600
**Severity:** **weak**

**Why:**
1. Body assumes `borrow_amount == available_reserves` and `available_reserves > 0`, then asserts `!(borrow_amount > available_reserves)` — which is `!(false)` = `true`. Reflexivity.
2. Production has the *real* check at `pool/src/lib.rs:177-179` (`if !cache.has_reserves(amount) { panic InsufficientLiquidity }`). The rule does not invoke this. A regression that changes `>` to `>=` is invisible.

**Note:** Comment at lines 583-585 is candid that the bound was tightened from `i128::MAX / 2` to `10 * RAY` to placate prover timeouts. This is a symptom of the larger "no pool summary" problem: the prover cannot reason about pool boundaries, so boundary rules tighten domains until they fit.

---

### `boundary_rules.rs::withdraw_more_than_position` and sanity

**Lines:** 609-629
**Severity:** **weak**

**Why:**
1. Body assumes `requested > position_value > 0`, computes `actual := requested.min(position_value)`, asserts `actual == position_value`. Pure arithmetic on local values.
2. Production has the real cap at `pool/src/lib.rs:226-243` (`if amount >= current_supply_actual { ... }`). Not invoked.

---

## summaries/mod.rs

### `token_price_summary`

**Lines:** 50-62
**Severity:** weak (sound but loses determinism)
**Rubric items failed:** [2, 4]

**Why:** Bounds correctly capture `price_wad > 0`, `asset_decimals <= 27`, future-timestamp clamp. **Misses** two production-guaranteed equalities that domain rules need:
- `asset_decimals == cached_market_config(asset).oracle_config.asset_decimals` (production at `oracle/mod.rs:54`).
- `timestamp == cache.current_timestamp_ms / 1000` (production at `oracle/mod.rs:55`).

The summary lets these be any nondet value satisfying the inequalities. Rules in `oracle_rules.rs` that compare returned `asset_decimals` against the configured value, or assert `timestamp == now/1000`, would *fail under summary* despite production guaranteeing the equality.

**Aliasing:** Production at `oracle/mod.rs:61` writes `cache.set_price(asset, &feed)`. Summary does NOT write to the cache (`_cache: &mut ControllerCache` — argument unused in body). A subsequent `cache.try_get_price(asset)` after the summary would return `None` instead of the price the summary returned. Any rule chaining `token_price` → `cache.try_get_price` is unsound under summarisation.

**Affected domain rules:** `oracle_rules::price_cache_consistency`.

---

### `is_within_anchor_summary`

**Lines:** 69-77
**Severity:** weak

**Why:** Returns `nondet bool` with no constraints. Production at `oracle/mod.rs:381-391` is deterministic on `(aggregator, safe, upper, lower)`. The summary admits both branches for any input. Sound (loses information). Acceptable for prover-cost reasons but limits any rule that reasons about "same input → same answer".

---

### `update_asset_index_summary`

**Lines:** 88-101
**Severity:** **broken**
**Rubric items failed:** [3 — over-summarisation]

**Why:** Bound `borrow_index_ray >= supply_index_ray` (line 96) is **not** a production invariant. Production at `pool/src/lib.rs:521-525` (`apply_bad_debt_to_supply_index`) socialises bad debt by *increasing* `supply_index` proportionally to the bad-debt scaled amount divided by total supplied — this can push `supply_index > borrow_index` momentarily.

**Failure mode:** any rule that follows a `seize_position` or `clean_bad_debt` flow with a HF/index check will read a `MarketIndex` from the summary that satisfies `borrow >= supply`, an invariant production has just *broken*. The rule's verdict is therefore *unsound under post-bad-debt scenarios*.

**Affected domain rules:** `index_rules::supply_index_monotonic`, `liquidation_rules::*post_seize*`, `solvency_rules::bad_debt_socialisation_*`.

**Fix sketch:** drop the `borrow_index_ray >= supply_index_ray` bound. Add `borrow_index_ray <= MAX_BORROW_INDEX_RAY` if a ceiling is needed. Add a parallel summary or precondition for `apply_bad_debt_to_supply_index` so post-seizure reasoning has a contract.

---

### `calculate_health_factor_summary`

**Lines:** 113-122
**Severity:** weak

**Why:** Returns `hf >= 0`. Sound but loses every other production guarantee:
- `hf == i128::MAX` when `borrow_positions.is_empty()` (production line 64).
- `hf == i128::MAX` when `total_borrow == 0` (line 100).
- `hf` ordering preserved across position-mutating operations (the property `solvency_rules` and `health_rules` actually want).

**Affected domain rules:** every rule that asserts a relationship between `hf_before` and `hf_after`. The summary returns *independent* nondet values — no relationship can be expressed.

**Fix sketch:** add a ghost variable for "current account weighted-collateral" and "current account total-borrow"; have the summary derive `hf` as a deterministic function of those. Then a rule can verify the relationship by reasoning about the ghosts.

---

### `calculate_health_factor_for_summary`

**Lines:** 124-137
**Severity:** weak (same defect as above; this is the per-account wrapper)

**Why:** Same. Compounded by the fact that `calculate_health_factor_for` is a `#[cfg(feature = "certora")]`-only function — it exists *only* to be summarised. There is no production analogue to rebut the summary against.

---

### `calculate_account_totals_summary`

**Lines:** 145-163
**Severity:** **broken** (severity HIGH)
**Rubric items failed:** [2, 3, 4]

**Why:** Tuple-order bug. Production at `helpers/mod.rs:184` returns `(total_collateral, total_debt, weighted_coll)`. Summary at lines 158-162 returns `(total_collateral, weighted_coll, total_debt)`. **The second and third tuple elements are swapped vs. production.**

Production callers (verified):
- `liquidation.rs:168`: `let (total_collateral, total_debt, weighted_coll) = ...` — under summary, `total_debt` binds to `weighted_coll_raw`, `weighted_coll` binds to `total_debt_raw`.
- `liquidation.rs:437, 470`: `let (total_collateral_usd, total_debt_usd, _) = ...` — under summary, `total_debt_usd` binds to `weighted_coll_raw`.
- `views.rs:251`: `let (_, _, weighted_coll) = ...` — under summary, `weighted_coll` binds to `total_debt_raw`.

The `cvlr_assume!(weighted_coll_raw <= total_collateral_raw)` at line 157 is therefore applied to the *wrong tuple slot*: it constrains the second slot, but production callers expect the constraint on the third. Additionally, `total_debt_raw` is unconstrained vs. `total_collateral_raw` — but production has no such bound either, so this part is sound.

**Net effect:**
- `liquidation_rules::*` that test "weighted_coll <= total_collateral after seize" actually test "total_debt <= total_collateral", which production does NOT guarantee (debt can exceed collateral when HF < 1).
- `views::liquidation_collateral_available` returns `total_debt_raw` to the caller instead of `weighted_coll_raw`. A view client would receive the wrong number.

**Affected domain rules:** every rule that consumes any of the three values. Specifically:
- `liquidation_rules::seize_proportions_correct`
- `solvency_rules::*` (any using `liquidation_collateral_available`)
- `health_rules::*` indirectly via `calculate_account_totals` chains.

**Fix sketch:** swap lines 159-161 so the summary returns `(coll, debt, weighted)` matching production. Add the missing bound `cvlr_assume!(total_debt_raw >= 0)` (already present at line 156, good).

---

### `calculate_linear_bonus_summary`

**Lines:** 174-184
**Severity:** **broken under boundary rule 8**
**Rubric items failed:** [3]

**Why:** See finding 3 and `boundary_rules::bonus_at_hf_exactly_102`. The bound `bonus ∈ [base, max]` is the *envelope*, not the *value*. Production returns *exactly* `base` when `hf >= target_hf` (a knowable invariant). The summary discards this. Rule 8 then asserts `bonus == base` (within ulp), which the summary cannot satisfy.

**Affected domain rules:** `liquidation_rules::*bonus*` (if any).

---

### `total_collateral_in_usd_summary`, `total_borrow_in_usd_summary`, `ltv_collateral_in_usd_summary`

**Lines:** 195-218
**Severity:** weak (collateral, borrow); broken (ltv)

**Why:**
- The first two miss the zero-account and empty-map branches (production lines 114-116, 118-120, 146-148, 150-152). Acceptable looseness for boundary rules that explicitly assume specific values.
- `ltv_collateral_in_usd_summary` at lines 214-218 has a doc comment (lines 208-213) that *correctly identifies* a strong production invariant: "the per-asset weight at `loan_to_value_bps`, so the result is bounded by `total_collateral_in_usd`." The summary does not encode this bound. A rule asserting `ltv <= total_collateral` cannot use the summary.

**Fix sketch:** add `cvlr_assume!(total <= total_collateral_in_usd_for_account)` — but this requires linking the two summaries, which they don't currently do (each is independent nondet).

---

## compat.rs

### Compat shims overview

**Lines:** 1-84
**Severity:** weak (collectively); 1 broken (`multiply` argument compression)

**Purpose:** flatten variadic `Vec<(Address, i128)>` payments into single-asset (asset, amount) tuples for prover ergonomics. Maps `PositionMode` enum from `u32` for nondet-able input. Drops some optional arguments.

**Findings:**

1. **`supply_single`, `borrow_single`, `withdraw_single`, `repay_single` (lines 4-30)** — These build a single-element `Vec<&env, (asset, amount)>` and forward to the variadic Controller entry points. Sound shim; sound for testing single-asset operations. Misses: multi-asset operations (e.g., a borrow that touches two assets in one transaction) — every multi-asset coverage scenario is unreachable through these shims.

2. **`multiply` (lines 32-63)** — drops the `account_id: u64` argument by passing the literal `0` (line 53). Production at `controller::Controller::multiply` accepts an account_id. The literal `0` always means *new account*. Domain rules that reuse an existing account in multiply flows are unreachable through this shim. Also drops the trailing `(None, None)` pair (lines 60-61) for what appears to be the optional `(min_collateral_received, max_debt_in)` parameters; if those are slippage guards, the shim hides slippage-bypass scenarios. Severity: medium-broken; the rule designer must call `Controller::multiply` directly to reuse accounts.

3. **`repay_debt_with_collateral` (lines 65-84)** — passes `false` for the trailing `bool` argument. Production at `controller::Controller::repay_debt_with_collateral` exposes that flag (semantic unknown without context — the call signature here is the only evidence). Hard-coding `false` removes one branch from coverage.

4. **No shim for liquidation, no shim for flash-loan-end pairing.** `liquidation_rules.rs` calls `crate::positions::liquidation::process_liquidation` directly (`liquidation_rules.rs:79`) — that module-internal path bypasses the public `Controller::liquidate` entry point and any auth/validation gates that wrap it. This is *sound for verifying inner logic* but *unsound for verifying the public-API contract*. There should be a `compat::liquidate` shim that goes through `Controller::liquidate`.

5. **Module is not exported from `mod.rs:17`** — wait, it is (`pub mod compat`). Sound.

---

## mod.rs

### `mod.rs` — module registration

**Lines:** 1-31
**Severity:** nit
**Rubric items failed:** none structural; HANDOFF drift

**Findings:**

1. **`math_rules` is registered (line 25) but not documented in the doc-comment** (lines 1-15 list 11 modules; the module declarations actually register 13). The doc comment is stale.
2. **`model.rs` is referenced in `HANDOFF.md:149`** as `model.rs # ghost variables (currently unused)`, but the file does not exist (`ls controller/certora/spec/` confirms). HANDOFF drift.
3. **No `#![cfg(feature = "certora")]` guard inside `mod.rs`.** The guard is applied at the parent (`controller/src/lib.rs:42-44`: `#[cfg(feature = "certora")] pub mod spec`), so this is sound. But a future contributor who tries to use any spec module from a non-certora build would get a compile error one level higher than expected.

---

## Coverage gaps — invariants the framework cannot express today

### Missing-1: Pool supply/borrow/withdraw/repay summaries

Detailed in the table above. The complete absence of pool-call summaries means *every domain rule that traverses* `Controller::supply/borrow/withdraw/repay/multiply/liquidate/claim_revenue` *via `compat`* either passes vacuously (because the prover havoc satisfies whatever post-condition is asserted) or fails for the wrong reason (because the prover cannot prove a property the production code guarantees through pool's internal logic). Most affected: `health_rules::hf_safe_after_borrow`, `health_rules::supply_cannot_decrease_hf`, every `solvency_rules::*`, every `position_rules::*`.

### Missing-2: Flash-loan begin/end pairing summary

`flash_loan_begin` snapshots `pre_balance`; `flash_loan_end` verifies `balance_after >= pre_balance + fee`. Without a summary that links the two, `flash_loan_rules::*` cannot verify the round-trip.

### Missing-3: `cached_pool_sync_data` summary or Cache method summary

The cache reads pool state via `pool_client.get_sync_data()` at `cache/mod.rs:153`. This is *not* summarised. Every rule that constructs a `ControllerCache` and reads `cached_pool_sync_data` either directly (`positions/borrow.rs:378`, `positions/supply.rs:414`) or transitively does a real cross-contract havoc.

### Missing-4: SAC (token) transfer/balance ghost

No ghost variable models user balances. Rules cannot assert "caller balance increased by `withdraw.actual_amount`" or "pool balance decreased by `flash_loan.amount`".

### Missing-5: AccountMeta lifecycle invariants

The slim-AccountMeta + side-map refactor introduces a new invariant: *no `SupplyPositions(id)` or `BorrowPositions(id)` storage entry exists when `AccountMeta(id)` is absent*. The `remove_account_entry` path at `storage/account.rs:198-203` removes all three keys atomically. The `set_account` path at lines 190-195 writes all three. **No Certora rule asserts this invariant.** A regression in `remove_account_entry` that forgets to remove a side map would orphan position data; the framework cannot detect it.

### Missing-6: TTL chain invariant

`storage/account.rs:50-53`: any side-map write bumps `AccountMeta` TTL. The intent is "side maps cannot outlive meta in TTL terms". **No Certora rule** captures this. The `fuzz_ttl_keepalive.rs` integration test exercises *some* TTL behavior, but the formal invariant is unstated.

### Missing-7: ControllerKey collision-freeness

The enum at `common/src/types.rs:552-573` has 16 variants. Some are parameterised by `Address` (`Market`, `AssetEModes`, `IsolatedDebt`), some by `u32` (`EModeCategory`, `EModeAssets`, `PoolsList`), some by `u64` (`AccountMeta`, `SupplyPositions`, `BorrowPositions`). Soroban's `#[contracttype]` discriminates by enum tag, so collisions are structurally impossible — but **no rule asserts this property**. If a future refactor merged two variants by accident (e.g., used the same discriminant), the framework would not catch it.

### Missing-8: `bump_user` idempotency / monotonicity

`storage/account.rs:206-220` (`bump_account`) bumps three keys' TTLs. **No rule** asserts that calling it twice in the same transaction is equivalent to calling it once.

### Missing-9: Pool's monotone-index invariant after `seize_position`

`pool::seize_position` at `pool/src/lib.rs:510-545` may *increase* `supply_index` (bad-debt socialisation, line 524) or *not* change it (deposit-side seizure, line 532-535). The rules do not cover the asymmetry — and the `update_asset_index_summary` actively *forbids* the `supply_index > borrow_index` post-state via the broken constraint at line 96.

### Missing-10: `claim_revenue` revenue-conservation

Pool's `claim_revenue` at `pool/src/lib.rs:547-600` decrements `revenue` and `supplied` by the *same* `scaled_to_burn` amount. This conservation property is the linchpin of solvency-after-revenue-claim. **No rule** asserts it.

### Missing-11: `add_rewards` proportional-distribution invariant

Pool's `add_rewards` at `pool/src/lib.rs:367-387` increases `supply_index` such that all suppliers receive a proportional share. **No rule** asserts the proportionality property.

### Missing-12: Multiply's flash-loan-then-borrow accounting

`Controller::multiply` issues a flash loan via `pool::create_strategy`, swaps via aggregator, supplies the result, repays the flash loan. The composite invariant is "net debt position increases by `debt_to_flash_loan + fees`; net collateral position increases by `swap_output`". **No rule** asserts this composite.

### Missing-13: Reflector summaries

Every Reflector cross-contract call (`cex_spot_price`, `cex_twap_price`, `dex_spot_price` in `oracle/reflector.rs` and consumed throughout `oracle/mod.rs`) is havoc'd to the prover. No bound on Reflector's `(price, timestamp)` reply means the staleness/tolerance framework cannot reason about real Reflector behaviour.

### Missing-14: `apply_summary!` "unsummarised path is reachable" lemma

The macro at `vendor/cvlr-soroban-macros/src/apply_summary.rs:3-9` creates `pub(crate) mod $id { fn $id(...) body }` so that `crate::path::fn::fn(...)` invokes the unsummarised body. The summaries module's doc comment at `summaries/mod.rs:23-26` claims rules verify summary correctness by invoking the unsummarised path. **No rule does this.** The "verifying the summary itself" mechanism is documented but unused.

---

## HANDOFF.md alignment

| HANDOFF claim | Reality | Severity |
|---|---|---|
| `summaries/mod.rs` is "empty placeholder" (`HANDOFF.md:155`) | File is 218 lines, 9 active summaries wired via `summarized!` from oracle/helpers/views | broken |
| `model.rs` exists with "ghost variables (currently unused)" (`HANDOFF.md:149`) | File does not exist | nit |
| `apply_summary!` wrappers at pool / oracle / SAC call sites are "Pending" (`HANDOFF.md:126`) | Wrappers exist for oracle (3) and helpers/views (6); **none** for pool or SAC | broken (item documented Pending; the partial work has soundness defects not noted) |
| 102 strong rules (`HANDOFF.md:108`) | At least 4 rules in `boundary_rules.rs` (rules 6, 7, 9, 10) and rule 8 are **broken** under summarisation; comment in `boundary_rules.rs:204-211` claims they were "rewritten from local-constant tautologies to invoke the real helper" — they were not | weak (HANDOFF count is overstated) |
| 16 tautological rules listed (`HANDOFF.md:107`) | True at the *categorisation* level, but boundary rules 6-10 are not in that count and behave tautologically | weak |
| Recommended priority order: `boundary.conf` last (`HANDOFF.md:97`) | Sound — boundary rules are the cheapest verdict-wise but the summary defects make the *summary fixes* (item 1 + 6 above) higher-priority than the rule fixes | nit |
| Vendored CVLR resolves prior compile blockers (`HANDOFF.md:21-25`) | Confirmed — `vendor/cvlr/` and `vendor/cvlr-soroban/` directories exist and contain the required crates | sound |

---

## Recommended remediation priorities (meta-review level)

1. **Fix `calculate_account_totals_summary` tuple ordering** (`summaries/mod.rs:158-162`). High severity — every liquidation rule observes wrong values today. One-line edit; swap lines 160 and 161.

2. **Drop `borrow_index_ray >= supply_index_ray` from `update_asset_index_summary`** (line 96). High severity — over-constrains post-bad-debt scenarios that production allows.

3. **Reroute boundary rules 6, 7, 8, 9, 10 to the unsummarised module path.** Use `crate::helpers::calculate_health_factor_for::calculate_health_factor_for(&e, ...)` (note the doubled module name — the inner module nested by `apply_summary!`). Update the misleading comment block at lines 204-211 to reflect what the rules actually verify.

4. **Add pool cross-contract summaries** (`pool::supply`, `pool::borrow`, `pool::withdraw`, `pool::repay`, `pool::seize_position`, `pool::create_strategy`, `pool::flash_loan_begin/end`, `pool::get_sync_data`). The `summaries/mod.rs` doc comment at lines 9-13 already cites this as a goal ("Cross-contract `LiquidityPoolClient` calls are pure havoc to the prover; explicit nondet returns are equivalent semantically and orders of magnitude cheaper"). The summaries do not exist.

5. **Update HANDOFF.md** to reflect actual framework state. The three drift entries (`summaries/mod.rs` is not empty; `model.rs` does not exist; `apply_summary!` wrappers are partially deployed, not Pending) mislead the engagement team.

6. **Add AccountMeta lifecycle / side-map TTL invariant rules** to either `solvency_rules.rs` or a new `storage_rules.rs`. Specifically:
   - `remove_account_entry` removes all three (`AccountMeta`, `SupplyPositions`, `BorrowPositions`) keys.
   - Side-map writes bump `AccountMeta` TTL.
   - `set_account` and `account_from_parts` round-trip without value loss.

7. **Add `compat::liquidate` shim** routing through `Controller::liquidate` (not `process_liquidation`). The current direct call to the inner module bypasses the public-API auth gate.

8. **Boundary rules 15-20 (oracle tolerance + reserve/withdraw bounds)** should invoke production predicates rather than re-implementing the inequality locally. Without this, they verify nothing about the implementation. As written they are pure tautologies (assume `P` then assert `P`).

9. **Drop the `model.rs` reference from HANDOFF.md** or recreate the file if ghost variables are needed for the recommended `calculate_health_factor_summary` linkage.

10. **Remove or document the dead `math_rules` doc-comment gap in `mod.rs`.** Trivial; aligns the source of truth.

---

## Net assessment

| Category | Assessment |
|---|---|
| Boundary rule infrastructure correctness | **broken** — 5 of 19 main rules tautologise via summarisation; 4 more re-implement-and-assert without invoking production |
| Summary soundness | **broken** — 1 critical tuple-order bug; 1 over-constraint vs. production invariant; 7 over-weak vs. production guarantees |
| Pool cross-contract coverage | **missing** — 0 of 22 trait methods summarised; entire pool boundary is havoc |
| SAC/token coverage | **missing** — no balance ghost, no transfer model |
| Storage refactor compatibility | **sound** — no boundary rule reads new storage keys directly; refactor does not regress framework |
| HANDOFF.md alignment | **drifted** — 3 load-bearing claims do not match the codebase |
| Compat shim correctness | **weak** — multiply drops account_id and slippage params; repay_debt_with_collateral hard-codes a flag |

The Certora framework as configured today does not justify the formal-verification confidence level the audit-prep status implies. The summary defects (especially the tuple swap) should be treated as audit-blocking findings that must land before the engagement team's run, because every domain rule downstream of those summaries reasons over a misconfigured abstraction.
