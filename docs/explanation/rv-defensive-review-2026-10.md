# Defensive review for the Runtime Verification audit window (2026-10)

This note records an internal adversarial pass over the controller, pool,
`common` and price aggregator, run while the Runtime Verification audit is in
progress. One lead reviewer wrote the hypothesis list from a full read of the
four crates; eight workers turned the hypotheses into executable tests; the
lead re-ran every suite, re-read the code behind every suspected finding and
adjudicated severity. Nothing here is a claim without a test.

Every test lives in its own binary under `tests/test-harness/tests/rv_*.rs`,
so one file is one `cargo test -p test-harness --test <name>` run. A test
named `rv_finding_*` is ignored and asserts the documented or safe
expectation; it fails when run with the ignored filter, which is the reproduction.

| Suite | Tests | Ignored | Scope |
|---|---|---|---|
| `rv_auth_matrix` | 14 | 0 | Owner, delegate, NFT holder, pool owner, admin and two-step ownership gates |
| `rv_liquidation_economics` | 17 | 2 | Coverage ratio, self-liquidation, socialization, dust promotion, chains, under-delivery |
| `rv_strategy_edges` | 22 | 2 | Flash position, multiply, swaps, netting, pause matrix, re-entry, router residue |
| `rv_backing_invariant` | 16 | 0 | Pool backing identity under random operation sequences, donations, recapitalization, revenue |
| `rv_oracle_edges` | 17 | 3 | Dual-source, staleness, skew, sanity band, scaled sources, TWAP, quotes/prices, decimals boundary |
| `rv_pool_direct` | 16 | 1 | Pool driven as its owner: validation, rounding, flash exactness, gates, write-down, model switch, ceiling |
| `rv_caps_flags` | 28 | 2 | Caps on every path, saturation, halt-flag matrix, ratchet and epoch, position limits, usage identity, spokes |
| `rv_accrual_consistency` | 13 | 0 | View equals mutator, cadence, allocation, ceiling and overflow horizon, curve, backwards ledger |

## What held

The protocol defended every hypothesis that targets funds or authority. The
lead confirmed each of these by re-running the suite:

- **Authority.** A stranger is refused on every owner verb with no side
  effects and keeps exactly the permissionless verbs. Delegate grants follow
  the active-manager flag and NFT custody, revive when the NFT returns without
  a delegate write (documented), and are purged by the next owner's write.
  Every pool mutator and every admin entrypoint refuses a non-owner exact
  invocation while the owner reaches the body. Two-step ownership including
  expiry holds. NFT operator approval is full account control, as documented.
- **Liquidation economics.** On solvent multi-leg accounts no partial
  liquidation lowers the coverage ratio in the band, cap or curve regime
  (deterministic cases plus a 24-case proptest over thresholds, bonuses, fees
  and decimals). Self-liquidation through a credit receiver recaptures the
  liquidator's profit and nothing more; the protocol fee is identical. A
  solvent account is never socialized and no index moves; an insolvent
  collateral-backed quote seizes the book and socializes the residual. Six
  partial steps never out-seize one close. Fee-on-transfer debt pays
  collateral on the received value only, in both seize modes. The pool
  supplied-minus-revenue identity and the spoke usage identity held after
  every liquidation.
- **Strategies.** The self-referential flash position (debt asset returned as
  collateral) is admitted and every gate still binds: the LTV boundary, both
  spoke caps and the pre-deposit utilization gate; the zero-cash loop costs the
  rate spread (about 10.8% of notional per year at 28.6% utilization). Refund
  rules, minimum checks, router shortfall, zero output and overspend settle to
  one raw unit or revert everything. The pause matrix matches `lib.rs` exactly
  (13 gated, 7 open). Router and callback re-entry is refused by the host with
  clean books. No reusable router allowance survives a strategy.
- **Backing.** Across 6,400 random operations on zero-seed books (supply,
  borrow, partial and over-repayment, partial and full withdrawal, accrual,
  revenue claims, recapitalization, flash loans, same-asset netting, direct
  donations) every market kept a zero backing shortfall, revenue within
  supplied, custody equal to cash plus donations, non-decreasing indexes and
  exact agreement between the account share books and the pool totals. A
  standard write-down never creates a shortfall; only the index floor clamp
  does, and supply is then refused until exactly the shortfall is
  recapitalized. Donations are never cash and are never paid out.
- **Oracle.** A stale leg is never papered over and staleness precedes
  disagreement; the tolerance boundary is inclusive and the midpoint floors;
  leg age spread is exact at 3600 s and future skew at 60 s; sanity band edges
  are exact, narrowing fails every valuation path closed while supply survives,
  widening is refused; scaled factor bands, stale nested quotes and cycles fail
  closed; `prices` and `quotes` agree on twelve failure modes.
- **Pool.** Negative amounts are rejected on every mutator; phantom positions
  trap on every full exit; rounding at every boundary matches `formulas.md`
  when rebuilt in BigInt; flash-loan repayment is exact in both directions and
  the fee is the only book change; the utilization gate and the liquidation
  buffer bind at the exact unit; the write-down formula is exact and the floor
  clamp leaves the documented residual until recapitalization; a model switch
  accrues under the old model first; the index ceiling is exact and inert.
- **Caps, flags, limits.** Every debt-creating and deposit path respects its
  cap at the exact unit; a saturated scaled cap at the index floor never admits
  a deposit above the configured cap, because the scaled-share ceiling trips
  first (the documented fail-open is in practice a lower ceiling); the halt
  flag matrix, bad-debt bypass, ratchet and epoch rules match INV-HALT-02 and
  ADR-0007; position limits and whole-unit isolation hold; deprecated spokes
  block entry and keep exits; the spoke binding is immutable on every path.
- **Accrual.** The view projection equals the mutator bit for bit over random
  books including multi-chunk accrual; zero elapsed time is a no-op; the
  allocation never books more than accrued interest (largest unbooked residue
  one raw RAY unit); the curve is monotone and capped; a backwards ledger is
  inert and never moves `last_timestamp`.

## Findings

No finding moves funds beyond the configured bonus or bypasses an
authorization gate. Severity is the lead's assessment.

### F1. Hardening: the controller never cross-checks pool decimals against oracle decimals

`rv_oracle_edges`: `rv_finding_liquidate_usdc_decimals_six_under_seizes_tenth`,
`rv_finding_liquidate_usdc_decimals_eight_overseizes_full_balance`,
`rv_finding_liquidate_eth_decimals_six_overoffer_pays_fraction_for_full_seizure`.

Valuation ignores `asset_decimals` (shares times index to WAD times price), but
liquidation applies the oracle's decimals twice: to convert seized value into
base units that the pool then reads in its own decimals, and to value the
repayment. With a 7-decimal USDC market and the oracle reporting 6 decimals, a
0.3 ETH repayment seizes 126 USDC instead of 1,260. With the oracle reporting
8 decimals the planned 12,600 USDC exceeds the 10,000 USDC position, the pool
treats the request as a full close, and the remaining 2.7 ETH of debt is
socialized against ETH suppliers (supply index 1.0 to 0.73). With the debt
oracle at 6 decimals a 1.0 ETH offer pays 0.238 ETH, seizes the whole 10,000
USDC position and socializes 2.76 ETH.

Reachability: governance refuses a decimals change on `set_oracle` and pins
listed decimals at proposal time, so the mismatch needs a seeded registry, an
aggregator re-point to a differently configured registry, or an upgrade.
The threat model assigns this to operator checks. Severity: high impact, low
likelihood; classified as hardening.

Recommendation: assert equality at the mutation boundary, where both values
are already in hand. In `apply_liquidation_repayments` and
`apply_liquidation_seizures` each pool mutation returns `asset_decimals` and
each plan entry carries `feed.asset_decimals`; assert they match with
`OracleError::InvalidOracleDecimals`. A load-time check in
`Context::load_markets` would also protect estimates and views at the cost of
one pool sync read per hub asset.

### F2. Low: flash_position in Long or Short mode skips the distinct-asset rule

`rv_strategy_edges`: `rv_finding_flash_position_long_same_market_should_reject`,
`rv_finding_flash_position_long_cross_hub_token_should_reject`.

`multiply` rejects a Long or Short pair whose collateral and debt share a token
with `AssetsAreTheSame`, and `docs/reference/errors.md` documents that rule for
both modes. `flash_position` checks only that the mode is Multiply, Long or
Short, so a Long flash position with ETH debt and ETH collateral (same market
or another hub) is admitted. The mode is a label with no risk parameter behind
it, and the Multiply-mode loop is intentional under INV-STRAT-04, so there is
no fund path. Recommendation: apply the `multiply` distinctness rule to Long
and Short in `validate_collaterals`, or amend the error reference.

### F3. Low: insolvent quote on an under-delivering debt token does not seize every unit

`rv_liquidation_economics`: `rv_finding_liq_insolvent_fee_on_transfer_quote_seizes_every_unit`.

`seize_all` is decided on the planned repayment; INV-LIQ-03 then floor-scales
the seizure by received over planned. On an insolvent account with a 10%
fee-on-transfer debt token, the collateral-backed quote leaves 10% of the
collateral in the account with the residual debt above the $5 cleanup gate,
while `formulas.md` says that call seizes every unit. No value moves beyond
the bonus rate; the residual waits for another liquidation or the owner's
force-socialize path. Recommendation: state the under-delivery exception next
to the "seizes every unit" sentence, or re-evaluate `seize_all` after scaling.

### F4. Informational: an estimate taken before accrual is not a bound for a trimmed over-offer

`rv_liquidation_economics`: `rv_finding_liq_over_offer_seizure_exceeds_t0_estimate_plus_one_unit`.

Five seconds of accrual raise the debt, so the trimmed quote grows and an
over-offer executed one ledger later repays 1,308 more debt units and seizes
920 more USDC units than the earlier estimate. Execution equals the estimate
taken on its own ledger within one unit, and the extra is the bonus rate on the
extra repayment. `formulas.md` lists only rounding and measured receipt as
causes of quote drift; accrual belongs on that list.

### F5. Low: the pool accepts a negative scaled position on supply, borrow and create_strategy

`rv_pool_direct`: `rv_finding_negative_scaled_position_accepted_on_supply_borrow_strategy`.

A position of `-5` with a one-unit amount returns a stored position of
`1e27 - 5`. INV-ACCT-10 places that trust on the controller and no user input
reaches the position argument, so this is defence in depth: add
`require_nonneg_amount` on `position.scaled_amount` in `ops::load_leg`, which
already validates the amount. The exit paths reject a negative position, but
with `MathOverflow` or `RepayRoundsToZeroShares` rather than a typed error.

### F6. Low: "whole unit" is ambiguous between the documents and the code

`rv_caps_flags`: `rv_finding_two_decimal_debt_floor_counts_raw_units_not_whole_units`,
`rv_finding_two_decimal_withdraw_leaving_one_unit_with_debt_admitted`.

`formulas.md` says a collateral leg below 3 decimals needs "at least 2 whole
units while in debt". `require_whole_unit_collateral_floor` counts base units,
and the Liqvid listing runbook says "2 shares", which for the 0-decimal Liqvid
tokens is the same thing. For a hypothetical 2-decimal listing the code floor
is 0.02 whole tokens. The whole-unit liquidation math is written in base units
throughout, so the code is self-consistent; the tests encode the literal
reading of the document. Recommendation: define "unit" as the token's smallest
indivisible unit in `formulas.md` and rename the constant, or change the floor
to `MIN_WHOLE_UNIT_COLLATERAL * 10^decimals` if whole tokens were intended.

### F7. Informational: non-owner calls to delegate and renew entrypoints return AccountNotInMarket

`rv_auth_matrix`. `add_delegate`, `remove_delegate` and `renew_account` fail
in `require_account_owner` with `AccountNotInMarket` (13) rather than
`NotAuthorized` (44). The outcome is a refusal either way; the code is
misleading to integrators.


### F8. Economic, configuration dependent: revenue claims can take cash against uncollectible interest before cleanup

`rv_backing_invariant`: `rv_uncleaned_revenue_claim_drains_cash_and_leaves_floor_residual_when_ceiling_disabled`,
`rv_uncleaned_bad_debt_accrual_writes_off_phantom_yield_without_shortfall`.

An insolvent account that nobody cleans keeps accruing interest, and the
reserve factor turns part of that uncollectible interest into revenue shares.
On a market whose utilization ceiling is disabled, a permissionless
`claim_revenue` then pays the whole remaining cash (6 ETH against 1,487 ETH of
phantom revenue after three years in the fixture) to the accumulator; the
later cleanup clamps the supply index to its floor and leaves the suppliers'
100 ETH of shares worth 0.1 ETH. With the default 95% ceiling the same claim
is refused with `UtilizationAboveMax`, because utilization is already at the
top, and cleanup leaves the suppliers 4.74 ETH against 6 ETH of cash.

The mainnet configuration disables the ceiling on the collateral-only RWA and
LP listings and keeps 90% to 95% on the borrowable markets, so the
precondition does not appear to hold today; it would if a listing with a
disabled ceiling became borrowable. The cash goes to the protocol accumulator,
not to a third party, and governance can return it through `recapitalize`.
Recommendations: keep a ceiling below one RAY on every borrowable market, keep
bad-debt cleanup prompt (the keeper and the force-socialize runbook), and
consider gating `claim_revenue` on a positive post-claim cash buffer
independent of the utilization ceiling.

## Economic observations for the auditors

These are documented mechanics whose magnitude the tests measured.

1. **Accrual cadence at high utilization.** Over one year on a 100k USDC book,
   one accrual step versus 365 daily steps differs by 6.5e-5 at 10%
   utilization, 0.34% at 60% and 31% at 90% (borrow index 2.458 versus
   3.230). Coarser cadence favours borrowers. Any caller sets the cadence
   through the permissionless `update_indexes`. `formulas.md` disclaims cadence
   independence; the size at the top of the curve is worth a note in the
   operations guidance.
2. **Value-overflow freeze horizon.** A 97,000 USDC debt on a 100,000 USDC
   book at the 200% rate cap aborts accrual with `MathOverflow` after 8.89
   years, before the 1e36 index ceiling. From that point every accruing path
   fails, including `update_params`, so the rate cannot be lowered without a
   pool upgrade. The horizon in years is about the natural log of the ratio between the i128 maximum and the raw debt times 1e27, divided by the annual rate.
   Documented in the numeric limits; the `update_params` consequence is not.
3. **Unclaimed revenue compounds at the supplier rate.** After a long idle
   period at the cap, revenue shares reach two thirds of supplied shares. This
   follows from revenue being supply shares; claims still sum to book value.
4. **Uncleaned insolvency inflates supplier balances.** With one insolvent
   account left for three years at the rate cap, a 100 ETH supplier claim grew
   to 5,610 ETH before cleanup restored it to 4.74 ETH. Withdrawals stay
   bounded by cash, but the exit-ahead advantage documented in the threat model
   scales with this accrual. Prompt cleanup limits it.
5. **Utilization griefing via self-referential flash positions.** One loop
   can halve the borrowing headroom of other users and, in an empty market,
   reach exactly 100% utilization at zero net cash. The cost is the rate
   spread on the notional and roughly a third of the debt as collateral.
6. **Thin-market strategy fee.** A strategy fee minted into `supplied` on a
   market with no real suppliers activates the utilization gate against the
   next strategy debt. Fail-closed liveness only.

## Second pass: adversarial hunt for critical and high defects

After the suites above, a second pass looked only for defects that drain cash
or supplier claims, break the accounting identities, steal or bypass revenue,
escape a solvency or authorization gate, or brick a market cheaply. Six
independent finders, each with one attack lens (accounting conservation,
liquidation value extraction, index and precision extremes, token semantics
versus measured receipts, composition and cross-contract ordering, oracle and
configuration races), traced the code with concrete numbers; any candidate
would have gone to a skeptic and then to an executable proof of concept. The
finders raised no candidate. Their negative evidence, in brief:

- Liquidation: an exact-integer mirror of the plan and seizure arithmetic swept
  18,000 random mainnet-like books (one to three collateral legs at 7, 8, 9 and
  18 decimals, one or two debt legs, indexes from one RAY to 1.5 RAY and the
  floor, offers from 1% to 300% of debt) under both mainnet curves. The
  liquidator's net collateral never exceeded repaid times one plus bonus by a
  single WAD unit in either seize mode, and the insolvent quote never underpaid
  the collateral-backed amount by more than one native debt unit.
- Accounting: every controller position writer takes its value from the pool's
  returned mutation or from the credit-mode triple that is asserted to sum
  exactly; a fresh executable probe of credit-mode liquidation into a receiver
  that already held supply and debt in the same market, followed by cleanup,
  net settlement and closure, left every identity gap at zero. A second probe
  listed one token on two hubs and showed each hub's cash book, flash-loan
  reserve check and revenue stay isolated while the pool balance equals the
  sum of the books.
- Precision: the surplus (cash plus debt value minus supplier claims) is
  non-decreasing under every operation except the documented floor clamp, so
  utilization cannot exceed one, the supply index can never bind at its cap
  before the borrow index, and the rounds-to-zero-shares gates are unreachable
  for positive amounts at any admitted decimals. Cap saturation at the floor
  index affects one mainnet supply cap (AQUA), which only admits more
  collateral.
- Tokens: every inbound leg credits a measured recipient delta and every
  outbound refund is sized from that measured receipt; no path transfers a raw
  controller balance, so a residual left by a hostile token or router cannot be
  swept by a later caller.
- Composition: each top-level verb reloads the account and resolves ownership
  from the NFT; nothing a user controls moves a health factor below one inside
  one ledger; callbacks, routers and Blend carry no controller authority, so
  pool mutators are unreachable from them; the host refuses re-entry into any
  contract on the call stack.
- Oracle and configuration: risk stamps are only ever written from the current
  listing; LTV is restamped ungated on every risk-increasing action and the
  effective LTV is the minimum of LTV and threshold, so a stale generous
  threshold never raises borrowing power; the midpoint skew bound at the
  mainnet tolerances stays inside the lender-safety bound of the threat model;
  Aquarius fair value is at most the redemption value in every reachable
  state.

Two observations below the bar came out of the pass and are recorded here:

- A debt-free account that receives shares through a credit-mode liquidation
  keeps its own risk tuple without the gated refresh that a supply would run
  (ADR-0019 documents this). Its owner can then borrow in the same transaction
  to a health factor below 1.05 and keep a threshold that governance has
  already lowered. The exposure is the same class as the restamp ratchet and
  needs a prior governance tightening.
- A supply top-up to an indebted account whose leg has a pending
  liquidator-favouring listing change values the whole account for the 1.05
  gate, so a feed outage on an unrelated debt asset blocks that supply.
  Availability only.

### The documented risk worth escalating

The threat model's DoS.1 says an indebted borrower can add a dust leg of any
listed collateral and choose which feed outage shields the account, and that
for an Aquarius LP leg liquidity providers can cause that outage by taking the
pool below `min_pool_value_wad`. `audit_supply_stale_shield.rs` pins the stale
feed case. The mainnet spoke layout makes the LP variant practical: spokes 6
to 9 list up to nine LP collaterals beside the borrowable majors, with LP
floors of $200k to $1M. A borrower who also provides liquidity to one of the
thinner pools can take the pool under its floor at will, which halts every
valuation of its account (liquidation, cleanup, withdrawal) until the pool is
refilled, and then restore it. That is a self-serve liquidation delay whose
cost is only the liquidity the attacker holds anyway. Lenders carry the
bad-debt risk while the delay lasts.

Options, in increasing order of change: keep LP collateral in spokes that do
not share accounts with the majors, so a dust LP leg cannot be added to a
USDC or XLM account; size each `min_pool_value_wad` far below the plausible
pool value and monitor the gap; or add a guardian-settable listing flag that
excludes an asset from valuation and seizure while its feed is known to be
unusable, so the rest of the account stays liquidatable.

## Third pass: break attempt on the router, the DeFindex adapter and governance

Same method as the second pass, pointed at the three contracts the first two
passes left out: eight attack lenses (three on the swap aggregator, two on the
DeFindex adapter, three on governance) read the sources against a brief of
seeded hypotheses, a skeptic re-derived every candidate from code and existing
tests, and anything that survived was to become an executable proof. Each lens
also left its probes behind as tests. One integration test was written outside
the workflow: the controller driving the production router WASM, which no
earlier test did.

Result: no candidate survived. The router and the adapter raised none; the
four governance candidates are the documented reach of a stolen non-owner
proposer key, refuted as code defects and escalated below because the
configured delay makes the documented control (a canceller veto) nominal.

### Suites added

| Suite | Tests | Pins |
|---|---|---|
| `contracts/swap-aggregator/tests/unit/rv_router_reserves.rs` | 5 | Greedy Aquarius, LP mint, LP burn and Comet venues under enforcing auth cannot take a second unit of the input or one unit of a parked fee reserve; a duplicate registry entry round-trips with one input fee |
| `contracts/swap-aggregator/tests/unit/rv_router_fees.rs` | 4 | Fee base is the whitelisted side's final balance; a referral owned by the router itself keeps `ReservedTotal` equal to the bucket sum; components floor independently; the route minimum is enforced net of fee |
| `contracts/swap-aggregator/tests/unit/rv_router_auth.rs` | 3 | A hop pool cannot replay the sender's transfer entry; a mint whose pool pulls less than offered is refused under enforcing auth; the index-based same-token check is documented |
| `tests/test-harness/tests/rv_real_router.rs` | 4 | Production router WASM behind the controller: settlement equals the pool payout, fees come off the input into backed buckets a sweep cannot take, an unmet route minimum reverts the whole multiply, and under enforcing auth the user signs one invocation plus the payment |
| `contracts/defindex-strategy/tests/rv_adapter_lifecycle.rs` | 4 | Partial exits leave at most one base unit, stale full amounts and donation front-runs stay reachable, a dust account is recovered by deposit then close |
| `contracts/defindex-strategy/tests/rv_adapter_halts.rs` | 9 | Global pause, frozen, paused, deprecated spoke and supply cap each block or admit exactly what INV-HALT-02 says; a fee-on-transfer asset is haircut twice with nothing stranded; constructor misconfiguration moves no funds; a foreign-asset donation is refused |
| `tests/test-harness/tests/rv_gov_identity.rs` | 6 | Replay after execution is refused, the operation hash binds function, args and predecessor, self and external paths are not interchangeable, expiry residue blocks only the same salt, the recovery operation is isolated, self-operations re-validate at execution |
| `tests/test-harness/tests/rv_gov_proposer_powers.rs` | 5 | Characterization of what a non-owner proposer can execute after the delay (see below); these assert the documented outcome, not a safe expectation |

### What held

**Router.** For every token, the router's real balance change over one
`execute_strategy` equals the vault's closing balance plus the fees it booked:
every vault credit is a measured balance increase, every non-fee debit is a
measured and enforced-equal decrease, and the payout and the residual sweep
both read the vault, never a declared amount. That identity is why no venue
can reach the fee reserve: the router grants exactly one invoker entry per
pull, for the offered amount, and the host drops the entry when the callee
returns. Four greedy venues (an Aquarius hop, an LP mint listing the reserve
token as an unfunded constituent, an LP burn naming the reserve token as its
share token, and a Comet pool that re-spends its allowance) each got exactly
their authorized pull and nothing more, with only the sender's own tree
mocked. A hop pool that replays the sender's input transfer, into the router
or into itself, is refused and the strategy rolls back. The decoder's caps,
index checks and chain rules hold at every boundary, and the referral id is a
byte the sender controls, so every fee is opt-in by construction. Driving the
production router WASM through the controller's multiply confirmed the
cross-contract contract on both sides: the account is credited exactly the
pool payout, the controller and the router keep nothing, and the user's
signature covers one invocation.

**Adapter.** Each vault owns one controller account whose id is a sequential
position-NFT id that is never reused, and the adapter authorizes only its own
direct supply and withdraw calls on that id. No path reaches another vault's
account; third parties can only top up the asset the account already holds.
Partial exits burn ceil shares and pay exactly the amount, a terminal close
pays floor, so a vault loses at most one base unit over any sequence. Every
halt state behaves as INV-HALT-02 states: the global pause and the frozen
flag block deposits and keep exits open, a deprecated spoke keeps exits open,
a paused listing blocks exits until the owner relaxes it.

**Governance.** The operation id binds target, function, arguments and
predecessor; a tampered tuple is simply unscheduled. Execution removes the
entry, so the same tuple cannot run twice without a fresh proposal and a
fresh delay. A self-targeted operation cannot be driven through the generic
path nor a controller operation through the self path. The recovery operation
has no proposer-reachable twin and the owner-canceller cannot cancel it. A
self-operation is re-validated against current state when it executes, so a
delay update that was valid when proposed is refused after a larger one
lands.

### Router observations

**R-A. Verify before routing LP mints on mainnet: the mint authority is
exact-amount.** `add_liquidity` offers the vault's whole balance of each
constituent and grants the pool one transfer entry for exactly that amount.
Under enforcing auth, a pool that pulls one unit less than offered is refused
(the pin in `rv_router_auth`). The in-repo Aquarius LP mock pulls the desired
amounts and refunds the excess, which matches that grant; the live Aquarius
standard pool could not be checked from this environment. If the live pool
pulls the proportional amounts it computes rather than the desired ones, every
mint whose offer is not already exactly proportional to the reserves fails
closed. No funds are at risk either way; the strategy reverts. The adapter
would then need to offer the pool's own computed amounts, read from its
reserves in the same call, instead of the full vault balances.

**R-B. Low, hardening: the same-token check compares registry indices.** A
registry that lists the input token twice can declare the input as the output.
The router then pays the unrouted input back to the sender as the output and
books the hop's real output as admin residual. The payout is bounded by the
vault ledger and the controller rejects the case with `NoSwapOutput`, so it
costs the sender their own hop output and nothing else. Comparing addresses in
`decode` closes it.

**R-C. Informational.** The Soroswap adapter assumes a 30 basis-point pair fee
and the address ordering of the pair's tokens; a pair on another tier
under-requests output or fails closed. The fee policy charges the whitelisted
side's final balance, so a sender-owned tail hop can shrink the base; since
the sender can omit the referral byte altogether, this is not an avoidance
path. Neither adapter has been exercised against a live venue.

### Adapter observations

- A paused listing traps a vault's exit until the owner relaxes the flag.
  Documented in INV-HALT-02, reversible, and the balance keeps reporting.
- The constructor does not validate the spoke id; a wrong one is caught by the
  first deposit before any transfer. Low hardening: probe the spoke at
  construction as the hub market is probed.
- A fee-on-transfer asset is haircut twice (vault to adapter, adapter to pool)
  and credited at the pool-measured amount. Nothing strands on the adapter.
- A third-party donation into the vault account inflates its reported balance.
  Already documented; the NAV defence belongs to the DeFindex vault.

### Governance: the proposer key is a liquidation-terms key

Nothing here escapes a gate. The threat model already says a stolen non-owner
proposer key "can schedule listing, cap, curve and limit changes" and calls
them disruptive. The characterization suite puts numbers on what that means,
because the word understates it. Every row is executable by anyone once the
delay elapses, and the repository's network configuration sets the delay to
12 ledgers on testnet and mainnet (about one minute), which is also the
sensitive-tier floor during the audit period.

| Operation a non-owner proposer can schedule | Delay tier | Outcome pinned |
|---|---|---|
| `EditAssetInSpoke` to threshold 3200 bps with the largest bonus the bounds admit (21250 bps), then the permissionless restamp | Standard | An account at health factor 2.67 restamps to 1.07 (the gate passes); an 8% price dip makes it liquidatable and, on the threshold-times-bonus boundary, one liquidation repays the whole debt and seizes 9.9998 of 10 ETH |
| `ForceSocializeBadDebt` on a merely insolvent account | Sensitive | The whole debt is written against the debt market's suppliers and all collateral is booked as protocol revenue; a permissionless liquidation would have left a quarter of that loss |
| `RemoveSpoke` | Standard | Irreversible; indebted accounts can no longer add collateral, no account can open in the spoke; exits and repayment stay open |
| `SetPositionManager(manager, true)` | Sensitive | Every grant stored under a manager the owner had deactivated is live again; a compromised manager drains the accounts that granted it |
| `UpgradeLiquidityPoolParams` | Standard | A 200% borrow rate at every utilisation with a 99.99% reserve factor; one day adds 32 USDC to a 6000 USDC debt |

The first row is the one to act on. `validate_risk_bounds` admits any
threshold above the loan-to-value and any bonus up to the solvency bound, the
restamp gate only requires 1.05 after the new threshold, and the
health-factor-preserving bonus cap turns every liquidation below 1 into a
full close on that boundary. Three controls, in increasing order of change:
set the configured minimum delay to the seven-day target before funding, so a
canceller can act; move threshold decreases, curve edits, forced
socialization and manager activation into the owner-only proposal set; or
bound a single threshold cut (for example 500 bps per operation) so a ratchet
takes several delays. The threat model's sentence on proposer powers should
also name forced socialization, spoke removal and manager re-activation.

## Fourth pass: second opinion on the Specula findings

The protocol team received a TLA+-driven audit (Specula, run against commit
a2486b25, which is the base of this branch; no contract source changed since).
It reported 28 reproduced findings, 3 masked and 2 unfinished across the price
oracle, governance and the lending core. Every one of them was re-derived here
against HEAD by an independent reviewer, and every confirmation rated medium or
above was handed to a second agent whose only job was to refute it or break the
proposed fix. The reviewers wrote 22 harness binaries (`rv_specula_g1` through
`rv_specula_o8`, 61 tests) that drive each mechanism through production entry
points; all pass under an independent re-run.

Outcome: 33 of 35 items are confirmed, most with corrections to location,
precondition or reach; 2 are documented design. None was refuted. None of the
7 challenges overturned a confirmation. The headline disagreement is severity,
in both directions: the auditors' three High governance items and the High
oracle item are medium or low here, and the item they dropped as already known
is the one we rate high.

### Verdict table

Rating is the auditors' DeFi severity; ours weighs reachability under the
mainnet configuration in this repository and who loses.

| Target | ID | Auditors | Ours | Verdict and the correction that matters |
|---|---|---|---|---|
| Oracle | CR-1 | High | low | Confirmed. A Scaled leg with a Fundamental factor is labelled Fundamental, so the 3,600 s Market+Market spread bound is skipped. Needs a second config deviation to bite (a multi-hour market budget in the quote's subtree); no mainnet oracle has a Market partner for a Scaled leg; the blend's deviation check still bounds the drift to half the tolerance |
| Oracle | CR-2 | Low | low | Confirmed. Same root; the misfire direction trips when a Fundamental sibling two levels down ages the Scaled leg's stamp. Not reachable with the mainnet shapes |
| Oracle | MC-1 | Medium | medium | Confirmed. A ready `ConfigureAssetOracle` stores the whole oracle, band included, so a permissionless execute undoes an ORACLE narrowing for the entire grace window. Mirror of the flags epoch; the attacker only chooses timing of an owner-made proposal |
| Oracle | CR-6 | Medium | low | Confirmed. `revalidate_dependents` reads every registered oracle; the cap is the per-transaction footprint limit (400 entries in the SDK snapshot), reached near 380 to 390 keys. Mainnet has about 32 keys |
| Oracle | CR-7 | Low | low | Confirmed. The band-cap exemption tests trust-set inequality, not disjointness; a shared provider can move the price only within the factor bounds times the tolerance. Option A would break the mainnet SolvBTC band at the next revalidation |
| Oracle | CR-10 | Low | low | Confirmed. `attest` admits a TWAP whose oldest sample is always past the budget; the safe bound is (records + 2) times the resolution |
| Oracle | CR-11 | Low | low | Confirmed. Narrower than "never produces a price": the dual is valid whenever the partner lags enough; at the mainnet 300 s resolution the window can at most equal the bound |
| Oracle | CR-13 | Low | low | Confirmed. `solve_stable_d` traps at line 52 for a one-stroop leg against a leg at the reserve guard; the final multiplication never traps. Checked arithmetic closes it |
| Oracle | CR-16 | Low | low | Confirmed and wider: any unusable price of any supplied token blocks the full-tuple refresh of a debt-free account; the one-line guard closes it |
| Oracle | MC-2 | masked | informational | Depth half confirmed, cycle half refuted (that error is position-independent). Only `quotes` can observe it |
| Governance | MC-1 | High | medium | Confirmed, corrected: the flags are cleared on the add arm of the listing upsert, which skips the ratchet; removal keeps the epoch. The proposed fix does not close the class (a pre-queued listing of the same asset in another spoke or hub is unflagged too); an asset-level freeze epoch checked at execution does |
| Governance | MC-2 | High | medium | Confirmed and wider: every owner-only operation the previous owner queued survives the handover, upgrades included. The guard belongs in `prepare_execute`, which serves all three execute paths. The new owner holds the canceller role and can cancel leftovers it can see |
| Governance | MC-3 | High | medium | Confirmed, corrected: not "known in #90"; that merged PR documents owner-only, non-cancellable and slow, not survival across a handover. The chain of stale resets is finite (about 37 days after the handover). Storing an address in the recovery marker breaks live entries; reuse the owner-epoch sidecar instead |
| Governance | MC-4 | Medium | medium | Confirmed for `Unpause`: a reopen proposed while open stays ready and lands the moment a guardian pauses, bundleable with the exploit. The band arm is oracle MC-1 again; the tolerance arm does not widen. A freeze runbook already tells operators to cancel pending reopens; the invariants assume the delay is served after the pause |
| Governance | MC-5 | Medium | medium | Confirmed. Nothing orders ready operations; in an incident an `Unpause` proposed with a controller fix runs first and reopens the old code, and at the 7-day target tiers it is ready five days before the fix. Restoring the predecessor needs the Done marker kept and the self-execute path to carry it |
| Governance | MC-6 | Medium | low | Confirmed, exact count depends on the owner's address kind and the live event-size limit, which could not be read here. Needs a malicious previous owner. A cap on growth closes it |
| Governance | CR-1 | Medium | low | Confirmed. One-shot, bounded to the grace window, only against the same nominee. An idempotent zero-deadline cancel closes it without an interface change |
| Governance | CR-6 | Low | informational | Confirmed; the ordinary-op variant is already pinned as accepted, the stuck recovery salt has no consequence |
| Governance | CR-4 | masked | informational | Confirmed doc drift: four places cite the older OpenZeppelin revision |
| Governance | CR-8 | dropped | high | Documented design, and the enabling condition for everything above: both tiers are 12 ledgers on mainnet and testnet, so a proposal is executable by anyone about one minute later. Raising the minimum delay is a Standard-tier operation that waits 12 ledgers |
| Lending | MC-1 | Medium | low | Confirmed. A fully repaid coarse debt leg is credited at its ceiling-rounded amount; the crossing needs an account within one debt unit of insolvency, under a cent on the mainnet listings. Credit the cleared WAD value instead |
| Lending | MC-2 | Medium | low | Confirmed, corrected: at most half a unit per leg stays in the pool as ownerless cash; nothing is burned and the socialization is not caused by it. Use the burned-shares result of `resolve_withdrawal`; the one-unit-lower variant is worse than the defect |
| Lending | MC-3 | Medium | low | Confirmed exactly. The `seize_all` tolerance is one unit of every kept repayment leg, so an untrimmed under-offer within that sum takes all collateral. Bounded by the unit value of borrowable listings (under one cent on mainnet, dollars only with a coarse high-priced listing). Set it only when the pre-trim offer reached the quote |
| Lending | MC-4 | Medium | low | Confirmed, corrected: not "no path"; the liquidator can supply the cash-less asset first and seize in Transfer mode. The Credit(0) limit fix is still right |
| Lending | MC-5 | Medium | low | Confirmed under a precondition the table omits: the open legs' debt must back less than one whole collateral unit. The proposed rounding fix does not close its own reproduction; document the exception |
| Lending | CR-21 | Medium | medium | Confirmed, corrected site: the panic is in the accrual kernel reached from `get_bulk_indexes`, so every valuation of an account holding the overflowed market reverts (views, withdraw, borrow, liquidation, cleanup); repay and supply still work. Recovery is upgrade-only |
| Lending | CR-20 | dropped | low | Documented arithmetic limit; the docs understate its reach, not its existence |
| Lending | CR-1 | Low | low | Confirmed; the reopened reported shortfall is at most one unit. Two-line RAY-precision fix |
| Lending | CR-2 | Low | low | Confirmed; the invariant text, not the code, is what it contradicts. One guard in `mint_debt` closes it |
| Lending | CR-10 | Low | informational | Confirmed; only listings whose threshold-to-LTV ratio is under 1.05 can end a withdraw there, and it needs a pending liquidator-favouring edit |
| Lending | CR-11 | Low | informational | Confirmed in the protocol-conservative direction only |
| Lending | CR-19 | Low | informational | Confirmed: the batch event, not the flash-position event, reports the entry owner. One-line re-read |
| Lending | CR-7 | masked | informational | Confirmed and stronger: with collateral equal to debt the cap is at most zero, so both Certora rules are vacuous. Repair them as three rules, one per arm |
| Lending | CR-24 | unfinished | informational | Confirmed test-fidelity gap: seeded cash without shares hides the backing gates in default fixtures |
| Lending | CR-25 | unfinished | low | Confirmed: the Credit-mode estimate skips every receiver gate |

### What the review changes about priorities

**Before mainnet growth.** Raise the configured minimum delay. Every
governance item above is an ordering or binding gap whose documented control is
a canceller veto, and the veto has no window at 12 ledgers. Until the epoch
fixes land, the freeze runbook's rule should be the general one: after any
emergency action or ownership handover, cancel every pending operation that
could undo it.

**Next release, one mechanism for the governance family.** A single owner
epoch stamped on owner-only proposals (the recovery reset included) and checked
in `prepare_execute` closes MC-2 and MC-3 without a storage-type change. A
pause epoch carried by `Unpause` closes MC-4; a per-key band epoch carried by
`ConfigureAssetOracle` closes oracle MC-1; an asset-level freeze epoch checked
by `AddAssetToSpoke` closes MC-1 for every spoke and hub. MC-5 needs the
predecessor restored and the Done marker kept, which flips two pins in
`rv_gov_identity` and the same-salt re-proposal rule, so it is the one change
to design rather than patch. The lending items are small, local and
independent: the pre-trim `seize_all` condition, crediting the cleared debt,
using the burned-shares result, the `mint_debt` guard, the RAY-precision
shortfall, the debt-free skip in `update_account_threshold`, the Credit(0)
limit, the estimate's receiver gates, and checked arithmetic in the stable-LP
pricer. CR-21 is the exception: it needs the accrual kernel to stop trapping,
and the proposed "skip frozen legs" variant must be rejected because it
misstates health.

**Documentation.** Four OpenZeppelin revision citations, the ADR-0008 exception
for whole-unit collateral, the INV-ACCT-09 scope wording, the proposer-power
sentence in the threat model, and the two vacuous Certora rules.

### Follow-up answers from the protocol team's questions

**Does the seven-day timelock fix governance?** It restores the control the
design relies on, not the role boundary. With the minimum raised to the
120,960-ledger target, Standard and Sensitive both wait seven days and Recovery
thirty, the update is one-way, and a canceller or a user has a week to act on
anything a proposer schedules. What stays: a non-owner proposer still holds
liquidation-terms authority, and `revoke_role_immediate` covers only the
guardian and oracle roles, so a stolen proposer key is removed only after a
Sensitive delay while each of its proposals is cancelled one by one. MC-5 also
gets worse at the target tiers, because a reopen proposed with an incident fix
is ready five days before the fix. The fix set is the delay, an immediate
revocation path for the proposer and executor roles, and either the owner-only
set widened to threshold cuts, curve edits, forced socialization and manager
activation, or a per-operation bound on threshold decreases.

**Router dust and the fee system.** The balance the team sees on the mainnet
router is the admin residual bucket, not a fee: every `execute_strategy`
accrues each token left in its vault, up to the larger of 1,000 base units or
one millionth of what it credited, into `AdminFee` and `ReservedTotal`, and
anything above that reverts. LP mint routes leave exactly this kind of
remainder when the pool takes less of a constituent than it was offered, and
split routes leave it from parts-per-million rounding. It is claimable with
`claim_admin_fees`. No fee was ever charged because fees require a nonzero
active referral id in the payload byte the sender controls; a referral-less
route pays nothing even when a static fee is set. The ledger that wrote each
bucket is the entry's last-modified ledger, readable with the storage key
`AdminFee(token)`; the on-chain check could not run from this environment
because its network policy denies the Stellar hosts. That LP mints settled on
mainnet also answers R-A above: the live pool's pull matches the router's
exact-amount grant.

## Scope and limits

- The harness registers the controller natively and uses mocked
  authorization, so signature trees are not exercised; the controller's own
  owner, delegate and NFT checks are. The existing rogue-hop tests under `tests/test-harness/tests/strategy/` cover
  enforced authorization trees.
- The XOXNO oracle contract and the position NFT are outside these passes.
  The Specula review verified mechanisms, not the auditors' own reproduction
  files, which were not available here; four on-chain facts (registry size,
  SolvBTC bounds, the TWAP setting, the event-size limit) remain unchecked.
  The router was exercised against in-crate venue doubles and, behind the
  controller, as its production WASM; no live venue was reached.
- The test environment restores expired entries, so multi-year jumps do not
  model archival.
- Certora, libFuzzer, mutants and the full workspace suite were not re-run as
  part of this pass; the eight suites, `make fmt-check`, `make docs-check`,
  `make access-control-check` and `cargo clippy -D warnings` on the eight
  binaries were.
