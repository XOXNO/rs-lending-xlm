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

## Scope and limits

- The harness registers the controller natively and uses mocked
  authorization, so signature trees are not exercised; the controller's own
  owner, delegate and NFT checks are. The existing rogue-hop tests under `tests/test-harness/tests/strategy/` cover
  enforced authorization trees.
- Governance, the swap aggregator, the XOXNO oracle contract and the DeFindex
  adapter are outside this pass.
- The test environment restores expired entries, so multi-year jumps do not
  model archival.
- Certora, libFuzzer, mutants and the full workspace suite were not re-run as
  part of this pass; the eight suites, `make fmt-check`, `make docs-check`,
  `make access-control-check` and `cargo clippy -D warnings` on the eight
  binaries were.
