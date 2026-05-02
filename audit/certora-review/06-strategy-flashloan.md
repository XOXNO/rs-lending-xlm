# Domain 06 — Strategy & Flash Loan (Certora rule review)

**Phase:** Independent formal-verification review of Certora rules for the
controller's strategy and flash-loan domains.

**Files in scope:**
- `controller/certora/spec/strategy_rules.rs:1-701` (701 lines, 19 rules total)
- `controller/certora/spec/flash_loan_rules.rs:1-153` (153 lines, 5 rules total)

**Production code re-read:**
- `controller/src/strategy.rs:1-1058` — `process_multiply`,
  `process_swap_debt`, `process_swap_collateral`,
  `process_repay_debt_with_collateral`, `swap_tokens`,
  `verify_router_input_spend`, `verify_router_output`,
  `call_router_with_reentrancy_guard`, `strategy_finalize`.
- `controller/src/flash_loan.rs:1-77` — `process_flash_loan`.
- `controller/src/positions/borrow.rs:26-75` —
  `handle_create_borrow_strategy` (the pool `create_strategy` call).
- `controller/src/positions/repay.rs:88-140` — `execute_repayment`.
- `controller/src/storage/instance.rs:126-137` — flag accessors.
- `controller/src/validation.rs:30-84` — `require_not_flash_loaning`,
  `require_account_owner_match`, `require_amount_positive`,
  `require_healthy_account`.
- `controller/src/storage/certora.rs:1-179` — Certora storage views.
- `controller/certora/spec/summaries/mod.rs:1-218` — current summary set.
- `controller/certora/spec/compat.rs:32-84` — `multiply` /
  `repay_debt_with_collateral` shims used by the rules.

**Reviewer verdict:** the strategy/flash-loan suite is the **weakest** of the
specs reviewed so far. Most rules are technically sound (no tautologies in
the assumption sense), but a *systemic* soundness gap — un-summarized
cross-contract `LiquidityPoolClient` and `AggregatorClient` calls — leaves
many of the post-conditions vacuously satisfiable and lets several stated
properties pass against a buggy implementation. Coverage of the most
load-bearing strategy invariants (HF gate post-finalize, conservation of
debt/value, allowance-zeroing, idempotency on revert) is absent.

**Tally:** 24 rules reviewed. 4 hard-sound, 6 sound-but-weak, 9
unsound-or-vacuous due to missing summaries, 3 misalignment bugs, 2 missing
companions to existing rules. Eight high-priority gaps documented in §3.

---

## 1. Rule-by-rule findings

For every `#[rule]` we score on the rubric: (1) right invariant, (2) sound
preconditions, (3) sound postconditions, (4) summary use, (5) catches real
bugs, (6) tautology check, (7) coverage. A missing-property failure is
flagged when the rule, as written, would silently green-light a known bug.

### 1.1 `multiply_creates_both_positions` (strategy_rules.rs:31-70)

| Criterion | Verdict |
|---|---|
| Right invariant | Partial — checks "position exists with scaled > 0" but not value alignment with `debt_to_flash_loan` |
| Preconditions | Sound (`debt_to_flash_loan > 0`, `collateral != debt`, `mode ∈ 1..=3`) |
| Postconditions | **Vacuous** — see below |
| Summary use | Production calls `pool_client.create_strategy()` with no summary; `LiquidityPoolClient` calls collapse to havoc |
| Catches real bugs | NO — see "Missing-property" |
| Tautology | None statically; vacuous in practice |
| Coverage | Misses the HF post-condition, the strategy_finalize gate, the `actual_amount > 0` check on the pool result |

**Mechanism of vacuity.** `process_multiply` →
`open_strategy_borrow` → `borrow::handle_create_borrow_strategy`
(`controller/src/positions/borrow.rs:54-74`) calls
`pool_client.create_strategy(...)` and feeds the returned
`result.position` into `record_borrow_update` →
`update::update_or_remove_position` (`controller/src/positions/update.rs:5-21`).
That helper writes
`account.borrow_positions.set(asset, position.clone())` whenever
`scaled_amount_ray != 0`. Because `pool_client.create_strategy` is
**unsummarized**, the prover treats the entire `StrategyResult` as a fresh
nondet havoc — any `scaled_amount_ray > 0` value is a valid model. The
assertion `borrow.scaled_amount_ray > 0` (line 69) is therefore satisfied
trivially regardless of whether the production function would actually
mint a borrow with the correct amount.

**Concrete buggy implementation that still passes.** Suppose
`pool::create_strategy` mistakenly returns
`StrategyResult { actual_amount: 0, amount_received: -1, position: { scaled_amount_ray: 1, ... }, ... }`
— i.e. it inserts a 1-ray "ghost" borrow but never disburses tokens. The
swap then operates on `amount_received = -1` (or 0), but the assertion at
strategy_rules.rs:69 still passes because the inserted position has
scaled > 0. Reality: `borrow.rs:74` returns `result.amount_received` and
the swap math at `strategy.rs:213-227` would silently pass a negative
input to the (havoc) router, which would then satisfy any post-condition
the rule asks for.

The same vacuity affects the deposit assertion at line 62 (the
`pool_client.supply` call inside `supply::process_deposit` is also
unsummarized; see `controller/src/positions/supply.rs:370`).

**Missing-property:** the rule does not assert
- `account.is_isolated == false` for non-isolated collateral (multiply does
  not enforce isolated-asset rules — that's a Phase-2 invariant gap),
- `borrow.scaled_amount_ray * supply_index_ray ≈ debt_to_flash_loan` (value
  conservation),
- HF after `strategy_finalize` ≥ WAD (the actual safety property).

### 1.2 `multiply_rejects_same_tokens` (strategy_rules.rs:79-106)

| Criterion | Verdict |
|---|---|
| Right invariant | Yes |
| Preconditions | Sound |
| Postconditions | Sound — `cvlr_satisfy!(false)` proves unreachability |
| Summary use | OK — guard sits at `strategy.rs:158-160`, before any pool call |
| Catches real bugs | YES — would fail if the equality check were dropped |
| Tautology | None |
| Coverage | OK |

Solid. The guard panics at `strategy.rs:158-160` (`AssetsAreTheSame`)
before any unsummarized cross-contract call, so the prover cannot havoc
its way out.

### 1.3 `multiply_requires_collateralizable` (strategy_rules.rs:115-148)

| Criterion | Verdict |
|---|---|
| Right invariant | Yes |
| Preconditions | **Subtle bug** — the precondition is established via a *fresh* cache, but the rule does not propagate that to the cache used inside `multiply` |
| Postconditions | Sound conceptually |
| Summary use | OK |
| Catches real bugs | Likely yes, but soundness depends on storage determinism |
| Tautology | None |
| Coverage | OK |

**Soundness concern.** Line 131-133:
```
let mut cache = crate::cache::ControllerCache::new(&e, false);
let config = cache.cached_asset_config(&collateral_token);
cvlr_assume!(!config.is_collateralizable);
```
This loads the config via the rule's own cache, asserts it's
non-collateralizable, then drops the cache and lets `multiply` build a new
one at `strategy.rs:186`. `cache.cached_asset_config` reads
`storage::get_market_config(&collateral_token)`; under the Certora prover,
storage reads through the cache should be deterministic across two cache
instances built in the same call, so the `cvlr_assume!` constrains the
underlying storage. **This is sound only if `cached_asset_config` is a
pure projection of storage** — verify by inspecting
`controller/src/cache/mod.rs`. If a future cache version mutates derived
state (e.g. lazily computes a synthetic config), the precondition would no
longer constrain the second cache and the rule would be vacuously
satisfied.

Recommend rewriting via a direct storage-write
(`storage::set_asset_config(...)` style) to avoid the second-cache
question entirely.

### 1.4 `swap_debt_conserves_debt_value` (strategy_rules.rs:157-201)

| Criterion | Verdict |
|---|---|
| Right invariant | **Wrong** — the title says "conserves debt value" but the assertion is "old debt position scaled decreased / new debt position exists with scaled > 0" |
| Preconditions | OK |
| Postconditions | **Trivially satisfiable** — see below |
| Summary use | Pool calls unsummarized (havoc) |
| Catches real bugs | **NO** — does not catch the highest-impact bug class |
| Tautology | None statically |
| Coverage | Critical gap |

**The crucial property.** The actual safety invariant for `swap_debt` is:

> The new debt position's USD value at swap time is **≥** the old debt
> position's USD value (no silent debt reduction). Equivalently, swap_debt
> must not be a stealth repayment.

The rule does not check this. It only asks that:
- the new position exists with `scaled_amount_ray > 0` (line 192),
- the old position decreased or was removed (line 198).

**Buggy implementation that still passes.** A `pool::create_strategy`
that returns `position { scaled_amount_ray: 1 }` (one ray of new debt) and
a `pool::repay` that fully repays the old debt would satisfy both
assertions — the new debt is `1 ray` (scaled > 0) and the old is gone — but
the user just received a free repayment funded by the protocol. The
production code is *correct* on this front (the swap_tokens call moves
the new borrow proceeds through the aggregator and into the repay leg),
but if `repay::execute_repayment` over-credited (e.g. via a
fee-on-transfer regression) the rule would not flag it.

**Recommendation.** Capture USD values pre/post via the summarized
`total_borrow_in_usd` view and assert
`new_debt_usd >= old_debt_usd - slippage_tolerance`. Even with the
nondet summary, a buggy "repay > borrow" path would force the prover to
return values that violate the bound.

### 1.5 `swap_debt_rejects_same_token` (strategy_rules.rs:209-232)

| Criterion | Verdict |
|---|---|
| Right invariant | Yes |
| Preconditions | Sound |
| Postconditions | Sound (`cvlr_satisfy!(false)`) |
| Summary use | Guard at `strategy.rs:264-266` runs before any unsummarized call |
| Catches real bugs | Yes |
| Tautology | None |
| Coverage | OK |

Solid.

### 1.6 `swap_collateral_conserves_collateral` (strategy_rules.rs:240-284)

| Criterion | Verdict |
|---|---|
| Right invariant | **Wrong** — title implies value conservation, body checks structural existence only |
| Preconditions | OK |
| Postconditions | **Vacuous** under havoc |
| Summary use | Aggregator + pool both unsummarized |
| Catches real bugs | NO — see below |
| Tautology | None |
| Coverage | Critical |

Same defect class as Rule 1.4. The rule asserts:
- new collateral position exists with scaled > 0 (line 275),
- old collateral decreased or removed (line 281).

**Buggy implementation that still passes.** A `process_deposit` that
silently inflates the new collateral (e.g. a bug in
`supply::process_deposit` that double-counts the deposit) would satisfy
"scaled > 0" trivially. Worse: a misbehaving aggregator that returns 0
output AND a buggy `verify_router_output` that doesn't enforce
`amount_out_min` would yield `swapped_amount = 0`, the deposit would
write a zero position (which `update_or_remove_position` would actually
*remove*, breaking the assertion — the rule would catch *this* failure
mode), but a positive nondet `swapped_amount` returned from the havoc
swap would re-pass the rule.

**Recommendation.** Bound the new collateral USD value below by the old
collateral USD value scaled by `(1 - max_slippage_bps)`. Use the
`total_collateral_in_usd_summary` (already in
`summaries/mod.rs:195-199`).

### 1.7 `swap_collateral_rejects_same_token` (strategy_rules.rs:291-314)

Sound. Same shape as 1.5.

### 1.8 `swap_collateral_rejects_isolated` (strategy_rules.rs:322-351)

| Criterion | Verdict |
|---|---|
| Right invariant | Yes |
| Preconditions | OK (`attrs.is_isolated == true`) |
| Postconditions | `cvlr_satisfy!(false)` — sound |
| Summary use | Guard sits at `strategy.rs:349-351`, before pool calls |
| Catches real bugs | Yes |
| Tautology | None |
| Coverage | OK |

Solid. Note: the rule reads `is_isolated` via `get_account_attrs`
(`controller/src/storage/certora.rs:31-39`), which falls back to
`is_isolated: false` when meta is absent. If the precondition forces a
state where meta exists, the rule is meaningful; under havoc storage,
the prover can still choose meta-present. OK.

### 1.9 `repay_with_collateral_reduces_both` (strategy_rules.rs:359-411)

| Criterion | Verdict |
|---|---|
| Right invariant | **Underspecified** — checks "decreased" but not "by a sensible amount" |
| Preconditions | OK |
| Postconditions | **Strictly weaker than the production spec** |
| Summary use | Pool/aggregator unsummarized |
| Catches real bugs | Partially |
| Tautology | None |
| Coverage | Misses `close_position=true` semantics entirely (compat helper hard-codes `false`) |

**Compat shim issue.** `compat::repay_debt_with_collateral`
(`compat.rs:65-84`) always passes `close_position: false`. The rule
never exercises the `close_position=true` branch where production
**deletes the account** (`strategy.rs:961-962` via `strategy_finalize`)
and where the `CannotCloseWithRemainingDebt` guard
(`strategy.rs:901-903`) lives. There should be a companion rule with
the close-position branch.

**Strict-decrease assertion is too coarse.** Both assertions
(`scaled_amount_ray < before`) accept a 1-ray decrease as success. A
buggy `seize_position` or `execute_repayment` that decremented by 1 ray
while the swap output went to a third address would pass.

### 1.10 `clean_bad_debt_requires_qualification` (strategy_rules.rs:423-442)

| Criterion | Verdict |
|---|---|
| Right invariant | **Wrong** — uses `calculate_health_factor_for` to model qualification; production uses `calculate_account_totals` |
| Preconditions | **Misaligned** — see below |
| Postconditions | OK (`cvlr_satisfy!(false)`) |
| Summary use | Both helpers summarized as nondet `>= 0` |
| Catches real bugs | NO — see "Misalignment" |
| Tautology | None |
| Coverage | Critical |

**Misalignment bug.** Production
(`liquidation.rs:470-480`) qualifies bad debt via:
```
total_debt_usd > total_collateral_usd && total_collateral_usd <= 5*WAD
```
The rule precondition (line 434-435) instead reads HF and assumes
`hf >= WAD`. These are NOT equivalent:

- HF combines debt against **weighted** (LT-discounted) collateral;
  bad-debt qualification uses raw collateral.
- An account with `hf >= WAD` (healthy by LT) can still satisfy
  `total_debt_usd > total_collateral_usd` if the LT discount is large
  enough — though in practice LT ≤ 100% so this is impossible. The
  reverse, however, is the actual concern: an account with `hf < WAD`
  may still have `total_debt_usd ≤ total_collateral_usd`, which means
  the `cvlr_assume!(hf >= WAD)` does not strictly imply
  "non-qualifying for bad debt". Under the nondet summary, the prover
  picks any `hf` and any totals independently — so the assumption
  constrains nothing about `total_debt_usd vs total_collateral_usd`.

**The rule passes vacuously**: the prover picks `hf = WAD + 1` AND
totals satisfying `debt > coll && coll <= 5 WAD`, then `clean_bad_debt`
**succeeds** in production logic, the cleanup runs all the way through
(the seize calls being havoc, not panicking), and the call returns
*without* hitting `panic_with_error!(CannotCleanBadDebt)`. The rule
expects the call to panic; it doesn't; `cvlr_satisfy!(false)` fails;
**rule fails to verify on the buggy non-bug**. So the rule is over-strict
in the abstract domain but happens to be tautologically *passing* if
the HF and totals summaries are independently nondet — both possible
behaviors satisfy a path the prover can take.

In short: the rule's outcome under the prover is unstable and depends
on exactly which nondet path is chosen. Either way, it does NOT prove
the production property.

**Fix.** Drop the HF assumption. Constrain
`calculate_account_totals` directly:
```
let (coll, _, debt) = helpers::calculate_account_totals(...);
cvlr_assume!(!(debt.raw() > coll.raw() && coll.raw() <= 5 * WAD));
```
This requires summarizing the call and surfacing the totals. Or — better
— write a tiny helper `qualifies_for_bad_debt` and assume its negation.

### 1.11 `clean_bad_debt_zeros_positions` (strategy_rules.rs:451-465)

| Criterion | Verdict |
|---|---|
| Right invariant | Yes (post-state) |
| Preconditions | **Underconstrained** — only assumes borrows non-empty; does not pre-establish qualification |
| Postconditions | OK |
| Summary use | `seize_position` is unsummarized — havoc |
| Catches real bugs | Partial |
| Tautology | None |
| Coverage | OK |

The precondition only requires `!borrow_list.is_empty()` (line 454).
Production additionally requires the bad-debt qualification check at
`liquidation.rs:478` to pass; if it doesn't, the call panics and the
post-condition assertions are unreachable (vacuously true, since we
never get to the `cvlr_assert!`). So this rule essentially proves "**if
qualification holds AND seize succeeds AND remove_account succeeds,
then both maps are empty**". The seize calls are havoc, but
`positions::account::remove_account` (called at line 520 of
liquidation.rs) is in-controller and will deterministically clear the
maps via `storage::remove_account`. So the assertion holds when reached.

The rule does prove a meaningful property — but only conditionally on
reaching the cleanup. Add a companion rule that asserts qualification
implies the cleanup path is reachable (a `cvlr_satisfy!(true)` after
qualifying preconditions), to rule out silent unreachability.

### 1.12 `claim_revenue_transfers_to_accumulator` (strategy_rules.rs:474-484)

| Criterion | Verdict |
|---|---|
| Right invariant | Trivial |
| Preconditions | None |
| Postconditions | `amount >= 0` and `cvlr_satisfy!(amount >= 0)` |
| Summary use | `pool.claim_revenue` unsummarized |
| Catches real bugs | NO |
| Tautology | **YES** — `cvlr_satisfy!(amount >= 0)` after `cvlr_assert!(amount >= 0)` is identical to the assertion |
| Coverage | Misses the actual transfer |

The `cvlr_satisfy!(amount >= 0)` on line 483 is reachable iff the assert
on line 479 passes — so it adds zero information. The whole rule
collapses to "claim_revenue returns a non-negative i128", which is
enforced by the i128 type itself once the havoc returns a non-negative
value (and the prover can trivially pick that). **This rule should be
deleted or rewritten.** The dead `Rule 13` comment block at lines
487-493 already acknowledges that revenue accounting is tested in pool
tests; `Rule 12` should follow it into the trash.

### 1.13 `strategy_blocked_during_flash_loan_*` family (strategy_rules.rs:503-616)

Four rules — multiply, swap_debt, swap_collateral, repay_with_collateral.

| Criterion | Verdict |
|---|---|
| Right invariant | Yes |
| Preconditions | Sound (`set_flash_loan_ongoing(&e, true)` at the top) |
| Postconditions | `cvlr_satisfy!(false)` |
| Summary use | Guard sits BEFORE any unsummarized call (verify per entry point) |
| Catches real bugs | Yes |
| Tautology | None |
| Coverage | OK for the four entry points |

Verified by reading the entry points:
- `process_multiply` → `validation::require_not_flash_loaning` at
  `strategy.rs:156`,
- `process_swap_debt` → `validation::require_not_flash_loaning` at
  `strategy.rs:262`,
- `process_swap_collateral` → `validation::require_not_flash_loaning` at
  `strategy.rs:340`,
- `process_repay_debt_with_collateral` →
  `validation::require_not_flash_loaning` at `strategy.rs:470`.

All four guard *before* `caller.require_auth()` for swap_debt /
swap_collateral / repay (no `require_auth` in those — they rely on
`require_account_owner_match`); `process_multiply` does
`caller.require_auth()` at `:155` *before* the flag check at `:156`.
**Subtle order of operations:** in `process_multiply` the
`require_auth()` runs first, so the four-rule family for `multiply`
will only trip the flag panic if the call has a successful auth. The
test passes a havoc `caller` and a havoc env, so auth is havoc-decided;
the rule may or may not exercise the guard depending on the prover's
auth model. **Worth adding a comment** in the rule clarifying this. If
the prover's `caller.require_auth()` is unsummarized (likely a host
call), it could either panic on its own — making the rule's intended
panic unreachable but `cvlr_satisfy!(false)` still pass — or succeed,
in which case the guard fires. Both behaviors are acceptable for the
rule's stated property (the call ultimately reverts), but the rule does
*not* distinguish "reverted due to auth" from "reverted due to the
flash-loan guard".

**Strengthening:** add an analog rule that asserts the *first*
mutating storage write does not occur. Otherwise a refactor that moves
the guard *after* a mutating write (e.g. an event emission, a meta
TTL bump) would still pass these rules while leaking partial state on a
revert.

### 1.14 Sanity rules (strategy_rules.rs:622-700)

`multiply_sanity`, `swap_debt_sanity`, `swap_collateral_sanity`,
`clean_bad_debt_sanity` — all simple `cvlr_satisfy!(true)` reachability
checks. Fine; they prove the rule harness compiles and the call paths
aren't dead. No safety property.

### 1.15 `flash_loan_fee_collected` (flash_loan_rules.rs:42-64)

| Criterion | Verdict |
|---|---|
| Right invariant | Yes (revenue cannot regress) |
| Preconditions | `amount > 0`; sound |
| Postconditions | `revenue_after >= revenue_before` |
| Summary use | **`pool_client.protocol_revenue()` is unsummarized** |
| Catches real bugs | Likely yes; depends on havoc semantics |
| Tautology | None |
| Coverage | The strict `>` form was relaxed for documented reasons (see line 32-41); reasonable |

**Subtle havoc concern.** `protocol_revenue()` is a cross-contract view.
Without a summary, the prover treats both calls as independent havocs
returning arbitrary i128. The assertion `after >= before` is then
satisfiable only if the prover picks values where after >= before — which
it can (it's free to choose). So the assertion reduces to "there exists
a choice of `protocol_revenue` returns under which the inequality
holds", which is **always true** under havoc.

In other words: **this rule is vacuous.** A buggy `flash_loan_end`
that *decreases* revenue would not be caught; the prover just picks
`revenue_after = revenue_before + 1` (or any larger value) and the rule
passes.

**Required fix.** Summarize `LiquidityPoolClient::protocol_revenue` to
read a deterministic ghost storage variable (e.g. a per-asset cell), and
make `flash_loan_end` (also summarized) write that ghost variable
according to its known post-condition. Without summaries the rule does
not prove anything beyond compile-cleanliness.

### 1.16 `flash_loan_guard_blocks_callers` (flash_loan_rules.rs:80-90)

| Criterion | Verdict |
|---|---|
| Right invariant | Yes (helper panics when flag set) |
| Preconditions | `set_flash_loan_ongoing(&e, true)` |
| Postconditions | `cvlr_satisfy!(false)` |
| Summary use | Helper is in-controller and unsummarized; storage is real |
| Catches real bugs | Yes |
| Tautology | None |
| Coverage | OK; narrow form generalizes — see comment |

Solid. The narrow form (calls just the helper instead of `borrow_single`)
is correctly identified as covering all mutating endpoints transitively.

### 1.17 `flash_loan_guard_allows_when_clear` (flash_loan_rules.rs:96-103)

Sound companion. The `cvlr_satisfy!(true)` after the helper proves
reachability. Catches a regression where the guard panics
unconditionally.

### 1.18 `flash_loan_guard_cleared_after_completion` (flash_loan_rules.rs:117-135)

| Criterion | Verdict |
|---|---|
| Right invariant | Yes |
| Preconditions | Flag clear before; amount > 0 |
| Postconditions | Flag clear after |
| Summary use | `process_flash_loan` calls unsummarized
  `flash_loan_begin/end` and the receiver's `execute_flash_loan` host
  call |
| Catches real bugs | **Partial — see below** |
| Tautology | None |
| Coverage | Critical gap — does not cover the panic path |

**Critical gap.** The rule asserts the flag is clear after a
*successful* completion. But the most dangerous regression class is the
**panic path**: a `flash_loan_end` (or the receiver callback) that
panics after `set_flash_loan_ongoing(env, true)` would normally roll
back atomically (Soroban semantics), so the flag is restored. **But if
the controller is upgraded mid-flight**, or if a future refactor adds
`finish: true` write before a fallible step, the flag could be left
stuck at `true`. This rule cannot detect that because it only inspects
the success path.

**Strengthening — add a panic-path companion.**
```rust
#[rule]
fn flash_loan_guard_cleared_on_revert(e, ...) {
    cvlr_assume!(!is_flash_loan_ongoing(&e));
    // Force a revert: e.g., assume the receiver address is invalid
    // or amount triggers an inner panic.
    let _ = catch_unwind(|| process_flash_loan(...));
    cvlr_assert!(!is_flash_loan_ongoing(&e));
}
```
Soroban's atomicity should make this trivially true, but pinning the
property is the point: a refactor that opens a non-rolling-back code
path (e.g. an explicit `try` wrapper) would break the invariant
silently.

**Vacuity due to unsummarized callback.** Same concern as 1.15: the
`env.invoke_contract::<()>(...)` call to `execute_flash_loan` and the
`pool_client.flash_loan_begin/end` calls are pure havoc. The prover
may exit `process_flash_loan` *before* reaching
`set_flash_loan_ongoing(env, false)` at flash_loan.rs:64 — for
example, if the post-callback `flash_loan_end` is havoc'd to panic. In
that case the rule's call to `process_flash_loan` panics, the
post-assertion is unreachable, and the rule is vacuously satisfied. The
intended invariant — that `set_flash_loan_ongoing(false)` *executes* —
is therefore not actually proved.

**Required fix.** Summarize `flash_loan_begin/end` and the receiver
callback to a no-op that returns `()` without panicking. Then the
post-assertion is reachable on every successful return.

### 1.19 `flash_loan_sanity` (flash_loan_rules.rs:142-152)

Reachability only. Fine.

---

## 2. Cross-cutting issues

### 2.1 Missing `apply_summary!` on cross-contract calls (highest impact)

`controller/certora/HANDOFF.md` line 126 explicitly tracks this:
> Add `apply_summary!` wrappers at pool / oracle / SAC call sites — Pending

**Concrete unsummarized call sites that affect this domain:**

| Call site | File:line | Affects rules |
|---|---|---|
| `pool_client.create_strategy` | `borrow.rs:56-62` | 1.1, 1.4, 1.13 (multiply branch) |
| `pool_client.repay` | `repay.rs:106` | 1.4, 1.9, 1.13 |
| `pool_client.supply` | `supply.rs:370` | 1.1, 1.6 |
| `pool_client.borrow` | `borrow.rs:263` | borrow rules; transitively swap_debt |
| `pool_client.flash_loan_begin/end` | `flash_loan.rs:53,62` | 1.15, 1.18 |
| `pool_client.protocol_revenue` | `flash_loan_rules.rs:57,61` | 1.15 |
| `pool_client.seize_position` | `liquidation.rs:527` | 1.11 |
| `aggregator.swap_exact_tokens_for_tokens` | `strategy.rs:584-595` | 1.1, 1.4, 1.6, 1.9 |
| `token::Client::approve/balance/transfer/transfer_from` | `strategy.rs:417-434` and others | All swap-related rules |

Every cross-contract call without a summary produces a havoc return
value. When a rule's post-condition depends on a value that was
returned (directly or transitively) from such a call, the post-condition
becomes vacuously satisfiable. Out of the 19 strategy rules, **at least
9 — every one that asserts something about post-state position values —
suffers from this.**

**Recommendation.** Author a `summaries/pool.rs` and `summaries/swap.rs`
that capture the production post-conditions:
- `create_strategy` returns `result` with
  `result.position.scaled_amount_ray > 0` and
  `result.amount_received <= amount` (after fee).
- `repay` returns `result` with
  `result.position.scaled_amount_ray <= position.scaled_amount_ray`.
- `flash_loan_end` enforces the repayment delta and returns nothing;
  may panic if under-repaid.
- `swap_exact_tokens_for_tokens` is the *adversarial* router; the
  summary should havoc balances within the bounds the controller's
  `verify_router_input_spend` and `verify_router_output` enforce.
  Specifically: after the call, `token_in.balance_of(controller)` may
  decrease by up to `amount_in` (not more), and
  `token_out.balance_of(controller)` may increase by any non-negative
  amount.

This single piece of work would convert most of the rules in §1 from
"vacuous post-conditions" to "actually proves something".

### 2.2 No conservation rule

**Missing.** None of the rules state the conservation invariant for
strategy operations:
> For each asset, the controller's net change in
> `pool.borrowed[asset]` equals the user's net change in
> `account.borrow_positions[asset]` (after index normalization), plus
> any flash fee.

This is the single most important strategy invariant — a swap_debt that
opens 100 USDC of new debt but only retires 50 USDC of old debt should
either succeed (with the user holding the remaining 50 in their wallet)
or revert. The rules only check existence, never balances.

### 2.3 No "tokens-only-to-controller-or-user" rule

**Missing.** The strategy paths take user tokens (initial payment),
control router input/output, and disburse refunds. There is no rule
asserting:
> After process_multiply / process_swap_debt /
> process_swap_collateral, no tokens are held by the **router** address
> attributable to this controller's account; allowance from controller
> to router is exactly **zero**.

Production zeroes the allowance at `strategy.rs:616` (inside
`settle_router_input`). A regression that skipped that line would not
be caught.

### 2.4 No HF-gate-non-bypassable rule

**Missing.** `strategy_finalize` (`strategy.rs:942-982`) is the only
post-mutation HF gate for multiply / swap_debt / swap_collateral /
repay_with_collateral. The rule set does **not** verify that this
gate runs on every successful exit. A refactor that returned early on
some path (e.g. a "no-op short-circuit" branch inserted into
`process_swap_collateral`) would skip the gate silently.

**Recommendation.** Add a rule that captures the HF before and after,
and asserts: "if the call returned successfully, then HF >= WAD at
exit". Use `calculate_health_factor_for_summary` (already present) plus
explicit pre/post reads.

### 2.5 No allowance-zero rule

**Missing.** `verify_router_input_spend` (`strategy.rs:629-644`) and
the explicit zero-approve at `strategy.rs:616` are critical to prevent
a router from over-pulling. No rule asserts:
> Before swap_tokens: allowance(controller -> router) == 0.
> After swap_tokens: allowance(controller -> router) == 0.

This requires summarizing the SAC (`token::Client`) which is currently
havoc.

### 2.6 No close_position=true rule

**Missing.** The compat shim `repay_debt_with_collateral` always passes
`close_position: false`. The branch where `close_position: true` and
`account.borrow_positions.is_empty()` is **the only path that deletes
the account in the strategy domain** (`strategy.rs:961-962` via
`strategy_finalize`). The deletion has these invariants worth
verifying:
- account is removed iff supply and borrow maps both empty post-call,
- the `CannotCloseWithRemainingDebt` panic fires iff caller asks to
  close while a borrow remains.

### 2.7 Compat shim `multiply` hides edge cases

`compat::multiply` (`compat.rs:32-63`) **always passes**:
- `account_id = 0` (forces new account creation),
- `initial_payment = None`,
- `convert_steps = None`.

This means **none of the strategy rules exercise the
load-existing-account branch nor the initial-payment swap branch**. The
initial-payment branch contains its own `swap_tokens` call (`strategy.rs:695-702`),
which has the same allowance / refund / verification logic and the
same vacuity concerns under havoc summaries. A bug in the
`collect_initial_multiply_payment` swap path would not be caught.

**Recommendation.** Add a second compat shim `multiply_with_initial_payment`
and a corresponding rule that exercises that branch.

### 2.8 No reentrancy-via-aggregator rule

**Missing.** `call_router_with_reentrancy_guard`
(`strategy.rs:570-598`) reuses the flash-loan flag to block aggregator
re-entry. There is **no rule** asserting that during the router call,
the flag is set. The four `strategy_blocked_during_flash_loan_*` rules
test the explicit-set case, not the implicit-set-via-strategy case.

A regression that removed the `set_flash_loan_ongoing(env, true)` at
`strategy.rs:582` would not be caught. The simplest fix is a "ghost
witness" rule:

```rust
#[rule]
fn swap_tokens_sets_guard_during_callback() {
    // Pre: flag false.
    cvlr_assume!(!is_flash_loan_ongoing(&e));
    // Have a summarized aggregator that checks the flag and panics if not set.
    // Run swap_tokens.
    // Post: aggregator_saw_flag_set ghost == true.
}
```
Or, more simply: add `cvlr_assert!(is_flash_loan_ongoing(&e))` inside
the summary for `aggregator.swap_exact_tokens_for_tokens`. Once the
aggregator is summarized.

### 2.9 No siloed-asset rejection rule for swap_debt

**Missing.** `process_swap_debt` (`strategy.rs:281-285`) panics with
`NotBorrowableSiloed` when either side is siloed. No rule covers this
guard.

### 2.10 No isolation-debt-ceiling rule for multiply

**Missing.** `handle_create_borrow_strategy` →
`handle_isolated_debt` enforces the per-asset isolation debt ceiling.
No rule covers it. A regression that skipped the ceiling check would
allow infinite leverage on an isolated asset.

---

## 3. Prioritized recommendations

| # | Priority | Action |
|---|---|---|
| 1 | **P0** | Author summaries for `pool_client.{create_strategy,repay,supply,borrow,flash_loan_begin,flash_loan_end,protocol_revenue,seize_position}` and the SAC `token::Client::{approve,balance,transfer,transfer_from}`. Without these, the strategy/flash domain is effectively unverified. |
| 2 | **P0** | Author the aggregator summary as an *adversarial* model: post-call, `token_in_balance(controller)` may have decreased by ≤ `amount_in` and `token_out_balance(controller)` may have increased by ≥ 0. This is the production model. |
| 3 | **P0** | Add an HF-gate rule: "after every successful strategy call, HF ≥ WAD". |
| 4 | **P0** | Add a conservation rule for swap_debt: "new_debt_usd ≥ old_debt_usd × (1 - tolerance)". |
| 5 | **P1** | Fix `clean_bad_debt_requires_qualification` (rule 1.10) — currently misaligned and vacuous. |
| 6 | **P1** | Replace `claim_revenue_transfers_to_accumulator` (rule 1.12) — currently tautological. |
| 7 | **P1** | Add `flash_loan_guard_cleared_on_revert` companion to rule 1.18. |
| 8 | **P2** | Add allowance-zero pre/post rules around `swap_tokens`. |
| 9 | **P2** | Add the siloed-borrowing and isolation-debt-ceiling rules for the strategy entry points. |
| 10 | **P2** | Add a compat shim for `multiply_with_initial_payment` and a coverage rule. |
| 11 | **P2** | Add a `repay_debt_with_collateral` compat shim with `close_position=true` and a rule covering account deletion + the `CannotCloseWithRemainingDebt` guard. |
| 12 | **P3** | Replace cache-precondition pattern in rule 1.3 with a direct storage write to avoid the cache-determinism question. |

---

## 4. Bugs the current rule set would NOT catch

Concrete buggy regressions that **all 24 reviewed rules pass against**:

1. **`pool::create_strategy` returns a position with the right
   `scaled_amount_ray` but a wrong `actual_amount`** — strategy proceeds
   with a misallocated swap, the user gets less collateral than
   expected, but the position-existence assertion (rule 1.1) is
   satisfied.
2. **`pool::repay` over-credits the repayment** — old debt cleared,
   small new debt opened, user receives a free repayment. Rules 1.4 and
   1.9 pass.
3. **`aggregator` returns less than `amount_out_min` AND
   `verify_router_output` is removed** — strategy continues with the
   shortfall; the deposit / repay leg writes a smaller-than-expected
   position. Rules 1.1, 1.6 still pass because "scaled > 0" is
   trivially true.
4. **`set_flash_loan_ongoing(env, false)` at flash_loan.rs:64 is
   removed** — flag stuck after every successful flash loan. Rule 1.18
   would catch this *if and only if* `flash_loan_end` is summarized as
   non-panicking; with current havoc, the prover can pick a panic path
   and the post-assertion becomes unreachable, hiding the bug.
5. **`approve_router_input` keeps a non-zero allowance after swap** —
   no rule covers this; production guard at strategy.rs:616 could be
   removed silently.
6. **`process_swap_debt` skips
   `validation::require_account_owner_match`** — no rule asserts
   ownership is enforced, so a third party could swap any account's
   debt. (The rule set has a borrow-side ownership rule, but not a
   swap_debt one.)
7. **`strategy_finalize` is moved to *before* the deposit/repay
   leg** — HF check runs on stale state. No HF-gate rule exists.
8. **`process_repay_debt_with_collateral` with `close_position=true`
   permits closure with residual debt** — no rule covers this branch.
9. **`handle_isolated_debt` is bypassed in `handle_create_borrow_strategy`** —
   no rule covers the isolation-debt ceiling on multiply.
10. **A future `PositionMode` variant slips through `process_multiply`'s
    allow-list** — no rule covers the allow-list at strategy.rs:164-169
    (only the `mode ∈ 1..=3` precondition is asserted by the rules,
    which is the inverse of what's needed).

---

## 5. Verdict

**Hard-sound rules:** 1.2 (multiply rejects same tokens), 1.5 (swap_debt
rejects same token), 1.7 (swap_collateral rejects same token), 1.8
(swap_collateral rejects isolated), 1.13 (the four-rule
`strategy_blocked_during_flash_loan_*` family — counted as 4), 1.16
(guard blocks callers), 1.17 (guard allows when clear). Total: **10
sound rules**.

**Sound but weak (correct shape, weak coverage):** 1.3 (relies on
cache determinism), 1.9 (decreases-only check), 1.11 (conditional on
qualification), 1.14 (sanity, 4 rules), 1.19 (flash sanity). Total: **8
weak-but-sound rules**.

**Unsound or vacuous:** 1.1 (multiply creates positions — vacuous on
`scaled > 0` post-state), 1.4 (swap_debt conserves — wrong invariant),
1.6 (swap_collateral conserves — wrong invariant), 1.10 (bad-debt
qualification — misaligned), 1.12 (claim_revenue — tautological), 1.15
(flash_loan_fee_collected — vacuous under havoc), 1.18 (cleared after
completion — vacuous on revert path). Total: **6 problematic rules**.

**Coverage gaps:** 8 missing invariants (§2.2 – §2.10), all
high-impact.

**Bottom line.** The strategy/flash-loan rule set covers the easy
"reject" properties (same-token, same-asset, blocked during flash loan)
well but does not yet prove any of the load-bearing strategy invariants
(value conservation, HF gate non-bypass, allowance hygiene, repayment
sufficiency). The headline fix is summarizing the cross-contract calls;
without that, half the rules pass against arbitrary buggy
implementations of the pool. After summaries land, ~6 new rules would
close the high-priority coverage gaps.

The HANDOFF document already flags the missing summaries as
"post-engagement remediation"; this review concurs and rates that
remediation as **mandatory** (not optional polish) for any meaningful
formal-verification claim on the strategy domain.
