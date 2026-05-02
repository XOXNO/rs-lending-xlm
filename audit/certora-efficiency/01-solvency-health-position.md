# Efficiency Audit — Solvency / Health / Position

**Files audited (this pass):**
- `controller/certora/spec/solvency_rules.rs` (993 lines, 21 substantive rules + 5 sanity)
- `controller/certora/spec/health_rules.rs` (129 lines, 4 substantive rules + 2 sanity)
- `controller/certora/spec/position_rules.rs` (150 lines, 5 substantive rules + 1 sanity)
- Cross-referenced summaries: `controller/certora/spec/summaries/{mod,pool,sac}.rs`
- Production: `controller/src/positions/{supply,borrow,repay,withdraw,liquidation}.rs`,
  `controller/src/helpers/mod.rs`, `controller/src/cache/mod.rs`,
  `controller/src/views.rs`, `pool/src/lib.rs`.

**Rules examined (substantive only):** 30
**Recommendation tally:** keep=8 tighten=10 rewrite-as-action=4 split=2 delete=6

The single dominant scope-cost driver across this domain is the unsummarised
cross-contract `LiquidityPoolClient` view set — `reserves()`,
`supplied_amount()`, `borrowed_amount()`, `protocol_revenue()`,
`capital_utilisation()`. Each call is pure havoc; in the rules below they
appear up to four times per rule and the prover treats every occurrence as a
new free variable. This is the main lever for budget reduction (see
*Cross-cutting recommendations*).

The other dominant cost is the unbounded symbolic `account_id: u64` paired
with the symbolic `supply_positions` / `borrow_positions` `Map`s the production
code iterates inside `process_supply` / `process_borrow` / `process_repay`.
Even with the HF / totals summaries in place, every pre/post position read
still goes through `storage::positions::get_scaled_amount`, which is one
storage havoc per call. Concrete `account_id = 1` and a single-asset focus
roughly halves the symbolic state per rule.

---

## Per-rule classification

### `solvency_rules::pool_reserves_cover_net_supply` (line 36)
**Action:** delete (or replace with action-focused transition rule).
**Current scope cost:** medium — three unsummarised cross-contract reads, no
operation under test, no decision branches; ~3 free symbols + storage havoc.
**Verification value:** zero. Both the LHS and the RHS are independent havocs
returned by the prover for `pool_client.reserves()` /
`borrowed_amount()` / `supplied_amount()`. With no joint summary tying them
together (none exists in `summaries/pool.rs` — only the *mutating* pool
endpoints are summarised), PASS does not mean the production identity holds;
it just means the prover happened not to pick a counter-example for that one
snapshot.
**Issue:** pure state-invariant rule over unconstrained havoc.
**Proposed change:** delete. If the property is wanted, the right home is a
**transition rule** that takes `pre = (reserves, supplied, borrowed)` and the
post-state after one `supply` / `borrow` / `repay` and asserts the identity
is preserved, with the pool view set summarised jointly so the snapshot is
internally consistent (see *Cross-cutting* for the joint-summary sketch).

### `solvency_rules::revenue_subset_of_supplied` (line 57)
**Action:** delete.
**Current scope cost:** low (two unsummarised pool reads, no op).
**Verification value:** zero — same defect: independent havocs.
**Proposed change:** delete; subsumed by the transition rule above once the
joint pool-view summary is in place.

### `solvency_rules::borrowed_lte_supplied` (line 79)
**Action:** delete.
**Current scope cost:** low (three unsummarised pool reads, no op).
**Verification value:** zero — same defect.
**Proposed change:** delete; covered by the transition rule above.

### `solvency_rules::claim_revenue_bounded_by_reserves` (line 102)
**Action:** keep + tighten the summary, not the rule.
**Current scope cost:** low — `claim_revenue_summary` already exists at
`summaries/pool.rs:319`, but the summary returns `amount >= 0` only;
`pool_client.reserves()` is still unsummarised so `pre_reserves` is an
independent havoc.
**Verification value:** PASS today does not actually prove the bound. With a
joint summary on `reserves()` it would prove the bound.
**Issue:** missing summary on the `reserves()` view, not in the rule body.
**Proposed change:** add a `reserves_summary(env) -> i128` to
`summaries/pool.rs` returning a nondet `>= 0` and *also* tighten
`claim_revenue_summary` to assume `result <= last_reserves_seen` via a
small per-tx state in the summary module (or capture `pre_reserves` as
`cache.cached_pool_sync_data(asset).state.supplied_ray + reserves_delta`,
whichever is cheaper). The rule stays as a one-liner.

### `solvency_rules::utilization_zero_when_supplied_zero` (line 127)
**Action:** rewrite-as-unit-test (delete from this spec set).
**Current scope cost:** low — two unsummarised pool reads.
**Verification value:** zero — same independent-havoc defect: `get_sync_data`
and `capital_utilisation` are unsummarised, so the assume on
`sync.state.supplied_ray == 0` constrains one havoc and the assert is on
another.
**Proposed change:** delete from the controller spec. `capital_utilisation`
lives in `pool/src/views.rs`; the right place to verify this is a
unit/property test in the pool crate (`pool/src/views.rs` already has the
guard) or a Certora rule run against the *pool* crate, not the controller.

### `solvency_rules::isolation_debt_never_negative_after_repay` (line 145)
**Action:** keep.
**Current scope cost:** medium — one full `process_repay` traversal but with
the pool `repay_summary` already in place; storage reads are bounded.
**Verification value:** PASS proves the production clamp at
`controller/src/utils.rs:61-92` plus the dust-erasure rounding hold across one
repay.
**Issue:** none.
**Proposed change:** none.

### `solvency_rules::borrow_respects_reserves` (line 169)
**Action:** tighten — pin `account_id = 1`, single-asset.
**Current scope cost:** high. `account_id: u64` is fully symbolic, the borrow
flow loads borrow + supply position maps (full symbolic Map iteration in
unsummarised parts of `process_borrow`), and `pre_reserves` is an
unsummarised pool view (independent havoc from the `borrow_summary`'s
internal accounting, so the rule is again checking *havoc1 >= amount* after
*havoc2-driven success*).
**Verification value:** PASS as written tells us almost nothing — the
production reserves check is at `pool/src/lib.rs:177-179` which the
`borrow_summary` does not encode. The whole point of the rule is to
prove that exact pool-side guard, but the summary erases it.
**Issue:** unsummarised `reserves()`; full symbolic account state.
**Proposed change:** keep the rule, but (a) add a `reserves_summary` and
make `borrow_summary` assume `amount <= pre_reserves` via a captured
snapshot, OR (b) replace with a smaller unit-style rule against
`pool::LiquidityPool::borrow` directly (the guard is at the pool layer; this
controller-side rule is the wrong layer). Pinning `account_id = 1` cuts the
symbolic-Map cost by a factor.

### `solvency_rules::ltv_borrow_bound_enforced` (line 196)
**Action:** rewrite-as-action with concrete bounds.
**Current scope cost:** very high. The rule calls `process_borrow` (full
multi-asset Map traversal in unsummarised parts of `prepare_borrow_plan` /
`execute_borrow_plan`), then `Controller::total_borrow_in_usd` and
`Controller::ltv_collateral_in_usd`. The latter two are summarised
(`views::total_borrow_in_usd_summary` etc.), but each summary returns a
fresh nondet `>= 0`, so the assert `total_borrow <= ltv_collateral` is
between two independent havocs after a full `process_borrow`.
**Verification value:** ~zero. With both views summarised independently the
post-condition is unrelated to whatever the production code computed during
`process_borrow`. The internal HF / LTV gate is at
`validation::require_healthy_account` which calls the
`calculate_health_factor_summary`, also independent.
**Issue:** independent-havoc summaries on the two view fns; no link between
the post-borrow state and the asserted inequality.
**Proposed change:** rewrite as an action-focused rule that operates on the
*helpers* layer where math is real and the summary boundary doesn't shred
the link:

```rust
#[rule]
fn ltv_collateral_dominates_borrow_after_borrow(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= 1_000_000_000_000); // bounded
    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset, amount);

    // Reach into the math layer that is NOT summarised:
    let mut cache = crate::cache::ControllerCache::new(&e, false);
    let acct = crate::storage::get_account(&e, account_id);
    let ltv = crate::helpers::calculate_ltv_collateral_wad(&e, &mut cache, &acct.supply_positions);
    let (_, total_debt, _) = crate::helpers::calculate_account_totals(&e, &mut cache, &acct.supply_positions, &acct.borrow_positions);
    cvlr_assert!(total_debt.raw() <= ltv.raw());
}
```

Even better: bypass `calculate_account_totals` (summarised) and compute
`total_debt` inline over `acct.borrow_positions` so the assertion sees the
real production math. Note that `calculate_ltv_collateral_wad` at
`controller/src/helpers/mod.rs:29-49` is *not* summarised — it's a real
target.

### `solvency_rules::supply_index_above_floor_after_supply` (line 222)
**Action:** keep.
**Current scope cost:** medium — `supply_summary` returns a nondet
`MarketIndex` already constrained by `supply_index_ray >=
SUPPLY_INDEX_FLOOR_RAW` (see `summaries/pool.rs:44`), so the post-state assert
is satisfied by construction of the summary. PASS is meaningful only for the
controller-side wiring; the pool-internal floor is a pool-layer rule.
**Verification value:** modest — proves the controller wiring respects the
summary contract; does not actually prove the floor in production pool code.
**Issue:** none structurally; the rule is honest about its scope.
**Proposed change:** none, but add a sibling rule run *against the pool
crate* that proves `pool::interest::clamp_supply_index_at_floor` (at
`pool/src/interest.rs:14`) really enforces the floor. Same suggestion in
`audit/certora-review/01-solvency-health-position.md` § 3g.

### `solvency_rules::supply_index_monotonic_across_borrow` (line 254)
**Action:** keep.
**Current scope cost:** medium — same as above; `nondet_market_index` does
not assume monotonicity by itself, so this rule does carry weight: it proves
the controller does not bypass the pool's `global_sync` along the borrow
path. PASS = controller invokes the summary in a way that exposes the
monotone post-condition.
**Verification value:** modest. It's a regression check on the controller's
borrow flow, not a proof of the pool-side accrual.
**Issue:** none.
**Proposed change:** none.

### `solvency_rules::supply_rejects_zero_amount` (line 287)
**Action:** keep.
**Current scope cost:** low — the satisfy-false pattern terminates on the
production panic at `validation::require_amount_positive`, so the prover
short-circuits.
**Verification value:** real — proves the validation layer rejects 0
amount on the public entry. The `validation` module is *not* summarised.
**Issue:** none.
**Proposed change:** none.

### `solvency_rules::borrow_rejects_zero_amount` (line 306)
**Action:** keep. (Same shape as supply_rejects_zero.)

### `solvency_rules::repay_rejects_zero_amount` (line 332)
**Action:** keep. (Same.)

### `solvency_rules::supply_position_limit_enforced` (line 356)
**Action:** tighten (concrete bounds on the list size + `account_id`).
**Current scope cost:** very high. `current_list.len()` is unbounded; the
inner loop `for i in 0..current_list.len()` becomes a symbolic loop with no
unroll bound. `account_id: u64` is symbolic. `process_supply` then traverses
the supply Map again. This is the kind of rule that historically blows past
the TAC budget.
**Verification value:** the property is real — the limit is enforced. PASS
matters.
**Issue:** unbounded symbolic loop over `current_list`; symbolic `account_id`.
**Proposed change:**
1. Pin `account_id = 1`.
2. Replace the dynamic-length-driven precondition with an *explicit* fixed
   list size that equals the limit. E.g.:
   ```rust
   cvlr_assume!(current_list.len() == 10); // production max_supply_positions
   for i in 0..10 { /* unrolled */ }
   ```
   The loop unrolls into 10 concrete steps instead of a symbolic count.
3. Pin `new_asset` to `e.current_contract_address()` so the equality check
   inside the loop bottoms out on a single Address.

This typically cuts the path count by an order of magnitude.

### `solvency_rules::borrow_position_limit_enforced` (line 399)
**Action:** tighten (same as above).
**Current scope cost:** very high (same shape).
**Verification value:** real.
**Proposed change:** same as supply variant — fix list length, fix
`account_id = 1`, pin `new_asset` concrete.

### `solvency_rules::supply_scaled_conservation` (line 438)
**Action:** split + tighten.
**Current scope cost:** very high. Calls `Controller::supply` (full
production traversal: validations, Map iteration, e-mode, isolation, pool
call) plus two `get_scaled_amount` storage reads, plus `pool_client.supplied_amount()`
(unsummarised havoc — used in the second assert).
**Verification value:** the rule asserts only `scaled_delta > 0` and
`supplied_after > 0`. These are weaker than the comment claims (no scale
math is checked); they're individually trivially satisfied by the
`supply_summary`'s `new_scaled >= position.scaled_amount_ray` and by the
unsummarised `supplied_amount()` havoc. PASS tells us essentially nothing.
**Issue:** rule asserts weak post-condition; touches unsummarised pool
view; over-broad scope.
**Proposed change:** split into two rules and tighten each:
1. Replace this with `supply_increases_position` already in `position_rules.rs`
   (delete this one — it's a strict subset of `position_rules::supply_increases_position`).
2. If a "scaled vs amount" relation is wanted, write it as a unit rule
   against `helpers::calculate_scaled_supply` (or whatever the
   amount→scaled converter is) with a concrete `supply_index` and
   concrete `amount`.

### `solvency_rules::borrow_scaled_conservation` (line 489)
**Action:** delete.
**Current scope cost:** very high.
**Verification value:** PASS asserts `scaled_delta > 0` and
`borrowed_after > 0`. The first is a strict subset of
`position_rules::borrow_increases_debt` (line 49). The second is an assert
over an unsummarised pool view (independent havoc).
**Issue:** redundant with `position_rules::borrow_increases_debt`.
**Proposed change:** delete.

### `solvency_rules::repay_scaled_conservation` (line 537)
**Action:** delete.
**Current scope cost:** very high.
**Verification value:** asserts `pos_after < pos_before` (strict subset of
`position_rules::repay_decreases_debt` line 126) and
`borrowed_after < borrowed_before` (independent-havoc — both are unsummarised
pool views).
**Issue:** redundant + havoc-comparison.
**Proposed change:** delete.

### `solvency_rules::borrow_index_gte_supply_index` (line 591)
**Action:** delete.
**Current scope cost:** low (storage reads only, no op).
**Verification value:** zero — `MarketIndex` is read directly from
`storage::market_index::get_market_index`. With no operation under test, the
invariant is asserted over arbitrary values and the prover can refute. **More
importantly**, this invariant is not always true: `seize_position`'s bad-debt
write-down can drop `supply_index` below `borrow_index` (the codebase's own
summary at `summaries/pool.rs:53-56` and `summaries/mod.rs:103-105` explicitly
calls out that `borrow_index_ray >= supply_index_ray` is *not* a global
invariant). The rule asserts a property that production deliberately allows
to be violated.
**Issue:** asserts a false invariant.
**Proposed change:** delete. (Already noted as bogus in
`audit/certora-review/01-solvency-health-position.md` § 13.)

### `solvency_rules::supply_index_grows_slower` (line 609)
**Action:** rewrite-as-action against the math layer.
**Current scope cost:** very high. Triggers a full `Controller::supply`
traversal just to "trigger interest accrual". The post-state index reads go
through `storage::market_index::get_market_index`, which is *not* one of the
pool views — it's the controller's own cache, but the rule reads pre/post
*storage* directly, bypassing the cache. Combined with full `process_supply`
this is the most expensive way to check a math property.
**Verification value:** the property is real (`supply_growth <= borrow_growth`)
but the harness is wrong. The summary on `supply_summary` returns a nondet
`MarketIndex` with no relationship between supply and borrow growth — so PASS
is independent havoc again.
**Issue:** state-invariant style for what is actually a math identity in
`pool::interest::global_sync`.
**Proposed change:** rewrite as a *unit* rule against the pool-layer accrual
function with a small symbolic `(rate, dt)` and bounded indexes:

```rust
#[rule]
fn supply_growth_le_borrow_growth(e: Env, prior: MarketIndex, rate_ray: i128, dt_ms: u64) {
    cvlr_assume!(prior.supply_index_ray >= RAY && prior.borrow_index_ray >= RAY);
    cvlr_assume!(rate_ray >= 0 && rate_ray <= /* per-ms cap */);
    cvlr_assume!(dt_ms <= MILLISECONDS_PER_YEAR);
    let after = pool::interest::accrue(&e, &prior, rate_ray, dt_ms, /* reserve_factor */ );
    cvlr_assert!(after.supply_index_ray - prior.supply_index_ray
                  <= after.borrow_index_ray - prior.borrow_index_ray);
}
```

This belongs in the **pool** spec, not the controller spec.

### `solvency_rules::index_cache_single_snapshot` (line 700)
**Action:** keep.
**Current scope cost:** low — calls `cached_market_index` twice. The first
call goes through `crate::oracle::update_asset_index` (summarised) and stores
the result; the second is a pure Map lookup.
**Verification value:** real — proves the cache memoization is in place.
PASS is meaningful.
**Issue:** none.
**Proposed change:** none.

### `solvency_rules::supply_withdraw_roundtrip_no_profit` (line 738)
**Action:** keep + tighten range.
**Current scope cost:** low — pure math rule on
`fp_core::mul_div_half_up`, no storage, no cross-contract.
**Verification value:** real.
**Issue:** `amount <= RAY * 1000` is `1e30` — that's a wide symbolic range
on i128. Tightening to e.g. `WAD * 1000` (=1e21) covers the realistic input
domain and keeps the SMT simpler.
**Proposed change:** tighten the upper bound on `amount` to `WAD * 1000`.

### `solvency_rules::borrow_repay_roundtrip_no_profit` (line 777)
**Action:** keep + tighten range. (Same as above.)

### `solvency_rules::price_cache_invalidation_after_swap` (line 810)
**Action:** keep.
**Current scope cost:** low — pure cache-mechanic rule, three fast
operations on `ControllerCache`.
**Verification value:** real.
**Issue:** none.
**Proposed change:** none.

### `solvency_rules::mode_transition_blocked_with_positions` (line 853)
**Action:** tighten.
**Current scope cost:** very high — symbolic `account_id`, a
`get_position_list` Map (unbounded), `get_account_attrs`, `get_asset_config`,
*then* a full `Controller::supply` traversal. The satisfy-false pattern
terminates on revert, but the production code's revert path traverses
`prepare_deposit_plan`'s full asset loop + e-mode validations.
**Verification value:** real but expensive.
**Issue:** symbolic account, full production supply path.
**Proposed change:** pin `account_id = 1`, pin `borrow_list` to a
single-asset list of length 1, drop the symbolic check loop. Also pin the
`asset` variable concrete (e.g. `e.current_contract_address()`) so the
e-mode / isolation lookups become concrete.

### `solvency_rules::compound_interest_bounded_output` (line 910)
**Action:** keep.
**Current scope cost:** low — pure math, single function call.
**Verification value:** real (Taylor-overflow defence).
**Issue:** none.
**Proposed change:** none.

### `solvency_rules::compound_interest_no_wrap` (line 942)
**Action:** keep. (Same.)

---

### `health_rules::hf_safe_after_borrow` (line 19)
**Action:** rewrite-as-action against the math layer.
**Current scope cost:** very high. Full `Controller::borrow` (multi-asset
Map traversal in unsummarised parts) + `calculate_health_factor_for` —
which is *summarised* and returns an independent nondet draw.
**Verification value:** zero. The rule asserts `hf >= WAD` on a
post-state nondet that is uncorrelated with whatever happened during
`process_borrow`. See top-level finding in
`audit/certora-review/01-solvency-health-position.md` § "Top-level
structural finding".
**Issue:** the assertion target (`calculate_health_factor_for`) is
summarised away.
**Proposed change:** either
(a) compute HF inline against the real `calculate_account_totals` (also
summarised — needs the same fix), OR
(b) drop the controller-side HF rule and prove HF safety via a unit rule
against `helpers::calculate_health_factor` *unsummarised* (which means
disabling the summary just for this rule's compilation — possible via
a feature gate or running the rule with a different binary).
Until either is done, this rule should be marked vacuous in CI.

### `health_rules::hf_safe_after_withdraw` (line 38)
**Action:** rewrite-as-action — same defect, same fix.

### `health_rules::liquidation_requires_unhealthy_account` (line 60)
**Action:** rewrite-as-action — same defect (HF nondet pre-state, HF nondet
mid-state, HF nondet post-state, all independent).

### `health_rules::supply_cannot_decrease_hf` (line 92)
**Action:** rewrite-as-action — same defect.

For all four, the structural fix is the same: assert a property over the
*real* helpers math, not over the summarised aggregate. Concretely, replace
each rule's HF reads with inline computation of weighted collateral / total
debt over `account.supply_positions` / `account.borrow_positions` directly
(the per-position math at `helpers/mod.rs:17-26` is *not* summarised). With
a single concrete `account_id = 1`, a single supply position, and a single
borrow position, the rule is small *and* meaningful.

---

### `position_rules::supply_increases_position` (line 23)
**Action:** keep.
**Current scope cost:** medium — full `process_supply` plus two
`get_scaled_amount` storage reads. The pool side is summarised
(`supply_summary` guarantees `new_scaled >= old`, with the `>` enforced by
the production scaled-amount math which is in scope for the prover).
**Verification value:** real — proves the controller writes the post-pool
position back to storage and that the scaled balance moves in the right
direction.
**Issue:** none.
**Proposed change:** none, but consider pinning `account_id = 1` to roughly
halve the symbolic state.

### `position_rules::borrow_increases_debt` (line 49)
**Action:** keep. (Same shape.)

### `position_rules::full_repay_clears_debt` (line 77)
**Action:** keep — already tightened (the bound `amount <= WAD` was
explicitly added per the inline comment to avoid the i128::MAX overflow
case-split).
**Current scope cost:** medium.
**Verification value:** real — proves the pool's overpayment-refund branch
zeroes the controller-side scaled debt.
**Issue:** none.
**Proposed change:** none.

### `position_rules::withdraw_decreases_position` (line 100)
**Action:** keep.

### `position_rules::repay_decreases_debt` (line 126)
**Action:** keep.

---

## Cross-cutting recommendations

### 1. Joint pool-views summary (highest-leverage change)

The five unsummarised pool views — `reserves`, `supplied_amount`,
`borrowed_amount`, `protocol_revenue`, `capital_utilisation` — are the
single biggest source of "PASS proves nothing" rules in this domain (Rules
1, 2, 3, 3c, 3e, all three `*_scaled_conservation` rules, plus parts of
`borrow_respects_reserves` and `claim_revenue_bounded_by_reserves`).

Add to `summaries/pool.rs`:

```rust
/// Joint summary: returns a tuple snapshot from instance storage with the
/// production identity wired in.
fn pool_view_snapshot() -> PoolViews {
    let supplied: i128 = nondet();
    let borrowed: i128 = nondet();
    let revenue:  i128 = nondet();
    let reserves: i128 = nondet();
    cvlr_assume!(supplied >= 0);
    cvlr_assume!(borrowed >= 0);
    cvlr_assume!(revenue  >= 0);
    cvlr_assume!(reserves >= 0);
    cvlr_assume!(revenue <= supplied);                 // production identity
    cvlr_assume!(borrowed <= supplied + revenue);      // production identity
    cvlr_assume!(reserves + borrowed >= supplied);     // solvency
    PoolViews { supplied, borrowed, revenue, reserves }
}

pub fn reserves_summary(_env: &Env) -> i128 { /* return the reserves field of a per-tx-cached snapshot */ }
pub fn supplied_amount_summary(_env: &Env) -> i128 { /* same snapshot */ }
// ... etc
```

Implementation note: each summary needs to draw from the *same* snapshot
within a single transaction so the joint identity holds. A static
`thread_local!` cache keyed by current contract address would do it; or
compose them off the existing `get_sync_data_summary` (already returns a
combined snapshot) and have each view summary route through it.

Once this lands, Rules 1 / 2 / 3 / 3c / 3e are *either* trivially true (and
should be deleted) *or* the meaningful work moves into the joint summary
(and the rule becomes a one-line consistency check on the snapshot).

### 2. Concrete `account_id = 1` everywhere

Every rule that does not specifically test the `account_id == 0`
new-account branch should pin `account_id = 1`. Affected rules:
`borrow_respects_reserves`, `ltv_borrow_bound_enforced`,
`supply_index_above_floor_after_supply`,
`supply_index_monotonic_across_borrow`,
`supply_position_limit_enforced`, `borrow_position_limit_enforced`,
`*_scaled_conservation`, `mode_transition_blocked_with_positions`, every
`health_rules` rule, every `position_rules` rule.

Affected rules that *should* keep the symbolic `account_id`: only the
new-account-creation rule (none in this batch — it lives in
`account_rules` if at all).

This is a one-line change per rule (`let account_id: u64 = 1;` in place of
the function parameter). It removes a 64-bit symbolic dimension from each
rule's state space.

### 3. Single-asset focus & concrete `asset`

Every rule that traverses `process_supply` / `process_borrow` /
`process_repay` runs the production iteration over `Vec<Payment>` and the
internal `supply_positions` / `borrow_positions` Maps. Pinning the assets
list to length 1 and the asset address to `e.current_contract_address()`
makes the e-mode / isolation / config lookups concrete and collapses the
inner asset loop to a single iteration.

The summaries already handle the *math*; the symbolic *iteration* is what
the prover spends budget on.

### 4. Tighten the position-limit rules' loops

`supply_position_limit_enforced` and `borrow_position_limit_enforced` both
have an `O(list.len())` symbolic loop with `list.len()` itself a free
variable. Replace with a fixed unrolled `for i in 0..LIMIT` after
`cvlr_assume!(list.len() == LIMIT)`. The production limit is 10 (per
`POSITION_LIMITS` storage; pinned in `controller/src/storage/account.rs`).

### 5. Delete redundant `*_scaled_conservation` rules

`solvency_rules::supply_scaled_conservation`,
`borrow_scaled_conservation`, `repay_scaled_conservation` are weaker
versions of the three `position_rules` rules and additionally reach for
unsummarised pool views. Delete all three; the `position_rules`
counterparts are sufficient.

### 6. HF rules need a non-summarised compilation path

The four `health_rules` rules cannot prove what they claim while
`calculate_health_factor` and `calculate_account_totals` are summarised.
Either build a second binary that disables those summaries for HF rules
only (cfg gate the `summarized!` macros to skip when a `health_unsummarised`
feature is on), or rewrite the rules to assert against
`helpers::calculate_ltv_collateral_wad` + inline borrow-side math (those
helpers are *not* summarised).

---

## Action-focused replacement set (proposed)

The four health rules and `ltv_borrow_bound_enforced` should collapse into
a small, math-anchored set against the *unsummarised* helper layer.

### Proposed: `hf_math_safe_after_borrow`
**Action under test:** `process_borrow` with `account_id = 1`, single asset
(=`e.current_contract_address()`), `amount` bounded `<= WAD * 1000`.
**Preconditions:** none beyond bounds.
**Postcondition:** `weighted_collateral >= total_debt` computed inline
against `account.supply_positions` / `account.borrow_positions` using the
non-summarised `helpers::position_value` / `weighted_collateral`.
**Replaces:** `health_rules::hf_safe_after_borrow`,
`solvency_rules::ltv_borrow_bound_enforced`.

### Proposed: `hf_math_safe_after_withdraw`
Same shape, replaces `health_rules::hf_safe_after_withdraw`.

### Proposed: `hf_math_supply_monotone`
Single-asset, concrete account, asserts pre/post weighted-collateral and
total-debt computed inline. Replaces
`health_rules::supply_cannot_decrease_hf`.

### Proposed: `hf_math_liquidation_only_when_unhealthy`
Single-debt single-collateral, asserts liquidation reverts when
`weighted_collateral >= total_debt` computed inline (do not call the
summarised aggregate). Replaces
`health_rules::liquidation_requires_unhealthy_account`.

### Proposed: `pool_solvency_transition_after_supply`
Captures `(reserves, supplied, borrowed, revenue)` snapshots before and
after one supply (using the joint pool-views summary above), asserts the
solvency identity is preserved across the transition. Replaces all of
`pool_reserves_cover_net_supply`, `revenue_subset_of_supplied`,
`borrowed_lte_supplied`. Add sibling versions for borrow / repay /
withdraw if the joint summary needs to be exercised separately for each
mutating path.

---

## Severity-tagged action items

### Critical — likely to time out the prover (must fix before next run)

1. **Add the joint pool-views summary** described in cross-cutting #1.
   Without it, ~7 rules in this domain are simultaneously havoc-asserting
   and burning budget on cross-contract reads.
2. **Pin `account_id = 1`** across every rule that does not test
   account creation (cross-cutting #2). Single-line change per rule, but
   removes a 64-bit symbolic dimension from each.
3. **Tighten `supply_position_limit_enforced` and
   `borrow_position_limit_enforced`** by fixing the list length and
   unrolling the inner loop (cross-cutting #4). These two rules are the
   most likely individual offenders for hitting the TAC budget.

### Important — large savings, modest effort

4. **Delete the three `*_scaled_conservation` rules** in
   `solvency_rules.rs` (lines 438, 489, 537). They are redundant with
   `position_rules` and additionally trigger unsummarised pool views.
5. **Delete `borrow_index_gte_supply_index`** (line 591) — it asserts a
   property that production deliberately violates after bad-debt
   write-down.
6. **Delete `pool_reserves_cover_net_supply`,
   `revenue_subset_of_supplied`, `borrowed_lte_supplied`** (lines 36, 57,
   79) and replace with one transition rule per mutating op (proposed
   `pool_solvency_transition_after_supply` etc.).
7. **Rewrite `ltv_borrow_bound_enforced`** as the proposed
   `hf_math_safe_after_borrow` against the non-summarised helpers layer.
8. **Rewrite `supply_index_grows_slower`** as a unit rule in the *pool*
   spec, not the controller spec.

### Polish — minor tightening

9. **Tighten `supply_withdraw_roundtrip_no_profit` and
   `borrow_repay_roundtrip_no_profit`** range bounds from `RAY * 1000`
   to `WAD * 1000`.
10. **Pin `mode_transition_blocked_with_positions`** to `account_id = 1`
    and concrete asset; pin `borrow_list` length to 1.
11. **Move `utilization_zero_when_supplied_zero`** to the pool spec; it
    cannot be soundly verified at the controller layer.
12. **Move `supply_index_above_floor_after_supply`** monotonicity proof
    to the pool spec for the actual floor enforcement; keep the
    controller-side version as a wiring regression.

### Already efficient and useful (keep as-is)

`isolation_debt_never_negative_after_repay`,
`index_cache_single_snapshot`, `price_cache_invalidation_after_swap`,
`compound_interest_bounded_output`, `compound_interest_no_wrap`, the three
zero-amount-revert rules, all five `position_rules` rules.
