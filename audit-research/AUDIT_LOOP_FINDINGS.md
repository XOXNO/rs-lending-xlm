# Audit Loop — Continuous Findings

Self-paced 5-minute audit loop. Each iteration picks one slice of the
protocol and stress-tests it via reading + thought experiments only. No
edits to production code. New observations are appended; older entries
remain so we can see what we've already considered.

Statuses:
- **Open**: a real concern that warrants follow-up.
- **Confirmed-safe**: looked, considered, no issue found.
- **Discuss**: judgment call worth surfacing; not necessarily a bug.

---

## Iteration 1 — 2026-05-19 15:34

**Focus**: Aftermath of the sanity-bound + zero-supply-withdraw fixes.
Looking for second-order effects.

### O-01 [Confirmed-safe] `MarketOracleConfig::pending_for` still emits (0, 0) sanity bounds

The validator now rejects `(0, 0)` at config time. But
`MarketOracleConfig::pending_for(asset, decimals)` in
`common/src/types.rs:507-508` still seeds zeros — this is the
`PendingOracle` initial state created inside
`router::create_liquidity_pool` before any oracle is configured.

Sequence: create_liquidity_pool → PendingOracle status + sanity bounds
(0, 0) in storage. configure_market_oracle → calls
`validate_sanity_bounds(min, max)` on the input, which now requires
`0 < min < max`. After configure, status flips to `Active`.

Question: could a read happen while status is `PendingOracle` and
the bounds are still (0,0)? `token_price` panics with
`PairNotActive` for `PendingOracle` BEFORE reaching the sanity
check. **Safe.** (Verified `controller/src/oracle/price.rs:30-39`.)

### O-02 [Open] `require_solvent_withdraw_state` is NOT applied to `seize_position` path

`pool/src/lib.rs::seize_position` decrements supplied via the seizure
math but does NOT call `require_solvent_withdraw_state`. The
liquidation path's existing comment says "the protocol must always
be able to liquidate", so blocking seizure on insolvent post-state
would defeat the policy.

But: a donation-backed last-supplier exit COULD happen via
liquidation if the liquidation completely drains supplied while
borrowed remains. Specifically — bad-debt cleanup zeros the supply
index and seizes the supplier's collateral. If the cleanup leaves
supplied=0 and borrowed>0 (residual debt on another asset's
borrower side that wasn't socialized), the same insolvent state
appears.

Actionable check: does `clean_bad_debt` ever leave the pool in
`(supplied=0, borrowed>0)`? Need to read
`controller/src/positions/liquidation.rs::check_bad_debt_after_liquidation`
and `pool::seize_position` carefully on the next iteration.

### O-03 [Discuss] Test fixture sentinel `(1, i128::MAX)` is functionally "disabled"

Replacing `(0, 0)` with `(1, i128::MAX)` in test helpers achieves
the same "any price passes" semantics as the old disabled sentinel.
This is arguably worse than (0,0) because:
- (0,0) was visibly a sentinel.
- (1, i128::MAX) looks like a real config — easy to copy-paste into
  a production deploy by mistake.

Mitigation: the deployment script's preflight requires explicit
`min_sanity_price_wad` and `max_sanity_price_wad` in the JSON. But
nothing prevents someone from copying the test-harness `(1,
i128::MAX)` into the JSON.

Suggestion (for later): add a `validate_realistic_bounds(min, max,
asset_decimals)` that requires `max <= 10^N * WAD` for some sane N
(e.g. N=12 → max $1 trillion per token), and similar lower bound.
Catches fat-finger configurations. Off-chain only is fine; making
this on-chain would over-constrain edge assets.

### O-04 [Open] Cache method `set_isolated_debt` is public

`cache.set_isolated_debt(asset, value)` in `cache/mod.rs:280` is
`pub`. External callers can mutate the cache's isolated-debt
accumulator without going through the usual `get_isolated_debt +
write delta` flow.

Used by `utils.rs::adjust_isolated_debt_usd` to actually apply the
delta. So the API is needed internally. But:
- Crate visibility (`pub(crate)`) would be sufficient.
- A future maintainer could write a path that bypasses
  `flush_isolated_debts` and never persists the delta. Currently
  every caller is correct, but the type system doesn't enforce it.

Defense-in-depth idea: wrap the accumulator in a private type whose
only public methods are `get` and `add_delta`. The flush method
takes ownership.

### O-05 [Confirmed-safe] Repay path on zero-supply: re-checked invariant

After the donation-bypass fix, I worried that `pool::repay` could
hit the new insolvency guard. It doesn't — repay decreases
borrowed, never supplied. Post-state can only move toward
(supplied>0, borrowed=0) or (supplied=0, borrowed=0); never toward
the bad shape.

### Next iteration focus
- Trace `check_bad_debt_after_liquidation` + `pool::seize_position`
  end-to-end. Confirm O-02 isn't an open vector.
- Look at `flash_loan` post-state — does it call any of the new
  guards? It mutates reserves directly, never supplied/borrowed, so
  probably exempt.

---

## Iteration 2 — 2026-05-19 15:39

**Focus**: resolve O-02 (seize_position + bad-debt vs new solvency
guard), then audit `flash_loan` and `create_strategy` for the same
class of bypass.

### O-02 [Confirmed-safe → CLOSED] `seize_position` can't produce `(supplied=0, borrowed>0)`

Traced both branches of `pool::seize_position` (lib.rs:321-350):

- **Borrow branch**: socializes debt via `apply_bad_debt_to_supply_index`
  (interest.rs:82-106), then `cache.borrowed -= position.scaled_amount`.
  The bad-debt write-down lowers the supply *index* (floored at
  `SUPPLY_INDEX_FLOOR_RAW = WAD`) but DOES NOT change `cache.supplied`.
  Borrowed goes down. Result: borrowed shrinks toward zero,
  supplied scaled count stays the same. Cannot reach (0, >0).

- **Deposit branch**: `cache.revenue += position.scaled_amount`,
  `position.scaled_amount = 0`. `cache.supplied` is NOT decremented.
  Result: total claims (supplied + revenue) unchanged; only the
  holder of the claim flips from "user position" to "revenue".

Bad-debt write-down on the supply index can floor at WAD; that means
1 RAY = 10^9 atto-share precision is preserved. No way to make
supplied=0 here.

**Verdict**: seize_position cannot trip the new
`require_solvent_withdraw_state` guard from a clean precondition.
The only path to insolvent post-state is `withdraw`, which is
covered. Item closed.

### O-06 [Discuss] `seize_position(Deposit)` accounting model: user → revenue conversion

Side note from the O-02 dig: when `seize_position(Deposit)` is
called, the user's `scaled_amount_ray` becomes 0 but
`cache.supplied` stays the same — the value is absorbed into
`cache.revenue`. Implicit model: `cache.supplied` is the
*total claim count*, where `revenue` is the protocol's share.

This is consistent with how `claim_revenue` works (it burns from
revenue and decrements `reserves` but doesn't touch `supplied`).
However, this isn't documented. A reader who sees
`seize_position(Deposit)` for the first time may wonder why
`supplied` isn't decremented and assume a bug.

Suggestion (for documentation, not code): one line in
`seize_position` comment clarifying the supplied = user_claims +
revenue_claims invariant. Not a security issue.

### O-07 [Confirmed-safe] `flash_loan` doesn't trip the new guard

`pool::lib.rs::flash_loan(...)` (lines 213-279):
- Decrements then re-increments pool's SAC balance (net = +fee).
- Adds fee to `cache.revenue` via `add_protocol_revenue_ray`.
- **Never touches `cache.supplied` or `cache.borrowed`**.

Flash loan is naturally exempt — it cannot produce the (0, >0)
state.

### O-08 [Discuss → Open] `create_strategy` on a zero-supply pool donates to revenue

`pool::create_strategy` (lib.rs:281-330): increases `cache.borrowed`
without touching `cache.supplied`. Calls
`require_utilization_below_max` which short-circuits on
`supplied == 0`. So this path:

1. Attacker creates pool X with 0 suppliers.
2. Donates tokens to X (via direct SAC mint).
3. Calls `multiply` → controller → `pool_create_strategy_call` →
   `pool::create_strategy`.
4. Post-state X: `supplied=0, borrowed=positive`.

`require_solvent_withdraw_state` is NOT called from
`create_strategy` (only from `withdraw` in my fix).

Is this exploitable?
- Attacker now owes debt to pool X with no suppliers. They must
  eventually repay (or be liquidated). The interest accrued on
  borrowed goes to revenue. No suppliers exist to be diluted.
- Net effect: attacker borrows against donated tokens, pays
  interest back to revenue. Donor lost; protocol gained.
- Not a security risk against existing users, but is a logical
  oddity: a pool can hold debt without any suppliers backing it.

**Stronger argument**: the create_strategy + multiply path doesn't
let the attacker walk away with stolen funds. They borrow → take
tokens out → multiply → end up with collateral on another pool.
But they MUST repay the strategy debt (it's their own position).
If they default, the bad-debt cleanup socialises to revenue
(zero suppliers, so floor at `SUPPLY_INDEX_FLOOR_RAW` triggers).

**Verdict**: not an immediate vulnerability. Worth pinning with a
test that verifies the create_strategy flow on a zero-supply pool
either:
(a) reverts via a different guard, or
(b) succeeds and the post-state is consistent with the donate-to-
   revenue model.

If (a) — good, add the test as documentation.
If (b) — also fine, but document the semantic.

### O-04 [Confirmed-safe → CLOSED] `set_isolated_debt` is `pub` — re-examined

Two callers (`utils.rs:180`, `borrow.rs:374`), both inside
`crate::*`. The `pub` is over-broad but harmless — the cache
module is `pub(crate) mod cache` in lib.rs, so external crates
can't see it anyway. `pub(crate)` would be more explicit but the
effect is identical.

Combined with my earlier audit (visibility tier A in the cache
write-up), this is part of the broader "tighten pub to pub(crate)"
recommendation rather than its own finding. Folding into the
existing recommendation.

### O-09 [Open] `MarketParams::verify_rate_model` doesn't enforce a lower bound on `max_borrow_rate_ray`

Spot-check: looking at `common/src/types.rs::InterestRateModel`
verification. There's a cap of `2 * RAY` (Taylor envelope). But
is there a *lower* bound? A `max_borrow_rate_ray = 0` would mean
zero interest at full utilization, which collapses the IR curve
and disincentivizes lending entirely.

Need to read the verify function on the next iteration to confirm.

### Next iteration focus
- Read `MarketParams::verify_rate_model` to resolve O-09.
- Read `check_bad_debt_after_liquidation` (controller side) end-to-
  end to confirm the bad-debt + dust threshold math doesn't
  underflow on edge cases (huge debt vs tiny collateral).
- Trace `repay_debt_with_collateral` for any cache-bypass left
  from the earlier sweep — I might have missed it.

---

## Iteration 3 — 2026-05-19 15:44

**Focus**: resolve O-09 (rate-model lower bound), audit
`check_bad_debt_after_liquidation`, and sweep `strategy.rs` for
cache bypass.

### O-09 [Confirmed-safe → CLOSED] Rate-model verifier allows `max_borrow_rate_ray` near zero — admin misconfiguration

Read `InterestRateModel::verify` (common/src/types.rs:115-151).
The constraint chain:
- `base >= 0` allows base=0
- `max > base` strict (line 124)
- `max <= MAX_BORROW_RATE_RAY = 2 * RAY` (Taylor envelope)
- Monotone slope chain `base <= s1 <= s2 <= s3 <= max`
- No lower bound on `max_borrow_rate_ray` other than `> base`

So a `max_borrow_rate_ray = 1` (1 atto-ray = nearly zero) IS
admissible if base=0 and slopes are also zero. The result would be
a pool with effectively zero interest at any utilization. No one
would supply (no yield).

**Verdict**: fat-finger admin risk, not a security issue. The
admin can already do worse damage (set bonus to zero, set
liquidation threshold above LTV, etc.). The protocol doesn't try
to second-guess admin config beyond invariant-breaking values.
Closed.

### O-10 [Open] `check_bad_debt_after_liquidation` reads `total_collateral_usd` from `calculate_account_totals` — should the threshold use weighted collateral instead?

`controller/src/positions/liquidation.rs:625-633`:
```rust
let (total_collateral_usd, total_debt_usd, _) = helpers::calculate_account_totals(...);
let bad_debt_threshold = Wad::from_raw(BAD_DEBT_USD_THRESHOLD);
if total_debt_usd > total_collateral_usd && total_collateral_usd <= bad_debt_threshold {
    execute_bad_debt_cleanup(...);
}
```

Two checks:
1. `total_debt_usd > total_collateral_usd` — confirms net-insolvent.
2. `total_collateral_usd <= $5` (BAD_DEBT_USD_THRESHOLD) — confirms
   below the dust threshold for socialization.

Both use **raw collateral USD** (not weighted by liquidation
threshold).

Question: should the bad-debt threshold check use weighted
collateral or raw? Let me think:
- The point of the bad-debt cleanup is "the position is so small
  that future liquidators can't profitably touch it". The
  liquidator's profit comes from the bonus on the seized
  collateral.
- If raw collateral is $4 and weighted is $3.2 (80% LT), the
  liquidator seizes raw collateral (the SAC tokens), and gets a
  bonus on top. Their profit is bounded by the raw collateral
  value, not the weighted.
- So **raw is the correct denominator** for "can this be
  liquidated profitably?".

Actually I think this is correct. Closed — confirmed-safe.

### O-11 [Open] Edge case: `total_collateral_usd == 0` with `total_debt_usd > 0` → bad-debt cleanup fires

If a position has 0 collateral but positive debt (e.g. all
collateral was seized in a previous partial liquidation, but the
oracle price moved to make remaining debt > 0), the check passes:
- `total_debt > 0 > total_collateral = 0` → first condition true.
- `0 <= $5` → second condition true.
- Bad-debt cleanup fires.

`execute_bad_debt_cleanup` then:
- Iterates supply_positions (empty, loop is no-op).
- Iterates borrow_positions, seizes each to pool (which socialises
  debt into the supply index of each pool).

This works. The borrowed debt gets written off across the
supply-side suppliers via the supply-index reduction. Account is
removed.

Edge: what if SOME pools have 0 suppliers? `apply_bad_debt_to_supply_index`
short-circuits when `total_supplied_value == 0` (interest.rs:85-87).
So debt write-off does nothing for that pool. The debt simply
vanishes from the position but doesn't reduce anyone else's
supply value. That's consistent with the "donate to revenue"
pattern.

Actually wait — is there a leak here? If pool X has 0 suppliers
and we apply bad-debt to it, the supply index is unchanged (early
exit). But `cache.borrowed` for pool X still gets decremented in
`seize_position::Borrow`:
```
cache.borrowed.checked_sub_assign(...)
position.scaled_amount = 0;
```
So borrowed counter decrements, but nothing absorbed the loss.
The total claims (supplied + revenue) stayed the same, but
borrowed went down. That means the pool's "ledger" now shows
fewer obligations than before — without anyone receiving the
payoff.

**This is actually clean** from an accounting standpoint: the
pool had a phantom debt (no supplier funded it) and now the
debt is removed. Pool returns to neutral.

Confirmed-safe.

### O-12 [Discuss] `multiply` flow: strategy fee semantics

Skim of `process_multiply` (lines 130-245). The flow:
1. Open strategy borrow on debt_token for `debt_to_flash_loan`.
2. Pool returns `amount_received = amount - fee` (the fee is
   subtracted upfront via `create_strategy`).
3. Swap `amount_received + debt_extra` to collateral via aggregator.
4. Deposit total collateral.

Question: when is the `fee` paid? Looking at `pool::create_strategy`:
- `position.scaled_amount_ray += scaled_debt` (debt counted in
  full amount, not amount-fee).
- `cache.borrowed += scaled_debt`.
- `cache.revenue += fee_ray` (line 309-ish).
- Returns `actual_amount = amount, amount_received = amount - fee`.

So the borrower owes `amount` (including the fee that already went
to revenue). They receive `amount - fee` in tokens. Their position
is `amount` in scaled-borrow Ray.

This is the standard flash-loan-style fee model. Confirmed-safe.

The interesting case: if the fee is large relative to the swap,
the borrower receives few tokens but owes the full amount. If
combined with bad slippage on the swap, the borrower ends up with
collateral worth less than the debt — and the borrow_batch HF
check would catch this.

`strategy_finalize` (line 240) calls `require_within_ltv` +
`require_healthy_account`. If the post-state is underwater, the
whole tx reverts. Good.

### O-13 [Discuss] Read-only views and lazily-created cache

`controller/src/views/mod.rs` creates a fresh
`ControllerCache::new_view(env)` per view function. Each view is a
single tx, so the cache lifetime is bounded.

But: view functions don't share their cache across calls. If an
off-chain caller fires N view queries against the same account in
sequence, that's N caches × M storage reads each.

Mitigation: views are typically called by SDKs that batch RPC
calls. Soroban's `simulate_transaction` is per-call. So caching
across SDK calls is impossible at the contract layer.

Verdict: nothing to fix on-chain. SDKs can cache client-side.

### Next iteration focus
- Audit `pool::repay` for the dust gate. After repay, the
  caller's borrow position can drop to sub-floor; does the dust
  gate apply on the pool side, or only on the controller side?
- Look at the strategy's swap mechanism (aggregator interaction)
  for any auth-chain bypass.
- Check `pool::keepalive` — what does it do, is it safe to call
  from anyone?
- Validate that `OraclePolicy::View` is never used by a
  state-mutating path.

---

## Iteration 4 — 2026-05-19 15:50

**Focus**: dust gate placement (pool vs controller), `pool::keepalive`
access control, `OraclePolicy::View` usage scan, strategy aggregator
auth chain.

### O-14 [Confirmed-safe] Dust gate lives on the controller, NOT in the pool

Every `#[only_owner]` entry in `pool/src/lib.rs` (supply, borrow,
withdraw, repay, create_strategy, seize_position, claim_revenue,
update_params, add_rewards) — none of them call a dust gate. The
pool is decimals-and-scaled-amount aware, but USD valuation lives
on the controller (it has the oracle).

Controller-side dust gate (`positions/dust.rs::require_no_dust_after`)
fires from every state-mutating controller entry:
- supply (process_deposit)
- borrow (process_borrow_plan)
- withdraw
- repay
- liquidation
- strategy_finalize (multiply/swap_*)

Reasoning: the pool can't know USD value (no oracle reference).
The controller is the right layer. Pool guards are scaled-Ray
solvency invariants; controller guards are USD-economics. Layered
correctly.

**One implication worth noting**: pool entrypoints are
`only_owner` (controller is owner), so the only caller is the
controller. Anyone calling pool directly without going through
controller can't bypass the dust gate — the pool would reject
the call at auth. Closed.

### O-15 [Confirmed-safe] `pool::keepalive` is `#[only_owner]` — KEEPER-gated upstream

`pool/src/lib.rs:423-426` — `keepalive` simply calls
`renew_pool_instance` (instance-TTL bump). Gated by `#[only_owner]`,
meaning only the controller can call it. The controller's
`router::keepalive_pools` (line 47) is `#[only_role(caller, "KEEPER")]`.

Two-layer gate: KEEPER role → controller → pool. Safe.

### O-16 [Confirmed-safe] `OraclePolicy::View` only used by `ControllerCache::new_view`

Grep confirms `View` policy is set exclusively at
`cache/mod.rs:48` inside `new_view(env)`. Every state-mutating
path uses one of `RiskIncreasing` / `RiskDecreasing` / `Repay` /
`IsolatedRepay` / `Liquidation`. View policy never leaks into a
mutating path. Closed.

### O-17 [Discuss → Open] Strategy aggregator auth chain — pre-authorize and reentrancy guard

`strategy.rs::swap_tokens` (line 423-onwards):
- `let router_addr = storage::get_aggregator(env);` — admin-set router
  address. Single point of trust.
- `validate_aggregator_swap(...)` — sanity-checks the swap params
  against token_in/token_out/amount_in.
- `pre_authorize_router_pulls(env, &router_addr, &batch)` —
  authorizes the router to pull tokens from the controller.
- `call_router_with_reentrancy_guard(env, &router, &batch)` —
  invokes router with a reentrancy guard.

Two trust assumptions:
1. The router contract behaves as expected (admin-vetted).
2. The token in `validate_aggregator_swap` matches what the router
   will pull. A compromised or buggy router could pull a different
   token via the chained sub-auths.

The pre-authorize is for ONE specific token transfer (the
`amount_in` for the swap's input leg). Other transfers the router
performs internally are NOT authorised by the controller — they
must be backed by the router's own state and the caller's auth.

Risk surface: the router IS the trusted third-party here. If the
admin sets a malicious router address, all strategy operations
become vulnerable.

Mitigations in place:
- Router address change is `#[only_owner]` (admin-gated).
- Slippage floor `swap.total_min_out` is required > 0 by callers.
- Post-swap balance check verifies the controller received what
  the router promised.

**Open question for next iteration**: what does
`validate_aggregator_swap` actually check beyond the
token_in/token_out alignment? Does it bound `amount_in` to a sane
range? Specifically — can the router pull more tokens than the
controller intended? Need to read the validator.

### O-18 [Open] Storage read `storage::get_aggregator(env)` is direct, not cached

`strategy.rs:423` reads the aggregator address from instance
storage per `swap_tokens` call. If `process_multiply` or
`process_swap_debt` triggers a SINGLE swap, that's one read —
acceptable.

But `process_repay_debt_with_collateral` can do TWO swaps (if
the close-position branch needs a follow-up swap). Each reads
storage independently.

Worth caching? The aggregator address is admin-set and rarely
changes. A per-tx cache method on `ControllerCache::cached_aggregator()`
would save 1 storage read per multi-swap path. Marginal but
trivially safe to add.

Marking as Open for future tidy-up.

### O-19 [Confirmed-safe] No-cache `storage::get_account` is correct

Earlier I flagged `storage::get_account` calls scattered across
strategy.rs. Re-examining: account state is per-user and changes
within a tx as the operation mutates it. The cache deliberately
does NOT cache it because the in-memory `Account` value IS the
working copy. After mutations, the controller flushes to storage.
Re-reading from storage during the same tx would clobber the
in-memory mutations.

So `storage::get_account` is the only entry point per tx; the
returned value is mutated in-place. Correct as-is.

### Next iteration focus
- Read `validate_aggregator_swap` to resolve O-17. Specifically:
  what does it enforce about `amount_in`? Can it allow the router
  to pull more than the caller's intent?
- Audit `verify_router_output` — the post-swap balance delta check.
  Confirms the controller actually received the expected output;
  what if the output is the SAME token as the input (e.g. attacker
  router that just returns the input)?
- Spot-check `flash_loan_receiver` — the test/external receiver
  contract. Does it have its own reentrancy guard?
- Consider: what happens if the SAC for an asset is upgraded
  mid-position? Is the asset address used as identity stable?

---

## Iteration 5 — 2026-05-19 16:13

**Focus**: resolve O-17 (aggregator swap trust model), audit
`flash_loan_receiver`, and consider SAC-upgrade resilience.

### O-17 [Confirmed-safe → CLOSED] Aggregator auth chain is well-defended

Read every piece of the swap-trust surface in `strategy.rs`:

`validate_aggregator_swap` (line 482):
- Rejects empty paths, non-positive `amount_in`, non-positive
  `total_min_out`.
- Each path: non-empty hops, non-zero `split_ppm`, first hop's
  `token_in == requested token_in`, last hop's `token_out == requested
  token_out`.
- `sum_ppm == 1_000_000` exactly.
- The router computes per-path amounts; controller's
  `amount_in` is the only spend authorization.

`pre_authorize_router_pulls` (line 674):
- Issues ONE auth entry: `transfer(controller, router, total_in)`
  on `first_hop.token_in`. Single-shot. No sub-invocations.

`verify_router_input_spend` (line 699):
- Allows underspend (leftover stays).
- Rejects overspend.

`verify_router_output` (line 719):
- Receives `>= total_min_out`, else revert.
- Negative delta → revert.

`call_router_with_reentrancy_guard` (line 653):
- Sets `flash_loan_ongoing` flag during router invocation.
- Any reentry into controller hits `require_not_flash_loaning`
  and reverts.

Attack-vector walkthrough:
1. **Router pulls a different token**: auth entry only covers
   `token_in`. Other token transfers need their own auth chain
   that's not provided → fail.
2. **Router calls back into controller**: flash-loan flag set →
   `require_not_flash_loaning` panic.
3. **Router reports success without delivering**: `verify_router_output`
   catches it (received = 0 < total_min_out → revert).
4. **Router delivers wrong token**: balance check is on
   `token_out_client` specifically; if received in different token,
   token_out balance unchanged → revert.
5. **Same-token swap (token_in == token_out)**: caller-side
   checks bypass swap entirely (e.g. `repay_debt_with_collateral`
   line 560). For paths that don't short-circuit, total_min_out
   would still need to be met, which for a self-swap means the
   router would need to refund > input + min_out. Inconsistent;
   reverts naturally.
6. **High slippage**: floor on `total_min_out` is required >0 and
   enforced.

Marked closed.

### O-20 [Confirmed-safe] `flash_loan_receiver` is a test fixture, not production

`flash-loan-receiver/src/lib.rs` is a TEST receiver implementing
every malicious mode (`NoRepay`, `UnderRepay`,
`ReenterPoolFlashLoan`, `Panic`, `ReenterControllerSupply`). The
hardcoded `TESTNET_CONTROLLER` address confirms it's testnet-only.

Production receivers are user-deployed. The protection lives at
the pool layer:
- `pool::flash_loan` requires the receiver to return `amount + fee`
  via the SAC's `transfer_from` flow.
- Pre/post balance assertions catch under-repay.
- `flash_loan_ongoing` flag blocks reentrant controller calls.

The receiver is not part of the protocol's TCB. Closed.

### O-21 [Confirmed-safe] Pool `flash_loan` is `#[only_owner]` — only controller can call

`pool/src/lib.rs:211` — `flash_loan` is `#[only_owner]`. Owner is
the controller. Direct user calls fail at auth.

This means the controller's own flash-loan path
(`flash_loan.rs::flash_loan`) is the single entry. That path
sets the reentrancy flag and validates the receiver before
delegating to the pool.

So the layered defense is: controller → pool. User can't skip
to the pool. Closed.

### O-22 [Discuss] SAC upgrade mid-position — not a realistic vector for host SACs

Host-managed Stellar Asset Contracts (the standard SAC backing
classical XLM-style assets) are wasm-managed by the host. They
aren't upgradeable in the usual sense — the host's wasm is the
implementation, not a user-deployable contract.

For wrapped / custom token contracts allowed via
`approve_token_wasm` (admin-gated): the asset Address is stable.
The wasm hash COULD change if the token contract is upgradeable
(`upgrade` entry on the SAC itself). Admin-vetted assets are the
trust anchor.

**Discussion**: should the controller pin the wasm hash at market
creation and refuse to operate if the SAC's hash has changed? In
Soroban this isn't standard. Most lending protocols rely on
admin allow-listing being a one-time approval. If the listed
token were to upgrade maliciously, admin would need to pause the
market.

Not actionable as an in-code fix — it's a governance question.

### O-23 [Open] `clean_bad_debt_standalone` uses `OraclePolicy::RiskIncreasing`

`liquidation.rs:651` — `clean_bad_debt_standalone` uses
`RiskIncreasing` policy. That policy is strict: rejects stale
source, rejects unsafe deviation, rejects missing TWAP fallback,
rejects disabled markets.

Is that right for bad-debt cleanup? The keeper-callable bad-debt
cleanup operates on under-water positions. If the oracle is
returning stale or deviated prices, blocking cleanup would let
bad debt accumulate.

Compare:
- `OraclePolicy::Liquidation` tolerates anchor deviation, prefers
  aggregator.
- `OraclePolicy::RiskIncreasing` (the current choice) is strict.

The cleanup zeroes the position entirely and socializes the
loss. It's RISK-REDUCING for the protocol. Using a strict policy
here could trap the cleanup behind oracle issues, exactly when
the protocol most needs to socialize.

**Recommendation**: change `clean_bad_debt_standalone` to use
`OraclePolicy::Liquidation` (same policy as the inline post-
liquidation cleanup `check_bad_debt_after_liquidation` reachable
via `process_liquidation`, which uses `Liquidation` policy at the
outer cache).

Wait — let me re-check. `process_liquidation` at line 82 uses
`Liquidation` policy. `check_bad_debt_after_liquidation` is
called from inside `process_liquidation` and inherits its cache,
so the inner check uses Liquidation policy. Good.

But `clean_bad_debt_standalone` is the keeper-direct entrypoint.
It creates its OWN cache with `RiskIncreasing` — inconsistent
with the inline path. This is a real asymmetry.

Marking as Open. Suggested fix: use `Liquidation` policy in the
standalone path to match.

### Next iteration focus
- Confirm O-23: read the full flow of `clean_bad_debt_standalone`
  vs the inline `check_bad_debt_after_liquidation`. Are there any
  semantic differences besides the cache policy?
- Audit `update_account_threshold` (the keeper-callable threshold
  propagation path). Risk: keeper could trigger HF degradation
  if threshold tightening happens on positions already near HF=1.
- Look at e-mode category deprecation: when a category is
  deprecated mid-position, what happens to existing borrowers in
  that category at their next operation?

---

## Iteration 6 — 2026-05-19 16:41

**Focus**: resolve O-23, audit threshold-update keeper path, and
trace e-mode deprecation behavior for existing borrowers.

### O-23 [Open — reaffirmed] `clean_bad_debt_standalone` oracle policy is asymmetric vs inline cleanup

Re-traced both paths:

**Inline (via `process_liquidation`)**:
- Creates cache with `OraclePolicy::Liquidation` at line 82.
- Calls `check_bad_debt_after_liquidation` which calls
  `execute_bad_debt_cleanup`.
- Inherits the outer `Liquidation` policy through the same cache.
- Tolerates anchor deviation, prefers aggregator on deviation.

**Standalone (keeper-direct)**:
- `clean_bad_debt_standalone` creates ITS OWN cache with
  `OraclePolicy::RiskIncreasing` at line 651.
- Strict policy: rejects stale, rejects deviation, rejects
  missing TWAP fallback.

**Operational consequence**: if an oracle is in a brief stale or
deviated state, the inline cleanup (triggered automatically when
a liquidation completes) can proceed. The keeper-direct cleanup
WOULD revert under the same conditions.

Why this matters: the keeper-direct path is the safety net for
positions that never get liquidated (e.g. positions too small to
attract a liquidator). If oracle conditions block both the
liquidator (already covered) AND the keeper (this asymmetry),
bad debt accumulates.

The mitigation: as long as a liquidation EVER fires while the
position is underwater, the inline check sweeps the bad debt.
The keeper-direct call is a backstop for the case where the
position has no liquidator interest at all.

**Recommendation persists**: change line 651 to
`OraclePolicy::Liquidation`. Same reasoning as the inline path —
bad-debt cleanup is risk-reducing; blocking it on oracle deviation
trades a recoverable price-uncertainty for permanent bad debt.

Confirmed Open. Suggested-fix outline:
```rust
let mut cache = ControllerCache::new(env, OraclePolicy::Liquidation);
```

### O-24 [Confirmed-safe] `update_position_threshold` enforces a 5 % HF buffer on risk-tightening updates

`positions/supply.rs:387-477`:
- `has_risks = true` means the keeper is propagating a
  threshold-tightening (lowering liquidation_threshold_bps).
- After the position is updated, line 467-477 runs:
  ```rust
  if has_risks {
      let hf = helpers::calculate_health_factor(...);
      if hf < THRESHOLD_UPDATE_MIN_HF {  // = 1.05 WAD
          panic_with_error!(env, CollateralError::HealthFactorTooLow);
      }
  }
  ```
- `THRESHOLD_UPDATE_MIN_HF = 1.05` WAD (5 % buffer above 1.0).

This means: if a tightening would put HF below 1.05, the WHOLE
update reverts. Keeper can't grief positions by tightening them
into immediate-liquidation territory.

**Subtle question**: what if the position's pre-update HF is
already below 1.05 (e.g. liquidatable but un-touched)? The
tightening reverts even though the position is *already* in
trouble. Result: keeper can't propagate the new params to that
position; the position keeps old (looser) params until liquidated
or repaid.

This is acceptable: the position is already at risk under the
OLD thresholds, so the protocol gains nothing by tightening it
further. The natural liquidation path takes over.

Closed.

### O-25 [Discuss] Threshold-update path uses `OraclePolicy::RiskIncreasing` — appropriate

`update_account_threshold` (line 45) uses `RiskIncreasing` cache.
That's correct: tightening thresholds is risk-increasing for the
existing borrowers (their HF drops). Strict oracle policy is the
right choice — don't tighten using stale or deviated prices.

If the oracle is fresh and price is unchanged, the tightening
applies normally. If oracle is degraded, the update reverts;
keeper retries when oracle recovers.

Closed.

### O-26 [Confirmed-safe] E-mode deprecation: existing borrowers retain old params until their next interaction

E-mode deprecation flow (`config.rs::remove_e_mode_category`):
- Sets `cat.is_deprecated = true` (line 304).
- Comment at line 310 confirms: "is_deprecated stays set so
  effective_asset_config can detect it".

`apply_e_mode_to_asset_config` (emode.rs:20-23):
```rust
if cat.is_deprecated {
    return;  // no override applied
}
```

So a deprecated category's overrides are NOT applied. The asset
config falls back to BASE (non-e-mode) params.

Two operational paths:
1. **Existing borrower opens a new position**: blocked.
   `active_e_mode_category` (line 98) panics with
   `EModeCategoryDeprecated` on supply/borrow/strategy entries.
   The user can't add to their position under the deprecated
   category.
2. **Existing borrower's params on already-held positions**:
   they keep their stored `liquidation_threshold_bps`,
   `loan_to_value_bps` etc. (snapshot at supply/borrow time).
   These are STORED on `AccountPosition`, not recomputed each
   read.

So existing borrowers DON'T auto-flip to base params on
deprecation. The keeper has to run
`update_account_threshold` per asset per account to propagate
the new (base) params. Until that happens, the position keeps
its e-mode-boosted thresholds.

**Question**: is this safe? An e-mode borrower with 95% LT can
keep that LT after deprecation if the keeper hasn't migrated
them. Their HF computation continues using the boosted LT
(stored on AccountPosition.liquidation_threshold_bps).

If admin deprecated the category because of an oracle issue with
one of its assets, allowing the borrower to operate at boosted
LT could be risky. But:
- New borrows in the category are blocked (path 1 above).
- The deprecated state is sticky until keeper migration.
- Liquidations on these positions still use the stored
  (boosted) thresholds, so HF=1 still represents 95% LT, not 80%.

**Risk window**: between deprecation and keeper-migration, the
position is operating under the OLD risk model. Whether that's
appropriate depends on WHY the category was deprecated:
- "We want to phase this out gradually" → fine to keep old
  params, migrate later.
- "This is unsafe immediately" → need rapid migration.

The protocol doesn't distinguish these cases. Admin should run
`update_account_threshold` for all affected accounts immediately
after deprecation if the urgency requires it. Operationally
documented elsewhere? Not visible in the contract.

Marking as Discuss → Open. Recommendation: document the
"deprecation + migration" expected operator runbook in a code
comment near `remove_e_mode_category`.

### O-27 [Open] Threshold-update CAN'T fix already-liquidatable positions

`update_position_threshold` with `has_risks=true` reverts if
post-update HF < 1.05. Practically that means: positions ALREADY
near HF=1.05 can't be migrated to tighter thresholds.

For e-mode deprecation specifically: a borrower at HF=1.10 under
boosted (95 %) LT might be at HF=0.80 under base (80 %) LT.
The keeper's threshold-update would compute HF=0.80, fail the
buffer check, and revert.

Result: the only way to "clear" that position is liquidation. But
liquidation uses the STORED threshold (95 %), so HF stays > 1.0
and no liquidation triggers.

This is a real corner case: deprecated-emode position that's
healthy under old params but unhealthy under new ones, can be
neither migrated nor liquidated until price moves.

Is this exploitable? Not directly — the borrower can't initiate
new borrows in the deprecated category (path 1 above), so the
position is bounded. They can repay or supply more collateral
to migrate, but they have no INCENTIVE to do so (the boosted LT
favors them).

**Suggested mitigation**: when a position is in a deprecated
e-mode category, allow the keeper to migrate even if it puts
HF below the 1.05 buffer (within reason). Or, automatically
migrate on EVERY repay/withdraw/supply operation by the borrower.

The current code does NOT auto-migrate on borrower interaction
— it just blocks NEW borrows. The threshold stays sticky.

Marking as Open. Worth a Codex follow-up.

### Next iteration focus
- Look at `accumulator` storage usage — the controller's revenue
  destination. Is it admin-changeable? What if it's set to a
  malicious address?
- Trace `set_emode_asset` / `edit_asset_in_e_mode_category` —
  can admin manipulate the e-mode asset map to suddenly enable
  a non-collateralizable asset?
- Audit `try_get_account_meta` callers — any path that uses the
  raw `None` return as a "user has no account" signal but
  silently does the wrong thing in production?
- Check whether `update_indexes` (keeper path) is rate-limited.

---

## Iteration 7 — 2026-05-19 17:08

**Focus**: revenue accumulator mutability, e-mode admin
re-config surface, `try_get_account_meta` silent-no-op callers,
`update_indexes` rate-limiting.

### O-28 [Discuss] Accumulator address is mutable by owner — single point of trust

`controller/src/config.rs:32-35` exposes
`set_accumulator(addr)` as `#[only_owner]`. Validates `addr` is a
contract (not an EOA) via `require_contract_address`. No
two-step / time-lock / multisig gate at the contract layer.

If a compromised owner key (or a malicious-but-elected DAO action)
calls `set_accumulator(attacker_contract)`, the next
`claim_revenue(...)` routes all accrued protocol fees to the
attacker. `claim_revenue_for_asset_with_cache`
(router.rs:290-319) reads the accumulator fresh each call —
no snapshot or commit-reveal.

Mitigation: owner is governance. Production deploys must use
multisig and review accumulator changes. The contract pattern is
standard for lending protocols.

Worth pinning: an `OnAccumulatorChanged` event is emitted (let
me check). Actually `set_accumulator` at line 175 doesn't emit
an event — only the storage write. **Improvement opportunity:
emit an `UpdateAccumulator` event so off-chain monitors can
alert on unexpected changes.** This is observability, not
security.

Marking O-28 as Discuss, low priority.

### O-29 [Confirmed-safe] `update_indexes` is keeper-gated and idempotent

Pool `update_indexes` (pool/src/lib.rs:182-190):
- `#[only_owner]` (controller only).
- Loads cache, runs `interest::global_sync`, snapshots, saves.
- Effectively idempotent within a tx; calling it N times yields
  the same state as one call (interest accrual is bounded by
  the time delta, which is the same).

Controller `update_indexes` (router.rs:22-28):
- `#[only_role(caller, "KEEPER")]`.
- Iterates `assets` and updates each.

Rate-limiting: NONE at the contract layer. A keeper could call
`update_indexes` every block. Each call pays the pool's
`global_sync` cost (instructions). At worst this is a gas
griefing of the keeper (paying for their own calls).

Is there any downside to over-frequent calls?
- Each call updates pool storage (TTL bump) — extends pool
  lifetime. Beneficial side effect.
- Re-emits `MarketState` snapshot events — could flood
  monitoring downstream. Filterable client-side.

Not a security concern. Closed.

### O-30 [Confirmed-safe] `try_get_account_meta` callers all handle `None` correctly

Five call sites:
1. `storage/account.rs:115` — wrapped in `get_account_meta`,
   panics with `AccountNotInMarket` on `None`. Internal helper.
2. `storage/account.rs:178` — `try_get_account`, returns
   `Option<Account>`. Internal helper for fallible reads.
3. `supply.rs:398` — `update_position_threshold`: returns silently
   on `None`. Comment: "No-op when the account is gone (bad-
   debt cleanup, full exit)". Correct: the keeper shouldn't
   panic when iterating accounts that were removed by a parallel
   liquidation.
4. `views/aggregates.rs:14, 42` — return 0 USD on `None`.
   Correct for view: missing account = nothing to count.
5. `views/mod.rs:155` — `get_account_positions`: returns empty
   maps on `None`. Correct.

No silent-wrong-behavior paths. Each `None` branch is appropriate
for its context.

Closed.

### O-31 [Open] Admin can flip `is_collateralizable` / `is_borrowable` on existing positions via e-mode edit

`config.rs:389+` `edit_asset_in_e_mode_category` — let me read.
The admin can change `can_collateral` / `can_borrow` for an
asset in an e-mode category at any time. The change is stored
in `EModeAssetConfig`; existing positions store their own
`liquidation_threshold_bps` snapshot in `AccountPosition`, but
the `is_borrowable` / `is_collateralizable` flags are NOT
stored per position — they come from the asset config at read
time.

So:
- Admin flips `is_borrowable=false` for asset X in e-mode 1.
- Existing borrower in e-mode 1 with X debt: at their next
  borrow attempt, `validate_e_mode_asset` panics with
  `AssetNotBorrowable`. They can't add to the position.
- Can they repay? `repay` uses `OraclePolicy::IsolatedRepay` or
  `OraclePolicy::Repay` cache; doesn't call
  `validate_e_mode_asset`. So they CAN repay (good).
- Can they get liquidated? Liquidation uses the stored
  thresholds on the position (snapshotted), not the e-mode
  flags. Liquidation path doesn't call
  `validate_e_mode_asset`. So liquidation works (good).

Conclusion: admin disabling `is_borrowable` for an e-mode asset
blocks NEW positions, allows wind-down. Same model as e-mode
deprecation but per-asset granular. Safe semantically.

**Subtle risk**: an admin could flip `is_collateralizable=false`
on an asset that lots of borrowers depend on as collateral.
Those borrowers can't add more collateral, but their existing
collateral keeps backing their debt — the position is "frozen
in size".

This is admin behavior, not a contract bug. Marking Open as a
governance consideration. Document in operator runbook.

### O-32 [Open] No batching limit on `update_indexes(assets: Vec<Address>)`

`controller/src/router.rs:22`:
```rust
pub fn update_indexes(env: Env, caller: Address, assets: Vec<Address>) {
    ...
    for asset in assets { ... }
}
```

The keeper passes a `Vec<Address>` of any length. Each iteration
hits `pool_update_indexes_call` (cross-contract). Soroban's tx
budget will reject the call if the gas exceeds the cap, so this
isn't a DOS for the contract — but a keeper might pass too many
assets in one tx and waste their gas.

No on-chain size limit. Marking Open as a UX issue. Off-chain
keepers should batch in sane chunks (≤ 10 markets per call).
Adding `validation::validate_position_limits`-style assertion
on the input length would help, but it's keeper-side guardrail
not protocol-side safety.

### Next iteration focus
- Audit `pause` / `unpause` / `when_not_paused`. Does pause
  block liquidations? It SHOULDN'T (liquidations must always
  proceed). Verify.
- Re-read `enforce_supply_cap` / `enforce_borrow_cap` —
  what's the cap-disabled sentinel? Can an attacker set cap=0
  to disable, or is cap=0 distinct from `i128::MAX`?
- Check `aggregate_payments` for `i128::MIN` / overflow
  scenarios — sum of payments with negatives or extreme values.
- Look at strategy auth: when controller authorizes the router
  to pull tokens, the router can use that auth to do more than
  the controller intended IF the router has nested
  sub-invocations. Verify `sub_invocations: Vec::new(env)` is
  empty — that limits the auth to exactly one downstream call.

---

## Iteration 8 — 2026-05-19 17:35

**Focus**: pause coverage on liquidation, cap-disabled sentinel,
payment-aggregation overflow, strategy auth sub-invocations.

### O-33 [Open] `liquidate` is gated by `#[when_not_paused]` — should it be?

`positions/liquidation.rs:22`: `pub fn liquidate(...)` is
annotated `#[when_not_paused]`. So calling `pause()` blocks
liquidations.

`clean_bad_debt` (line 32) is also `#[when_not_paused]`.

**This contradicts security best practice.** During a pause
(typically used during an emergency or oracle outage), the
protocol MUST still be able to liquidate underwater positions —
otherwise bad debt accumulates and a stale-price-induced pause
becomes a debt-amplifying event.

Compare with Aave: their pause is granular per-action. A
"global pause" still allows liquidations; only deposits/borrows/
withdrawals are blocked.

**Severity assessment**:
- If pause is used for routine maintenance (e.g. WASM upgrade
  prep), no liquidations are happening anyway. Fine.
- If pause is used because the oracle is acting up (the
  scenario where you'd most want to pause user-facing actions),
  liquidations also halt — but the protocol's
  `OraclePolicy::Liquidation` is designed exactly to tolerate
  oracle issues. Pausing is a HEAVIER hammer than the policy.
- During a flash crash, pausing user-facing while liquidations
  continue would be ideal. Currently both stop.

**Recommendation**: drop `#[when_not_paused]` from `liquidate`
and `clean_bad_debt`. Liquidations should always proceed.
Keep `pause` for supply/borrow/withdraw/flash_loan/strategy
paths.

Marking O-33 as Open. Real concrete fix candidate.

### O-34 [Confirmed-safe] Supply/Borrow cap sentinels are well-defined

`pool/src/utils.rs:39-41`:
```rust
pub(crate) fn cap_is_enabled(cap: i128) -> bool {
    cap > 0 && cap != i128::MAX
}
```

- `cap <= 0`: disabled (treats 0 and negatives as "no cap").
- `cap == i128::MAX`: also treated as "no cap".
- Any `0 < cap < i128::MAX` is a real cap.

Symmetric for supply and borrow. Negative caps are blocked at
the admin-config layer (controller validates configs before
passing to pool).

**Edge case to verify**: can a borrower set `borrow_cap = -1`
via a malformed call? No — the pool entry takes `borrow_cap: i128`
from `pool_borrow_call`, and the controller fetches it from
storage (where admin validated). User can't pass an arbitrary
cap.

Confirmed-safe.

### O-35 [Confirmed-safe] `aggregate_payment_amount` rejects negative and overflowing inputs

`controller/src/utils.rs:77-95`:
- Rejects `amount < 0` → `AmountMustBePositive`.
- `i128::MIN` would also be `< 0` → rejected.
- `checked_add` on the sum → `MathOverflow` panic on overflow.
- `zero_is_withdraw_all` branch handles full-withdraw sentinel
  correctly: once 0 is seen, the entry stays at 0 (any later
  positive amount preserves the sentinel via `previous == Some(0)`
  check).

Subtle: if a user passes `[(USDC, 0), (USDC, 100)]` and
`zero_is_withdraw_all=true`, the second iteration sees
`previous=Some(0)` and short-circuits to 0. So the user gets
full-withdraw even though they explicitly requested 100 after.
This is a "first-zero wins" semantic, documented behavior.

Edge: `[(USDC, 100), (USDC, 0)]` with `zero_is_withdraw_all=true`:
- First: previous=None, amount=100 → 100.
- Second: previous=Some(100), amount=0. The check at line 87:
  `amount == 0 || previous == Some(0)` → first arm matches, returns 0.

So zero in ANY position wins, regardless of order. Correct.

Confirmed-safe.

### O-36 [Confirmed-safe] Strategy auth `sub_invocations: Vec::new(env)` is verified empty

Two auth-construction sites in strategy.rs:
1. `pre_authorize_router_pulls` (line 674): `sub_invocations:
   Vec::new(env)` — empty.
2. There's a similar pattern in `flash_loan.rs` for the
   receiver's allow.

Empty `sub_invocations` means the granted auth covers EXACTLY
the named function call — no nested sub-calls inherit the
controller's auth. Soroban's auth model is strict on this: if
the router tries to invoke another contract that depends on
the controller's auth, that downstream call fails because no
sub-auth was provided.

This means even a malicious router can ONLY do one specific
SAC transfer with the auth granted. Closed.

### O-37 [Discuss] Pause owner-only — single-key risk

`controller/src/access.rs:101`: `pub fn pause(env: Env)` is
`#[only_owner]`. Same for `unpause` (line 106).

Single-key pause authority. Same trust model as accumulator-
address change (O-28). If the owner key is compromised, the
attacker can:
- Pause the protocol indefinitely (`pause`, never `unpause`).
- This blocks supply, borrow, withdraw, repay, strategy, AND
  liquidation (per O-33). Result: complete protocol freeze.

Combined with O-33, a compromised owner can permanently freeze
positions where they can't repay (because repay is paused) and
can't be liquidated (because liquidation is paused). Borrowers'
positions accrue interest indefinitely while held hostage.

Mitigation: owner is governance. Production deploys use multisig
with pause as a separate role from owner ideally. Currently
both pause and owner are the same address.

**Improvement**: separate `PAUSE_GUARDIAN` role distinct from
`OWNER`. Faster pause (lower-threshold guardian) but full
unpause requires owner. Aave v3 pattern.

Marking O-37 as Discuss. Governance improvement, not a bug.

### Next iteration focus
- Read `apply_liquidation_repayments` and
  `apply_liquidation_seizures` end-to-end. The liquidator
  pays the debt token directly; the controller seizes
  collateral. Are there any reorder vulnerabilities (e.g.
  could a liquidator pay LESS than the amount they're seizing
  due to a timing window)?
- Look at strategy's `swap_or_net_collateral_to_debt` for the
  same-asset short-circuit. Could a malicious aggregator
  return the same token claim 100% net?
- Spot-check storage-key collisions: `ControllerKey` enum
  variants for any potential overlap when keys are encoded.
- Check `EModeAssetConfig` storage: is there a max number of
  e-mode categories per asset (the `e_mode_categories` Vec on
  AssetConfig)?

---

## Iteration 9 — 2026-05-19 18:02

**Focus**: liquidation transfer ordering, same-asset swap
short-circuit, ControllerKey collisions, e-mode categories vec
bounds.

### O-38 [Confirmed-safe] Liquidation transfer ordering — atomic, no reorder window

`apply_liquidation_repayments` (line 164):
```rust
for entry in repaid {
    let pool_addr = cache.cached_pool_address(&entry.asset);
    let token = soroban_sdk::token::Client::new(env, &entry.asset);
    token.transfer(liquidator, &pool_addr, &entry.amount);   // ← real token move
    let position = ...;
    repay::execute_repayment(env, account, ..., &position, ..., entry.amount, cache);
}
```

Sequence per entry:
1. Real SAC transfer from liquidator → pool.
2. `execute_repayment` calls `pool_repay_call` (cross-contract)
   to update pool accounting + position scaled-amount.

If step 2 panics (any reason), Soroban's atomic-tx semantics
revert the step-1 transfer too. No half-state possible.

**Auth chain**: `process_liquidation` line 54 calls
`liquidator.require_auth()`. The outer auth covers the inner
SAC transfer (Soroban authorizes all sub-invocations on behalf
of the authorized caller within the tx).

`apply_liquidation_seizures` is symmetric but operates on the
collateral side: `execute_withdrawal` returns the seized
collateral to the liquidator. No reorder issue.

Confirmed-safe.

### O-39 [Confirmed-safe] Same-asset swap short-circuit bypasses the aggregator entirely

`strategy.rs:898-918`:
```rust
fn swap_or_net_collateral_to_debt(...) {
    if collateral_token == debt_token {
        return collateral_amount;
    }
    swap_tokens(...)
}
```

When collateral and debt are the same token (e.g. USDC/USDC
self-collateralized position), no aggregator call. The
controller transfers collateral straight to debt repayment.

This eliminates the attack surface I considered earlier (malicious
aggregator returning the same token claim) — the aggregator
isn't engaged at all. Closed.

### O-40 [Confirmed-safe] `ControllerKey` enum: no collision risk

`common/src/types.rs:870-897`:
Variants enumerated. Soroban's `#[contracttype]` encoding writes
the variant tag/name + variant data. Two variants with the same
inner type (e.g. `AccountMeta(u64)` vs `SupplyPositions(u64)`)
are differentiated by the tag — never collide.

`Market(Address)` and `IsolatedDebt(Address)` both take Address;
same — differentiated by variant tag.

The comment at line 881-882 notes `FlashLoanOngoing` is in the
enum for stable contracttype encoding but stored in TEMPORARY
storage rather than instance. Temporary and instance storage
are separate namespaces in Soroban — no key collision between
them. Even if both `instance().get(&FlashLoanOngoing)` and
`temporary().get(&FlashLoanOngoing)` were attempted, they'd
return different values (or none).

Closed.

### O-41 [Open] `AssetConfig::e_mode_categories: Vec<u32>` is unbounded

`common/src/types.rs:223`:
```rust
pub e_mode_categories: Vec<u32>,
```

`config.rs:374-376` pushes a category id if not already present:
```rust
if !market.asset_config.e_mode_categories.contains(category_id) {
    market.asset_config.e_mode_categories.push_back(category_id);
}
```

No length cap. An admin could (intentionally or by mistake) add
an asset to thousands of categories. Every read of `MarketConfig`
decodes the full vec → expensive when iterated.

Practical limits:
- Soroban tx budget caps total instruction count.
- `MarketConfig` is read on every supply/borrow/repay/withdraw
  on that asset (via `cache.cached_market_config`).
- Cache amortizes the cost within a tx, but the first read still
  decodes the full vec.

Threat model: only `#[only_owner]` can add categories. So this
is an admin-griefing surface, not user-exploitable.

Mitigation suggestion: bound at e.g. 16 categories per asset,
mirroring `POSITION_LIMIT_MAX`-style discipline. Reject pushes
beyond that.

Marking O-41 as Open. Low priority since admin-gated, but
trivial to fix at config time.

### O-42 [Discuss] Multiple-category memberships per asset: practical meaning?

Following up on O-41 — what's the use case for an asset being in
MULTIPLE e-mode categories? Looking at the code:
- `effective_asset_config` (emode.rs:33-44) takes ONE
  category_id (from `account.e_mode_category_id`) and looks up
  `cached_emode_asset(category_id, asset)`.
- Each account is in zero or one category at a time.
- The `e_mode_categories: Vec<u32>` on the asset is a REVERSE
  INDEX: "which categories is this asset part of?"
- Used by `token_e_mode_config` (line 67) for early rejection.

So multi-category membership is supported: e.g. ETH could be in
both "Stablecoin LSTs" and "Major-cap" categories simultaneously,
serving different account configurations.

Reasonable use case. The bound from O-41 should be high enough
to not unduly constrain protocol evolution (16-32 seems generous).

### Next iteration focus
- Audit `oracle/observation.rs::is_stale` and `check_not_future`.
  Confirm no integer-overflow or sign confusion on extreme
  timestamps near `u64::MAX`.
- Look at `pool::add_rewards` — admin distributes rewards via
  supply-index bump. Is the per-call amount bounded? Is the
  resulting supply-index growth bounded?
- Check `cache.cached_pool_sync_data` — does it correctly
  invalidate on pool state changes within the same tx? If a
  borrow updates the pool's sync data, does a subsequent read
  see the updated state?
- Scan `strategy.rs::validate_swap_new_collateral_preflight`
  for `MixIsolatedCollateral` edge cases.

---

## Iteration 10 — 2026-05-19 18:29

**Focus**: timestamp arithmetic safety, `add_rewards`
unboundedness, pool_sync_data cache freshness, isolated-
collateral preflight.

### O-43 [Confirmed-safe] Timestamp arithmetic is overflow-safe

`observation.rs:59-70`:
```rust
pub(crate) fn is_stale(now_secs: u64, feed_ts: u64, max_stale: u64) -> bool {
    now_secs > feed_ts && (now_secs - feed_ts) > max_stale
}
pub(crate) fn check_not_future_at(env: &Env, now_secs: u64, feed_ts: u64) {
    let max_future_ts = now_secs
        .checked_add(MAX_FUTURE_SKEW_SECONDS)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    ...
}
```

- `is_stale`: subtraction `now_secs - feed_ts` is guarded by
  `now_secs > feed_ts` precondition — no underflow.
- `check_not_future_at`: `checked_add` for the skew bound.
- `MAX_FUTURE_SKEW_SECONDS = 60`: overflow at `now_secs >
  u64::MAX - 60` ≈ year 584 billion. Not reachable.

Closed.

### O-44 [Open — actually defensible] `pool::add_rewards` is unbounded BUT the controller wrapper transfers tokens first

Re-traced the full add_rewards flow:

`router.rs::add_reward`:
1. `transfer_and_measure_received(env, asset, caller, pool_addr, amount, ...)` —
   caller's tokens are pulled into the pool. Returns actual received.
2. `pool_add_rewards_call(env, pool_addr, actual_received)` —
   bumps supply index.

So the supply-index growth IS bounded by what the REVENUE role
can transfer. They can't print money — they must source the
tokens.

Direct calls to `pool::add_rewards` (bypassing controller):
blocked by `#[only_owner]` (only controller can call).

Hypothetical: an `i128::MAX` reward call. The `transfer_and_measure_received`
would fail because no one has i128::MAX of any token (and even
if they did, the SAC would reject overflow in its own ledger).

Supply-index growth over time: each reward adds
`amount_ray / supplied` to supply_index. Over many calls,
supply_index grows. Theoretically could approach i128::MAX over
geological time at high reward rates. Not a practical concern.

**Reclassified**: was Open in the queued plan, but the controller's
token-transfer requirement bounds the practical attack surface.
Confirmed-safe-in-practice. Closed.

### O-45 [Confirmed-safe] `pool_sync_data` cache staleness is benign

Cache layout:
- `pool_sync_data: Map<Address, PoolSyncData>` — populated on
  miss in `cached_pool_sync_data`.
- `market_indexes: Map<Address, MarketIndex>` — populated by
  `cached_market_index` AND `record_market_update`.

`record_market_update` (cache/mod.rs:160) updates `market_indexes`
but NOT `pool_sync_data`. So after a mutating pool call (which
returns a fresh `MarketStateSnapshot`), market_indexes is fresh
but pool_sync_data could be stale.

Is this safe? Tracing callers of `cached_pool_sync_data`:
- ONLY `update_asset_index` in `oracle/price.rs:66`.
- `update_asset_index` is called by `cache.cached_market_index`
  on a market_indexes cache miss.
- After a mutating call, `record_market_update` populates
  market_indexes — so `cached_market_index` HITS the cache on
  subsequent reads and never re-calls `update_asset_index`.

Result: `cached_pool_sync_data` is read at most once per asset
per tx, BEFORE any mutating pool call for that asset (or in the
absence of one). Stale-but-unread is fine.

If the access pattern ever changes (e.g. `update_asset_index`
becomes called from a state-mutating path), this invariant would
need re-checking. Worth a comment noting the invariant.

Confirmed-safe. (Worth a follow-up doc improvement, not a fix.)

### O-46 [Discuss] `validate_swap_new_collateral_preflight` — isolated-collateral exclusion

`strategy.rs:1118` (swap_collateral preflight):
- Loads `config = emode::effective_asset_config(...)`.
- If `config.is_isolated_asset`, panics with
  `MixIsolatedCollateral`.
- Comment: "swap_collateral generally serves non-isolated
  positions only. Isolated accounts use repayDebtWithCollateral
  to deleverage."

So an isolated account CAN'T swap into a new collateral.
That's by design — isolated mode is strict single-asset.

Edge case: what if the NEW collateral is non-isolated but the
ACCOUNT is currently isolated? `account.is_isolated == true`
but `config.is_isolated_asset == false` (new asset). Currently:
- `effective_asset_config` is called with `account.e_mode_category_id`.
- The `is_isolated_asset` check is on the asset config, NOT on
  the account's `is_isolated` flag.
- If new collateral is non-isolated, this check passes. But...

`emode::validate_isolated_collateral` (line 137) handles the
account-side check. Is it called in this path? Let me re-read.

Actually looking at strategy.rs lines 1120-1145, the preflight
calls `ensure_e_mode_compatible_with_asset` (which is the e-mode
gate) and `validate_e_mode_asset`, but does NOT call
`validate_isolated_collateral`.

Result: an isolated account could attempt swap_collateral to a
non-isolated asset. The check at line 1131-1134 only fires if
the NEW asset is isolated (panic), not if the current account
is isolated.

But wait — `swap_collateral` is gated by `#[when_not_paused]`
and processes withdraw + deposit. The deposit-side eventually
hits `validate_isolated_collateral` because supply.rs's
`process_deposit` calls it. So the deposit would fail with
`MixIsolatedCollateral`.

So the preflight isn't comprehensive — but the deposit-time
check catches it. Tx reverts cleanly. Not exploitable.

Optional improvement: add the account-side check at the
preflight to fail faster. Marginal value.

Marking O-46 as Discuss.

### O-47 [Open] Per-position `e_mode_category_id` is mutable via account creation but NOT later

Side question while looking at e-mode: can an account CHANGE
its e-mode category mid-life? Searching the code:
- `create_account_full` accepts `e_mode_category`.
- `process_supply` accepts `e_mode_category` and creates the
  account if `account_id == 0`.
- Once created, `AccountMeta.e_mode_category_id` is set and
  never re-written.

To change category, the user must close the account (full
repay + full withdraw → cleanup_account_if_empty) and create
a new account.

That's restrictive but safe — preventing category switches
mid-position avoids risk-recalculation edge cases. Aave V3
allows mid-position category switches but at the cost of HF
recomputation logic.

This is a UX limitation rather than a bug. Worth documenting in
operator notes.

Marking O-47 as Open (documentation gap, not code fix).

### Next iteration focus
- Look at `process_deposit` and the iteration over `assets` —
  any way to trick it with duplicate assets that bypass the
  e-mode check?
- Audit `cache::record_position_update` for whether the
  recorded event includes the correct price_wad when the asset
  is being added to the cache for the first time during the
  same tx.
- Trace `claim_revenue` flow end-to-end. Does it ever leave
  the pool with `supplied > 0` but `revenue == 0` and
  reserves < supplied? (Insolvency check on revenue claim.)
- Spot-check `keepalive_shared_state` / `keepalive_accounts` /
  `keepalive_pools` — are they coherent with the TTL renewal
  in storage/account.rs?

---

## Iteration 11 — 2026-05-19 18:57

**Focus**: duplicate-asset deposits, claim_revenue insolvency
asymmetry, position-update event price, keepalive coherence.

### O-48 [Confirmed-safe] `process_deposit` collapses duplicate assets via aggregation

`positions/supply.rs:158`: `let deposit_plan = utils::aggregate_positive_payments(env, assets);`

`aggregate_positive_payments` (utils.rs:20) sums duplicate
`(asset, amount)` entries and rejects zero/negative. The
e-mode validation in `prepare_deposit_plan` iterates the
DEDUPED `deposit_plan`, not the raw `assets` input. So a
user passing `[(X, 100), (X, 200)]` ends up with a single
plan entry `(X, 300)` and one round of validation.

No bypass via duplicates. Closed.

### O-49 [Open — theoretical] `pool::claim_revenue` can produce insolvent `(supplied=0, borrowed>0)` post-state

Traced `burn_claimable_revenue` (pool/src/cache.rs:165-181):
```rust
self.revenue.checked_sub_assign(&self.env, scaled_to_burn);
self.supplied.checked_sub_assign(&self.env, scaled_to_burn);
```

claim_revenue decrements BOTH `revenue` AND `supplied`.

Sequence to reach the bad state:
1. Pool has `supplied = user_supplies + revenue_portion`,
   `borrowed > 0`.
2. All users withdraw their supplies (legitimate, per-user
   withdraw never produces supplied=0 with borrowed>0
   because `revenue_portion > 0`).
3. Now `supplied = revenue_portion`, `borrowed > 0`.
4. claim_revenue burns `revenue_portion` → `supplied = 0`,
   `borrowed > 0`. Insolvent state.

`require_solvent_withdraw_state` is NOT called from
claim_revenue. Pool ends with no supplier claims backing the
borrower's debt.

**Practical impact**:
- The borrower's eventual repayment goes into pool reserves
  with no claimant. Tokens are essentially "stuck" until
  future suppliers arrive (or admin manually transfers them).
- Owner-only call (controller is owner). Controller's caller is
  REVENUE role; tokens route to accumulator (admin-set).
- Not exploitable for direct theft, but creates a "stuck
  funds" scenario.

**Recommendation**: add `require_solvent_withdraw_state(env,
&cache);` after `burn_claimable_revenue` in `pool::claim_revenue`
(line 369-ish), same as the withdraw path. The check would
revert claim_revenue when the post-state is (0, >0), forcing
the owner to wait until borrowers exit before claiming the
final revenue tranche.

Alternative: accept the asymmetry and document it. The owner
revenue stream is itself a "supplier" in the accounting
model; claim_revenue is conceptually a withdraw by the
protocol. Treating it uniformly seems cleaner.

Marking O-49 as Open (theoretical, low-severity, low-cost fix).

### O-50 [Confirmed-safe] `cache::record_position_update` price_wad is passed by the caller

`cache/mod.rs:192-212`: `record_position_update` takes
`asset_price_wad: Option<i128>` from the caller. Doesn't
re-fetch or normalize. Just stores in the event.

Callers ALWAYS pass either:
- `Some(feed.price_wad)` from the cache's cached price (e.g.
  liquidation event with the seizure price).
- `None` when price isn't material (e.g. some param-update
  events).
- A computed `price_wad` from the position transfer.

No risk of stale-fetch — the caller controls what's recorded.
Closed.

### O-51 [Confirmed-safe] Keepalive helpers are coherent with TTL renewal

`router.rs::keepalive_shared_state`:
- Calls `storage::renew_controller_instance(env)`,
  `storage::renew_pools_list(env)`, then iterates assets and
  renews e-mode categories.

`keepalive_accounts`: per-account calls
`storage::renew_user_account(env, account_id)` — which renews
meta + both side keys (per the fix in iteration 1 about TTL
counterpart renewal).

`keepalive_pools`: per-asset calls `pool::keepalive` (which
renews the pool's instance storage).

All three are `#[only_role(caller, "KEEPER")]`. The keeper
periodically calls them to keep entries alive past their TTL.

No coherence issues — each helper renews exactly what it
should, the granularity matches the storage layout.

Closed.

### O-52 [Open] No bound on the number of accounts a single user can create

While looking at keepalive: there's no per-user account limit.
A user can create N accounts via repeated `create_account`
calls. Each account costs persistent storage (meta + supply +
borrow keys). The user pays the gas to create, but the
protocol-wide storage footprint grows.

Mitigations:
- Soroban storage has rent-bearing footprint costs paid by the
  account creator. So the user bears the cost economically.
- Accounts with no positions get cleaned up
  (`cleanup_account_if_empty`).
- Empty accounts that never reach `cleanup_account_if_empty`
  still consume storage until TTL expiry.

Not an attack vector against the protocol's operation, but a
DOS-via-storage-spam vector against the indexer (each account
emits events on creation).

Off-chain indexers can filter by activity. Marginal concern.

Marking O-52 as Open (low priority, off-chain monitoring
sufficient).

### Next iteration focus
- Look at the `cleanup_account_if_empty` path and whether the
  meta key is properly removed (vs. just left as an orphan).
- Audit `validate_bulk_isolation` for any edge case where a
  bulk supply could violate the isolation invariant if the
  first asset has different config than later ones.
- Check `OracleObservation::timestamp()` — uses min of
  observed_at and published_at. Is this conservative? Edge:
  what if published_at < observed_at by a lot (e.g.
  published_at = 1 second after epoch)?
- Trace `LiquidationResult.refunds` — when is a refund issued
  and does it correctly route back to the liquidator?

---

## Iteration 12 — 2026-05-19 19:25

**Focus**: cleanup-if-empty meta removal, bulk-isolation
first-asset edge, OracleObservation timestamp min-semantics,
LiquidationResult refunds.

### O-53 [Confirmed-safe] `remove_account_entry` removes all 3 keys; `write_side_map` idempotent on missing

`storage/account.rs::remove_account_entry`:
```rust
persistent.remove(&account_meta_key(account_id));
persistent.remove(&side_key(account_id, POSITION_TYPE_DEPOSIT));
persistent.remove(&side_key(account_id, POSITION_TYPE_BORROW));
```

All three keys deleted. No orphan meta.

Control-flow in `withdraw.rs::process_withdraw`:
```rust
if account.supply_positions.is_empty() && account.borrow_positions.is_empty() {
    utils::remove_account(env, account_id);
} else {
    storage::set_supply_positions(env, account_id, &...);
    ...
}
```

Mutually exclusive — either remove the account or write the
side maps. Never both. No double-mutation.

`write_side_map` defensively checks `persistent.has(&meta_key)`
before renewing — handles the cleanup-then-write path gracefully
(though that path doesn't currently occur).

Closed.

### O-54 [Confirmed-safe] `validate_bulk_isolation` only checks FIRST asset; per-asset loop catches the rest

`validation.rs:155-169`:
```rust
if assets.len() <= 1 { return; }
let (first_asset, _) = ...;
let first_config = cache.cached_asset_config(&first_asset);
if account.is_isolated || first_config.is_isolated_asset {
    panic_with_error!(env, FlashLoanError::BulkSupplyNoIso);
}
```

Only the first asset's `is_isolated_asset` is checked.
**Apparent gap**: what if `[Asset_A (non-isolated), Asset_B
(isolated)]` is supplied to a non-isolated account?

Tracing: this bulk validator passes (first_asset non-isolated +
account non-isolated). The per-asset loop in
`prepare_deposit_plan` then calls
`emode::validate_isolated_collateral(env, account, &asset,
&asset_config)` for EACH asset. Asset_B's iteration would panic
`MixIsolatedCollateral`.

So per-asset validation backstops the bulk check. The bulk
check is just an early-fail optimization for the common case.

Minor improvement: the bulk validator could iterate ALL assets
upfront for symmetry, at the cost of one extra cache lookup
per asset. Marginal.

Closed.

### O-55 [Confirmed-safe] `OracleObservation::timestamp()` returns min — conservatively old

`observation.rs:46-49`:
```rust
pub(crate) fn timestamp(&self) -> u64 {
    self.published_at
        .map_or(self.observed_at, |t| t.min(self.observed_at))
}
```

`published_at` = when oracle generated the price; `observed_at`
= when we read it. min() returns the OLDER timestamp.

Rationale: staleness gating uses this timestamp. Using the
older value makes the protocol more conservative — if the data
was already stale at publication, we shouldn't trust it just
because we read it recently.

Edge: `published_at = 1` (epoch+1s), `observed_at = now`. min =
1. Staleness check sees 1 → far in the past → revert. Correct.

Reverse edge: `published_at > observed_at` (future
publication?). The `check_not_future_at` runs in provider
construction (before `OracleObservation` is built), so future
timestamps are already rejected upstream — they never reach the
`timestamp()` accessor.

Closed.

### O-56 [Confirmed-safe] `LiquidationResult.refunds` is unused at the mutation site by design

`process_liquidation` (liquidation.rs:86): refunds vector is
intentionally discarded — comment: "The pull-model only
transfers the post-cap `amount` from `repaid_tokens` below. The
cap is enforced at the transfer step itself, so over-collection
is impossible."

The liquidator's debt-token transfer is for the EXACT post-cap
amount (computed during `execute_liquidation`). They can't
overpay because the transfer call uses the capped amount, not
the user-supplied amount.

The refunds vector is preserved because the public view
`liquidation_estimations_detailed` returns it as informational
metadata — off-chain simulators use it to anticipate over-
payment scenarios.

Confirmed-safe.

### O-57 [Discuss] Liquidator pays via SAC `transfer` from THEIR own address — auth requirement

`apply_liquidation_repayments` (liquidation.rs:175-177):
```rust
let token = soroban_sdk::token::Client::new(env, &entry.asset);
token.transfer(liquidator, &pool_addr, &entry.amount);
```

The transfer's `from` = liquidator. SAC will require
`liquidator.require_auth()`. The outer `process_liquidation`
calls `liquidator.require_auth()` once at the start; Soroban's
auth propagation covers the inner SAC calls.

But: what if the liquidator's auth scope is more restrictive
than they realize? E.g. they signed a transaction expecting to
pay ONE asset's debt, but the controller's logic decides to
repay across MULTIPLE assets within the same call.

`require_auth()` is unparameterized — it authorizes the entire
call tree, not a specific scope. So the liquidator's auth
covers everything within the tx. That's how Soroban works.

Mitigation: SDK / UI should clearly show the liquidator the
full debt-payment list before they sign. Off-chain
responsibility.

Marking O-57 as Discuss (UX consideration, not a contract
issue).

### Next iteration focus
- Look at `process_excess_payment` (liquidation.rs:298) — the
  refund-routing logic that's used in the view. Does it
  correctly handle multiple debt payments overpaying the same
  asset across different entries?
- Audit how `RepayEntry.feed.price_wad` is captured. If
  liquidation snapshots a price at one point and uses it
  later, is there a window where the price could change?
- Spot-check `OraclePolicy::Liquidation` allowances vs
  `OraclePolicy::Repay` — should they be identical?
- Check `pool::seize_position(side: AccountPositionType::Deposit)`
  vs `AccountPositionType::Borrow` — is the side parameter
  trustable since it comes across the ABI boundary?

---

## Iteration 13 — 2026-05-19 19:57 (also includes status updates)

**Focus**: status updates on prior open items now fixed
(O-23, O-49), plus refund-routing audit, RepayEntry price
snapshot, Liquidation/Repay policy delta, seize_position side
trust.

### O-23 [RESOLVED] `clean_bad_debt_standalone` switched to `OraclePolicy::Liquidation`

Codex's adversarial review independently confirmed this finding.
The user explicitly requested the fix. Applied:

`controller/src/positions/liquidation.rs:651`:
```rust
// before
let mut cache = ControllerCache::new(env, OraclePolicy::RiskIncreasing);
// after
let mut cache = ControllerCache::new(env, OraclePolicy::Liquidation);
```

Regression added:
`verification/test-harness/tests/keeper_tests.rs::test_clean_bad_debt_succeeds_under_oracle_deviation`
— sets up tight tolerance + primary/anchor skew, runs the
standalone cleanup, asserts the position is removed (which
would have reverted under the old `RiskIncreasing` policy).

Both `cargo build -p controller` and the full integration
suite remain green.

### O-49 [RESOLVED] `pool::claim_revenue` now calls `require_solvent_withdraw_state` post-burn

Codex's adversarial review independently confirmed this finding.
Applied:

`pool/src/lib.rs::claim_revenue`:
```rust
let amount_to_transfer = cache.burn_claimable_revenue();
utils::require_solvent_withdraw_state(&env, &cache);  // ← new
let mutation = cache.amount_mutation(amount_to_transfer);
cache.save();
```

The guard fires when `burn_claimable_revenue` would land the
pool at `(supplied = 0, borrowed > 0)`. This blocks the
donation-backed last-supplier-exit + revenue-claim chain:
attacker donates, last user withdraws (leaving supplied =
revenue scaled > 0), then claim_revenue would burn the
remaining supply with borrowed still outstanding.

Regression added:
`verification/test-harness/tests/pool_revenue_edge_tests.rs::test_claim_revenue_blocked_when_post_state_insolvent`
— builds the exact attack post-state, asserts claim_revenue
reverts with `UtilizationAboveMax`.

Side effect: 3 existing `revenue_tests.rs` tests relied on
the initial-liquidity-as-donation pattern (ETH pool with no
user supplier, Alice borrowing against USDC collateral). They
now require an explicit `t.supply(BOB, "ETH", 100.0)` to
seed real ETH supply before Alice's borrow — matching
production where borrows draw from actual supplier liquidity.
Comment in each test explains the prerequisite.

All 7 `revenue_tests` + 3 `pool_revenue_edge_tests` pass.

### O-58 [Confirmed-safe] `process_excess_payment` refund-routing is consistent

`liquidation.rs:563-608`:
- Walks `repaid_tokens` from the END (newest entry first),
  consuming excess against the most recent entry's USD value.
- Partial consumption: computes ratio
  `remaining_excess_usd / entry.usd`, scales the entry's
  `amount` proportionally, recomputes `new_usd` from
  `new_amount * price` to avoid precision drift (rather than
  subtracting `remaining_excess_usd` directly).
- Full consumption: removes the entry entirely.

The comment at line 584-586 explicitly notes the precision-
drift hazard and the chosen mitigation. Vec::remove on the
last index is O(1) and doesn't reorder earlier entries.

Multiple-asset overpayment: each iteration unwinds the most
recent entry; if it doesn't cover the excess, full-consume
and move to the next. Order is deterministic.

Closed.

### O-59 [Confirmed-safe] `RepayEntry.feed` is captured once per asset, never re-read

`calculate_repayment_amounts` (line 328):
- For each unique asset in `merged_payments`, calls
  `cache.cached_price(&asset)` ONCE.
- Stores the resulting `feed: PriceFeed` in the `RepayEntry`.
- Downstream consumers (apply_liquidation_repayments,
  process_excess_payment) read `entry.feed` — never re-fetch.

The cache layer guarantees consistency: a single tx's
`cached_price` calls for the same asset return the same value
(populated on first miss, served from cache afterward).

There's no window where the price changes mid-liquidation —
the snapshot is locked at the start of `execute_liquidation`.

Closed.

### O-60 [Discuss] `OraclePolicy::Liquidation` vs `OraclePolicy::Repay`: intentional asymmetry

| Field | Liquidation | Repay |
|---|---|---|
| disabled_market | false | true |
| stale_source | false | true |
| unsafe_deviation | true | true |
| missing_twap_fallback | false | true |
| prefer_aggregator_on_deviation | true | false |

Differences:
- Liquidation REJECTS disabled markets; Repay ALLOWS (so a
  borrower can always close debt against a deprecated reserve).
- Liquidation REJECTS stale source / missing TWAP fallback;
  Repay ALLOWS (repay is unconditionally beneficial).
- Both ALLOW unsafe deviation, but Liquidation prefers the
  aggregator (live market) while Repay prefers the safe source
  (slow TWAP).

The asymmetry is by design and well-reasoned:
- Liquidation MUST follow live market price to seize
  collateral at current value (slow TWAP would let underwater
  positions evade liquidation during a crash).
- Repay is benign for the protocol — anyone closing debt is
  reducing risk, so tolerating oracle issues is safe.

Closed (not a code change candidate; semantic intentional).

### O-61 [Confirmed-safe] `pool::seize_position(side)` — controller is sole caller, side parameter trusted

`pool::seize_position` is `#[only_owner]`. Only the controller
can call it. Controller calls it from
`execute_bad_debt_cleanup` via `seize_pool_position` (line
711-722):
```rust
seize_pool_position(env, cache, AccountPositionType::Deposit, &asset, &position);
seize_pool_position(env, cache, AccountPositionType::Borrow, &asset, &position);
```

The side is hardcoded per loop (collateral iter → Deposit,
debt iter → Borrow). No user-controllable input determines
the side.

User-direct calls to pool::seize_position would fail at the
auth gate. Safe.

### Next iteration focus
- Look at `validate_aggregator_swap.sum_ppm == 1_000_000`
  edge: what if a path has `split_ppm = 1_000_001` (overflow)?
  Does `checked_add` catch it before the equality check?
- Audit `add_e_mode_category` for any way to inject a
  category with `is_deprecated = true` from the start (which
  would block immediate use).
- Spot-check the `OraclePolicy::View` cache for whether it
  could ever bleed into a state-mutating path via shared
  helpers.
- Trace `aggregate_withdrawal_payments` for whether the
  zero-sentinel preservation has any edge case where a
  duplicate `(asset, 0)` + `(asset, 100)` could be ambiguous.

---

## Iteration 14 — 2026-05-19 21:00

**Focus**: PPM overflow, e-mode deprecated-from-start,
View policy bleed (re-check), withdraw zero-sentinel.

Light iteration — queue items all close as confirmed-safe.

### O-62 [Confirmed-safe] `validate_aggregator_swap.sum_ppm` overflow guarded by `checked_add`

`strategy.rs:516-518`:
```rust
sum_ppm = sum_ppm
    .checked_add(path.split_ppm)
    .unwrap_or_else(|| panic_with_error!(env, GenericError::InvalidPayments));
```

`sum_ppm` is `u32`. Each path's `split_ppm` is also `u32`. Even
if a single path's `split_ppm` is `u32::MAX`, `checked_add`
either succeeds (and the later `!= 1_000_000` check catches it)
or panics with `InvalidPayments`. No overflow window.

Closed.

### O-63 [Confirmed-safe] `add_e_mode_category` hardcodes `is_deprecated: false`

`config.rs:269` literal `is_deprecated: false`. Admin can't
create a pre-deprecated category. The only way to set
`is_deprecated = true` is the dedicated
`remove_e_mode_category` path (line 304) which is
`#[only_owner]`.

Closed.

### O-64 [Confirmed-safe — duplicate of O-16] `OraclePolicy::View` bleed paths

Already audited in iteration 7 (O-16). View policy is set only
in `ControllerCache::new_view`. Mutating paths use
`RiskIncreasing` / `RiskDecreasing` / `Repay` / `IsolatedRepay`
/ `Liquidation`. No bleed surface.

### O-65 [Confirmed-safe — duplicate of O-35] withdraw zero-sentinel handling

Already audited in iteration 8 (O-35). Once `(asset, 0)` is
seen with `zero_is_withdraw_all: true`, the sentinel persists
regardless of subsequent positive amounts in the same batch.
Order-independent: zero in any position wins.

### Concurrent Codex simplification pass running

User pointed out the `_cached` suffix bloat earlier this turn —
those have been folded back into the original
`require_market_active(env, cache, asset)` and
`require_asset_supported(env, cache, asset)`. Codex is running
an adversarial review looking for further over-engineering vs
the reference patterns in `/Users/mihaieremia/GitHub/rs-lending/controller/`.
Results will land in a separate message.

### Next iteration focus
- Audit `helpers::estimate_liquidation_amount` for the
  twin-target (1.02 → 1.01 → base-bonus) fallback chain.
  Are there inputs where the fallback never converges?
- Re-examine `validation.rs` for any other validator that
  duplicates a cache fetch unnecessarily (post the
  `require_market_active` cleanup).
- Spot-check `oracle/compose.rs::resolve_components` for any
  silent fall-through that returns a malformed value rather
  than panicking.

---

## Iteration 15 — 2026-05-19 22:30

### Theme: duplicate storage-key reads across one entry-point call

Manual trace of every controller flow against `storage::*`
plus hidden reads inside `storage` helpers
(`write_side_map`, `renew_user_account`, `set_account_meta`).
Question: where do we read the same persistent key more than
once in a single transaction?

### O-66 [Confirmed-low] `liquidation.rs:62` then `:83` duplicate `AccountMeta` read

In `process_liquidation`:
- `:62` reads `get_account_meta(account_id)` for the
  self-liquidation guard (`account_meta.owner == liquidator`).
- `:83` reads the full `get_account(account_id)` which
  internally calls `try_get_account_meta(account_id)`.
- Two side writes at `:142` and `:143` each issue
  `has(meta_key)` inside `write_side_map`.

**Net: `AccountMeta(account_id)` accessed 4× in one call.**

Fix: drop `:62`, move the self-liquidation guard after
`:83` using `account.owner == *liquidator`. Saves 1 explicit
read. The 2 internal `has` calls inside the paired side
writes are a separate concern (see O-72).

### O-67 [Confirmed-low] `keepalive_pools` reads `Market(asset)` twice per asset

`router.rs:424-434` iterates the assets vec and per asset:
- `:427` `has_market_config(asset)` (`persistent.has`)
- `:430` `get_market_config(asset)` (`persistent.get`)

The `has` exists to skip non-existent markets without
panicking. Equivalent with one read:
```rust
let Some(market) = storage::try_get_market_config(env, &asset)
    else { continue; };
```

**Net: `Market(asset)` accessed 2× per asset in a keeper
loop that can be passed many assets.** O(n) waste.

### O-68 [Confirmed-low] First-asset `Market` read twice in supply

`process_supply` with `account_id == 0`:
1. `utils.rs:111` natively reads
   `storage::get_market_config(first_asset).asset_config` to
   determine `is_isolated_asset` for `create_account`.
2. Cache is constructed.
3. `prepare_deposit_plan` (`supply.rs:187`) calls
   `require_market_active(first_asset)` →
   `cache.cached_market_config(first_asset)` → cache miss →
   re-reads `Market(first_asset)`.

**Net: `Market(first_asset)` accessed 2× when the supply
flow is opening a new account.**

Fix options:
- (a) Defer `is_isolated` resolution: build the cache first,
  then `cache.cached_market_config(first_asset).asset_config
  .is_isolated_asset`. Requires returning the resolved
  market to `create_account`.
- (b) Cheaper: have `create_account_for_first_asset` return
  the read `MarketConfig` alongside the new `(id, account)`
  pair, and the caller seeds `cache.market_configs` with it
  before `process_deposit`.

### O-69 [Confirmed-low] `apply_threshold_update_to_position` double TTL bump

`supply.rs:397-461` (the keeper threshold update path):
- `:397` `try_get_account_meta` — 1 explicit read of meta
- `:401` `get_supply_positions` — 1 read of supply
- `:410` `get_borrow_positions` (conditional) — 0/1 read
- `:415` `storage::renew_user_account(account_id)` — issues
  `has(meta) + has(supply) + has(borrow)` and renews all
  three TTLs
- `:461` `storage::set_supply_positions(account_id, ...)` →
  `write_side_map` issues `has(meta) + has(borrow)` and
  renews them BOTH AGAIN

**Net: meta + borrow each get renewed twice in one call.**

Fix: drop the `:415 renew_user_account` — the eventual
`set_supply_positions` already TTL-bumps meta and the
counterpart borrow side via `write_side_map`. The only key
that is NOT renewed by `set_supply_positions` in this case
is the supply key itself when the post-mutation map is
empty (the `remove` branch); the keeper path always writes
a non-empty map, so this gap doesn't apply.

### O-70 [Confirmed-low] `router::renew_account` double meta touch

`router.rs:414-422`:
- `:416` `get_account_meta(account_id)` — explicit
- `:421` `renew_user_account(account_id)` — `has(meta_key)`
  again then renew

**Net: `AccountMeta(account_id)` accessed 2× in one call.**

Could rewrite as:
```rust
let meta = storage::get_account_meta(env, account_id);
storage::renew_user_key(env, &ControllerKey::AccountMeta(...));
// ... + side renewals without the meta has() probe
```
But this requires either exposing a no-probe renew helper
or accepting that the second has() is cheap. Defensible
either way; flagging for inventory completeness.

### O-71 [Confirmed-design] `set_account_meta` compare-then-write read

`storage/account.rs:121-128` does
`persistent.get::<_, AccountMeta>(&key)` to skip the
underlying `set` on no-op writes. The compare adds 1 read
per call.

Only one caller in the tree: `positions/account.rs:32` in
`create_account`. There, the account is by definition
brand-new (`increment_account_nonce`), so the compare
always returns `None` and the write always fires.

**Net: 1 wasted read per `create_account` call.**

Fix: inline the unconditional write in `create_account`:
```rust
env.storage().persistent().set(&key, &meta);
renew_user_key(env, &key);
```
Then keep the compare-then-write logic accessible for any
future caller via a separate `upsert_account_meta` helper.

### O-72 [Confirmed-low] Paired side writes re-probe the counterpart side

When a flow writes BOTH `set_supply_positions` AND
`set_borrow_positions` (process_liquidation,
process_repay_debt_with_collateral when both sides change,
process_multiply-finalize), each `write_side_map` call
independently issues `has(other_side_key)`.

Example: liquidation does `set_supply_positions(...)` —
internally `has(borrow_key)`. Then
`set_borrow_positions(...)` — internally
`has(supply_key)`. The first write JUST wrote supply, so
the second's `has(supply_key)` is guaranteed-true.

**Net: 1 redundant `has` per paired-write flow.**

Fix: optional `set_account_sides(meta_known_present,
supply_map, borrow_map)` helper that does both writes and a
single TTL renewal pass without per-side has() probes.
Trade-off: forks the side-write API for a minor saving.

### O-73 [Confirmed-design] Strategy flows: `get_account` + paired side writes

`strategy.rs:270, 349, 568` etc. read `get_account` once
(meta + supply + borrow) then write back both sides at the
end. AccountMeta is accessed:
- 1× via internal `try_get_account_meta` in `get_account`
- 1× `has` inside `set_supply_positions`
- 1× `has` inside `set_borrow_positions`

**Net: 3× AccountMeta accesses per strategy operation.**

This is the standard "load full account, mutate, flush both
sides" pattern. The 2 internal `has` reads are O-72; the
meta read inside `get_account` is necessary because side
maps don't carry meta. Defensible as designed.

### O-74 [Discuss] `account.borrow_positions.is_empty()` after liquidation

In `process_liquidation`, if liquidation closes all debt
positions and the supply side has sub-floor dust, the
post-liquidation account state could pass the bad-debt
check (no debt → no socialization) but fail the dust gate.
The dust gate skip at `:135`
(`if !will_socialize { require_no_dust_after(...) }`)
depends on `post_total_debt > post_total_coll` — if debt
hit zero, this is `0 > post_total_coll` which is false, so
the dust gate FIRES. Possible accidental revert when a
legitimate full-liquidation leaves micro-dust supply.

Need a trace test: liquidate an account to zero debt where
the residual supply rounds to sub-floor. Does the protocol
revert, leaving the account stuck? Or does
`expand_to_full_close_on_dust_residue` already absorb the
residue at the seizure level?

### O-75 [Confirmed-safe] Singleton reads (PositionLimits, Aggregator)

`storage::get_position_limits` (validation.rs:185) and
`storage::get_aggregator` (strategy.rs:423) each read once
per flow. No duplicate observed.

`get_position_limits` runs inside
`validate_bulk_position_limits` which is itself called once
per supply/borrow batch. Even in a flow that exercises
both supply and borrow (e.g. multiply), only one of the two
is called.

`get_aggregator` runs inside `swap_tokens` which is invoked
once per strategy operation that needs a swap.

**Net: 1 read each. Cache slot would be over-engineering.**

### Cross-iteration summary update

| ID | Status | Pattern | Severity |
|----|--------|---------|----------|
| O-66 | Open (cheap) | Duplicate meta read in liquidation self-check | Low |
| O-67 | Open (cheap) | `has + get` in keepalive_pools | Low |
| O-68 | Open (cheap) | First-asset Market read twice | Low |
| O-69 | Open (cheap) | Threshold updater double TTL bump | Low |
| O-70 | Open (defensible) | `renew_account` meta double touch | Low |
| O-71 | Open (defensible) | `set_account_meta` compare-then-write | Low |
| O-72 | Open (refactor) | Paired side writes counterpart has() | Low |
| O-73 | Confirmed-safe | Strategy 3× meta access — designed | — |
| O-74 | Discuss/test | Liquidation+dust-gate interaction | Med |
| O-75 | Confirmed-safe | Singleton reads — 1 each per flow | — |

Total findings to date: 75 (66 confirmed-safe / duplicate /
no-action, 9 actionable cheap wins, 1 needs test
verification).

### Next iteration focus
- Trace O-74 scenarios via a Rust unit test on a fixture
  where post-liquidation collateral residue is below floor
  and debt is exactly 0. Does the revert actually fire?
- Survey the pool side for the same duplicate-key pattern
  — `pool::cache` might have similar `has + get` pairs.
- Investigate the `apply_e_mode_to_asset_config` call chain
  for any redundant `EModeCategory` reads when a flow
  touches multiple assets in the same e-mode.

---

## Iteration 16 — 2026-05-19 22:55

### Theme: cross-flow EModeCategory reads + pool-side cache audit + O-74 deep dive

Following up the prior iteration's "next focus". Three angles:
1. Trace `EModeCategory(id)` reads across composite flows
   (multiply, swap_collateral, swap_debt, threshold update)
2. Audit pool-side `Cache::load` / `save` for the same
   duplicate-read pattern found on controller
3. Walk through the O-74 scenario in detail without writing
   a test (loop says no edits)

### O-76 [Open] `process_multiply` reads `EModeCategory(id)` twice

Trace:
- `strategy.rs:203 open_strategy_borrow` →
  `borrow.rs:836 handle_create_borrow_strategy` →
  `borrow.rs:39 emode::active_e_mode_category(account.e_mode_category_id)` →
  `emode.rs:104 storage::get_emode_category(id)` — **read #1**
- `strategy.rs:231 supply::process_deposit` →
  `supply.rs:156 emode::active_e_mode_category(account.e_mode_category_id)` →
  `emode.rs:104 storage::get_emode_category(id)` — **read #2**

**Net: `EModeCategory(id)` accessed 2× per `process_multiply`
on an e-mode account.**

Fix options:
- (a) Add a 7th cache slot: `emode_categories: Map<u32,
  Option<EModeCategory>>` with a `cached_e_mode_category(id)`
  method. One read per id per tx.
- (b) Pass the resolved `Option<EModeCategory>` from
  `open_strategy_borrow` back up to `process_multiply`,
  then thread it into `process_deposit` via a new optional
  parameter.

Option (a) is cleaner (one new cache slot, no API surface
expansion). Severity is low (2 reads vs 1) but the fix is
trivial.

### O-77 [Confirmed-safe] Pool-side cache architecture has no duplicate reads

`pool/src/cache.rs::Cache::load(env)` reads
`PoolKey::Params` and `PoolKey::State` exactly once at the
top of every pool entry. Mutations stay in the in-memory
`Cache` struct. `cache.save()` writes `PoolState` back at
the end of mutating entries.

No `has + get` pair pattern. No internal helper that
re-reads. The single `instance().get(PoolKey::State)` calls
at `lib.rs:645, 653, 905` are inside the `#[cfg(test)]`
module — `edit_state` and `state_snapshot` test helpers.

Pool architecture is structurally immune to the
duplicate-read pattern because the entire pool state lives
in one instance key. **Pool side has zero findings on this
axis.**

### O-78 [Open/Medium] Liquidation dust gate may block full-debt-close on partial-weight collateral

Detailed trace of the O-74 scenario:

Setup: account with HF just below 1.0 holding multi-asset
supply with varying liquidation thresholds. Example:
- Supply: $99 of asset A (LT=80%) + $1 of asset B (LT=50%)
- Total collateral = $100
- Weighted_coll = $99 × 0.80 + $1 × 0.50 = $79.70
- Debt: $80 (HF = $79.70/$80 ≈ 0.996)

Liquidator pays full $80 debt. Inside `execute_liquidation`:

1. `calculate_seizure_proportions`:
   - `proportion_seized = weighted_coll / total_coll`
     = 79.70 / 100 = **0.797**
2. `calculate_liquidation_amounts` returns
   `ideal_repayment_usd ≈ $80` and `bonus` from the linear
   formula.
3. Assume bonus = 5% → `seizure_usd = $80 × 1.05 = $84`.
4. `expand_to_full_close_on_dust_residue`:
   - `residual_debt = 80 - 80 = 0` → `leaves_debt_dust = false`
   - `residual_collateral = 100 - 84 = $16` → `leaves_collat_dust`
     depends on whether $16 < min_collat_floor (typically $1).
   - Since $16 > $1, no expansion triggered.
5. `calculate_seized_collateral` distributes $84 seizure:
   - Asset A share = 99/100 → seize $83.16 worth of A
   - Asset B share = 1/100 → seize $0.84 worth of B
6. Post-state:
   - A: residue ≈ $99 - $83.16 = $15.84 — above floor, OK
   - B: residue ≈ $1 - $0.84 = $0.16 — **BELOW $1 floor**
7. `process_liquidation` post-state:
   - `post_total_coll = $15.84 + $0.16 = $16.00`
   - `post_total_debt = 0`
   - `will_socialize = (0 > $16) = false` → dust gate fires
   - `require_no_dust_after`: asset B has 0 < $0.16 < $1
     floor → **panic `DustResidueNotAllowed`**

**Result: liquidator's tx reverts. The account remains
unhealthy but cannot be liquidated.**

Recovery analysis:
- `clean_bad_debt_standalone` requires `total_debt >
  total_coll AND total_coll <= BAD_DEBT_USD_THRESHOLD`.
- Pre-liquidation: `$80 < $100` → bad-debt path not eligible.
- Account is stuck: barely unhealthy, multi-asset supply,
  one position below dust floor proportional to its weight.

Mitigations to debate next iteration:
- (i) `expand_to_full_close_on_dust_residue` could also
  inspect per-asset dust on the supply side (not just the
  aggregate-level test it does today) — but expanding the
  repayment doesn't help, the seizure is bound by
  `proportion_seized`.
- (ii) Allow liquidator to specify an extra "dust sweep"
  amount that absorbs the sub-floor residue on whatever
  asset would otherwise dust out. Adds API complexity.
- (iii) Skip the dust gate when post-liquidation
  `total_debt == 0` — let the dust persist on the supply
  side until the owner withdraws it. Simple but means we
  accept supply-side dust on closed-debt accounts.
- (iv) During seizure distribution, force-seize any
  position that would otherwise land in `(0, floor)` —
  effectively burn the residual. Loses fairness across
  collateral assets but unblocks the liquidation.

**Severity: Medium.** This is a real edge case with a
plausible trigger (multi-collateral debt-stable account
liquidated when one position is small). Recovery requires
the position to drift further into bad-debt territory,
which is undesirable for the protocol.

### O-79 [Open/Medium] `update_account_threshold` keeper batch — `EModeCategory(id)` read N times per batch

The keeper entry (supply.rs:33-65):
```rust
pub fn update_account_threshold(
    env, caller, asset, has_risks,
    account_ids: Vec<u64>,
) {
    let mut cache = ControllerCache::new(...);
    // asset-side reads are cached once:
    let base_config = cache.cached_asset_config(&asset);
    let price_feed = cache.cached_price(&asset);

    for account_id in account_ids {
        update_position_threshold(env, account_id, ..., cache);
    }
}
```

Inside `update_position_threshold` (supply.rs:386+) per
account:
- `try_get_account_meta(account_id)` — necessary, per-account
- `get_supply_positions(account_id)` — necessary, per-account
- `get_borrow_positions(account_id)` conditional — per-account
- **`emode::e_mode_category(env, meta.e_mode_category_id)`**
  at supply.rs:422 — natively reads `EModeCategory(id)` via
  `storage::get_emode_category(id)`. **NOT cached.**
- `cache.cached_emode_asset(meta.e_mode_category_id, asset)`
  — cached, good.
- `renew_user_account(account_id)` — per-account
- `set_supply_positions(account_id, ...)` — per-account

**Scenario: keeper passes 100 accounts that all share
e-mode category 1.** Native EModeCategory(1) reads: **100**.
With a cached `cached_e_mode_category(id)` slot: **1**.

**Net: O(N) duplicate reads per keeper batch.** This is the
keeper's hot path — `update_thresholds` may run nightly
against thousands of accounts.

Fix: same as O-76 — add `emode_categories: Map<u32,
Option<EModeCategory>>` to `ControllerCache`. The same slot
fixes both findings.

### O-80 [Confirmed-safe] Strategy compositions other than multiply have ≤1 EModeCategory read

Traced:
- `process_swap_debt` (strategy.rs:254+): only
  `open_strategy_borrow` reads e-mode → **1 read**
- `process_swap_collateral` (strategy.rs:333+): only
  `process_deposit` for new collateral reads → **1 read**.
  Withdraw side (`withdraw::execute_withdrawal`) doesn't
  re-read.
- `process_repay_debt_with_collateral` (strategy.rs:545+):
  no e-mode read at all in the active sub-flows
  (withdraw + swap + repay). **0 reads.**

Only multiply has the borrow-then-supply composition that
triggers the duplicate.

### Cross-iteration summary update

| ID | Status | Pattern | Severity |
|----|--------|---------|----------|
| O-76 | Open (cheap) | multiply reads EModeCategory ×2 | Low |
| O-77 | Confirmed-safe | Pool-side cache architecture | — |
| O-78 | Open (debate) | Liquidation dust on partial-weight | Medium |
| O-79 | Open (medium) | Threshold keeper O(N) EModeCategory reads | Medium |
| O-80 | Confirmed-safe | Other strategies single EModeCategory read | — |

Total findings to date: 80
- 70 confirmed-safe / duplicate / no-action
- 7 actionable cheap wins (O-66..O-71, O-76)
- 1 design refactor (O-72)
- 2 open mediums (O-74/O-78, O-79)

### Reconciling O-76 and O-79 with one fix

A single `cached_e_mode_category(id)` slot resolves both:

```rust
pub fn cached_e_mode_category(&mut self, id: u32) -> Option<EModeCategory> {
    if id == 0 { return None; }
    if let Some(cached) = self.emode_categories.get(id) {
        return cached;
    }
    let cat = Some(storage::get_emode_category(&self.env, id));
    self.emode_categories.set(id, cat.clone());
    cat
}
```

Then change `emode::e_mode_category(env, id)` to take
`&mut cache` and route through this. The `env`-only signature
is the source of the bypass — `e_mode_category` has no way to
talk to the cache today.

API ripple count (call sites that need the `&mut cache`
parameter added):
- supply.rs:156 (process_deposit) — already has cache
- supply.rs:422 (update_position_threshold) — already has cache
- borrow.rs:39 (handle_create_borrow_strategy) — has cache
- borrow.rs:131 (process_borrow_plan) — has cache
- account.rs:20 (create_account) — has env only; would
  need to take cache OR keep the native read for the cold
  account-creation path

Cold path: a single new account creation isn't worth a cache
slot. Either (a) keep `account.rs:20` on the native path
and only convert hot paths, or (b) make
`active_e_mode_category` polymorphic with an optional cache.

**Recommendation:** convert the 4 hot-path sites; keep
`account.rs:20` native (account creation already opens its
own cache via `process_supply` → `resolve_supply_account` →
`utils::create_account_for_first_asset`).

### Next iteration focus
- Walk `process_repay_debt_with_collateral` and
  `process_swap_collateral` for the same "load full account
  then write both sides" pattern as liquidation — confirm
  AccountMeta access count.
- Audit `oracle::compose::resolve_components` for any
  silent fall-through that doesn't surface as a panic.
- Probe `pool::flash_loan` callback contract for whether
  the pool re-enters the controller via the SAC callback
  paths beyond the reentrancy-flag check.
- Trace `set_borrow_positions` + `set_supply_positions`
  paired-write order for any flow where the SECOND write
  could see a partially-stale memory snapshot.

---

## Iteration 17 — 2026-05-19 23:25

### Theme: oracle composition pipeline — slot semantic enforcement

Walked `oracle/compose.rs`, `oracle/tolerance.rs`,
`oracle/mod.rs`, `oracle/validation.rs`, and cross-checked
against the test-harness oracle presets at
`verification/test-harness/src/helpers.rs`.

### O-81 [Open/Medium] Admin can invert `primary` ↔ `anchor` semantics; no validation guard

#### The convention

Across the codebase, `primary`/`anchor` carry an implicit
semantic mapping:

| Slot      | Semantic role | Test-harness convention      |
|-----------|---------------|------------------------------|
| `primary` | "safe" source | TWAP (Reflector with Twap)   |
| `anchor`  | "aggregator"  | Live spot / RedStone         |

Evidence:
- `oracle/mod.rs:42-52` `PriceComponents` view maps
  `aggregator_price_wad ← components.anchor_price_wad`
  and `safe_price_wad ← components.primary_price_wad`.
- Test harness `helpers.rs:122-123` standard preset uses
  `primary: Twap(3), anchor: Some(Spot)`.
- `keeper_tests.rs:168` comment: "Skew the TWAP/safe
  source so primary and anchor disagree" — confirms
  `primary == TWAP == safe`.

#### The deviation policy

`OraclePolicy::Liquidation` doc-comment in
`oracle/policy.rs:25-35`:
> "On unsafe deviation it resolves to the aggregator
> (spot) rather than the safe source so liquidation
> tracks the live market instead of getting stuck behind
> a slower TWAP."

`oracle/tolerance.rs::calculate_final_price` on outside-
tolerance + `prefers_aggregator_on_deviation`:
```rust
if cache.oracle_policy.prefers_aggregator_on_deviation() {
    agg_price   // bound from compose.rs:74 = anchor.price_wad
} else {
    safe_price  // bound from compose.rs:75 = primary.price_wad
}
```

So the comment + code only hold IF the admin actually
configured `primary = TWAP/safe` and `anchor = Spot/agg`.

#### The hole

`oracle/validation.rs::validate_oracle_config_shape` enforces:
- `PrimaryWithAnchor` requires anchor present
- `Single` rejects anchor

It does NOT enforce:
- `primary.read_mode == Twap(_)` for Reflector primary
- `anchor.read_mode == Spot` for Reflector anchor
- Per-provider "safe vs aggregator" semantic

#### Mis-configuration impact

If admin sets `primary = Reflector(Spot), anchor =
Reflector(Twap(N))`:
1. The view `PriceComponents.safe_price_wad` returns the
   Spot value (mislabeled).
2. The view `PriceComponents.aggregator_price_wad` returns
   the TWAP value (mislabeled).
3. Under `Liquidation` policy + unsafe deviation:
   `prefers_aggregator_on_deviation` returns the
   `agg_price` slot, which holds the TWAP value.
   **Liquidation now uses TWAP during a flash crash —
   the exact failure mode the comment warns against.**
4. Borrowers stay paper-healthy through a real crash;
   bad debt accrues to lenders.

#### Severity: Medium

The configuration is admin-controlled, so this is a
governance-failure mode rather than an attacker primitive.
But:
- The convention is undocumented in the public types.
- The mistake is silent — no test, no panic catches it.
- The view fields would lie to off-chain consumers.
- The keeper liquidation flow would behave inversely
  during the worst case.

#### Fix options

(a) **Enforce at admin time** — extend
`validate_oracle_config_shape`:
```rust
if config.strategy == OracleStrategy::PrimaryWithAnchor {
    if let OracleSourceConfigInput::Reflector(p) = &config.primary {
        if !matches!(p.read_mode, OracleReadMode::Twap(_)) {
            panic_with_error!(env, OracleError::InvalidPrimaryReadMode);
        }
    }
    // Anchor: any read mode acceptable (RedStone is always Spot;
    // Reflector Spot/Twap both valid as the cross-check).
}
```

(b) **Rename slots** to `safe`/`aggregator` instead of
`primary`/`anchor` in the storage type. Bigger churn but
makes the semantic explicit. Backwards-compatible
deprecation would need a migration window — but we're
deploying fresh.

(c) **Document the convention** in the
`MarketOracleConfig` doc-comment and the admin-facing
`set_market_oracle_config` entry. Cheapest fix; relies
on admin diligence.

Recommendation: (a) for hard enforcement + (c) for
documentation. The validation panic gives admins
immediate feedback if they typo the read_mode.

### O-82 [Cosmetic] `compose.rs:93` timestamp branch inference

```rust
let timestamp = if final_price == primary.price_wad {
    primary.timestamp()
} else {
    core::cmp::min(primary.timestamp(), anchor.timestamp())
};
```

Inferred branch from price equality. If the average
`(agg + safe) / 2` or the aggregator value (under
liquidation deviation) happens to equal
`primary.price_wad` exactly, we report `primary.timestamp()`
even though the branch wasn't "primary wins".

Trigger requires `primary.price_wad == anchor.price_wad`
(stable price) or `primary == (primary + anchor) / 2`
(implies primary == anchor). Both rare.

**Impact:** The reported feed timestamp can be wrong by
the gap between primary and anchor timestamps. Downstream
freshness checks would still pass since timestamps in
both branches are recent. Cosmetic, no correctness issue.

**Fix:** plumb the branch identity (`first_in_tolerance`,
`mid_in_tolerance`, `prefer_agg`, `prefer_safe`) out of
`calculate_final_price` and use it to select the
timestamp explicitly.

### O-83 [Discuss] `OracleStrategy::Single` removes the deviation defense entirely

`compose.rs:49-56` Single-strategy branch:
```rust
OracleStrategy::Single => ResolvedOracleComponents {
    primary_price_wad: Some(primary.price_wad),
    anchor_price_wad: None,
    final_price_wad: primary.price_wad,
    timestamp: primary.timestamp(),
    within_first_tolerance: true,    // hard-coded
    within_second_tolerance: true,
},
```

Implications:
- No deviation gate, no second-source cross-check.
- `Liquidation` policy `prefers_aggregator_on_deviation`
  has no effect — there's no aggregator.
- Only `RiskIncreasing`-style policies still benefit from
  the freshness gate at line 46
  (`validate_primary_freshness`).
- Sanity bounds (`oracle/price.rs:51-56`) remain as the
  last circuit breaker.

For deeply-pegged stablecoin assets (USDC, USDT) where
TWAP smoothing adds latency without security value, this
is a defensible admin choice. For volatile assets it
removes a key defense.

**Recommendation:** document that `Single` is for
peg-stable assets only, OR add a per-market policy flag
that requires `Single` admin opt-in with an attestation.
No code action needed if documented.

### O-84 [Confirmed-safe] `OracleStrategy::PrimaryWithAnchor` + `config.anchor = None` is correctly trapped

`compose.rs:58-60`:
```rust
let Some(anchor_config) = config.anchor.as_ref() else {
    return fallback_to_primary(cache, primary);
};
```

`fallback_to_primary` panics on
`!allows_missing_twap_fallback()` — i.e., RiskIncreasing
will revert.

`validate_oracle_config_shape` (line 56-62) already rejects
this combination at admin time. So the in-flight check at
`compose.rs:58` is a defense-in-depth backup. Both layers
present.

**Net: no issue.**

### O-85 [Confirmed-safe] Single-source missing-feed path panics correctly

`oracle/providers/mod.rs:11-30`:
```rust
pub(crate) fn read_source(..., required: bool) -> Option<OracleObservation> {
    let observation = match source { ... };
    if required && observation.is_none() {
        panic_with_error!(cache.env(), OracleError::NoLastPrice);
    }
    observation
}
```

`compose.rs:44` calls `read_source(... primary_max_stale, true)` with `required=true`.
A missing primary observation panics on this path —
no silent fall-through.

Anchor read at `compose.rs:62` uses `required=false`,
so a missing anchor returns `None` and is handled by
the `anchor_is_usable` check below.

**Net: no issue.**

### O-86 [Open/Low] `price_components` view exposes raw slot names that perpetuate the convention ambiguity

`oracle/mod.rs:34-40`:
```rust
pub struct PriceComponents {
    pub aggregator_price_wad: Option<i128>,
    pub safe_price_wad: Option<i128>,
    ...
}
```

This is a public SDK-visible type. The names
`aggregator`/`safe` carry semantic claims (live vs
stable) that the underlying storage doesn't enforce
(see O-81). If admin mis-configures primary/anchor, the
view will display values under the wrong field name,
misleading off-chain consumers.

**Fix:** depends on O-81 fix. If (a) is taken (admin-time
enforcement), the view becomes truthful. If only (c)
(documentation), the view comments must warn that the
fields reflect storage convention and are admin-trustable
only.

### Cross-iteration summary update

| ID  | Status               | Pattern                              | Severity |
|-----|----------------------|--------------------------------------|----------|
| O-81| Open                 | Primary/anchor inversion possible    | Medium   |
| O-82| Cosmetic             | Timestamp branch inference           | Low      |
| O-83| Discuss              | Single strategy no deviation gate    | Info     |
| O-84| Confirmed-safe       | PWA + None anchor trapped twice      | —        |
| O-85| Confirmed-safe       | Missing primary panics correctly     | —        |
| O-86| Open (depends on O-81)| View field naming perpetuates ambiguity | Low |

Total findings to date: 86
- 73 confirmed-safe / no-action
- 8 actionable cheap wins
- 2 medium open (O-78 dust gate, O-81 oracle inversion)
- 1 keeper hot-path optimization (O-79)
- 1 design refactor (O-72)
- 1 cosmetic (O-82)

### Reconciling O-81 with deployment posture

The user noted earlier this session that "we will deploy
fresh no need for backwards compatibility or migrations".
This is the right moment to enforce the
primary=TWAP/anchor=Spot convention at admin time —
post-deploy, retrofitting the validation would require
data migration. Recommend implementing fix (a) BEFORE
mainnet pool deployment.

### Next iteration focus
- Walk `oracle/providers/redstone.rs` end-to-end for
  the `read_price_data_for_feed` error handling — the
  `try_*` chain at `validation.rs:140-143` has two layers
  of `unwrap_or_else`. Confirm both ConversionError and
  InvokeError paths are covered.
- Audit `oracle/providers/reflector.rs::twap` for
  per-observation freshness gating: a TWAP window with
  some stale observations and some fresh — does the gate
  fire on the freshest, oldest, or aggregate timestamp?
- Trace `strategy::process_repay_debt_with_collateral`
  for same-asset short-circuit when
  `collateral_token == debt_token`. Are the position
  reads/writes idempotent?
