---
name: xoxno-lending-liquidations
description: "Use when building a liquidation bot, keeper, or risk monitor for XOXNO Lending on Stellar, or when a task mentions liquidate, is_liquidatable, get_liquidation_estimate, SeizeMode, liquidation bonus, health factor below 1, no_seize, bad debt, or liquidating from a Soroban contract."
user-invocable: true
argument-hint: "[liquidation task]"
---

# XOXNO Lending liquidation runbook

`liquidate(liquidator, account_id, debt_payments, seize_mode)` sizes the close,
pulls accepted debt tokens, seizes collateral pro rata, and pays underlying
tokens (`Transfer`) or supply credit (`Credit`). Immediately before building
the write, simulate `get_liquidation_estimate` with the same payments and mode.

References: [contracts ABI](../xoxno-lending-contracts/abi.md),
[SDK transactions](../xoxno-lending-sdk/transactions.md),
[liquidation math](../xoxno-lending/math.md#liquidation-bonus-close-amount-seizure-fees),
[errors](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/errors.md),
[events](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/events.md),
[troubleshooting](../xoxno-lending-troubleshooting/SKILL.md).

## 1. Discover candidates

Do not scan account ids. Watch successful controller `position:batch_update`
events (archived ones for older inventory); the v1 wallet positions route can
seed IDs for known owners (follow every pagination link). Recheck on a timer
and on pool index updates: prices move without controller events.

- Ignore events with `inSuccessfulContractCall === false`.
- Persist the `getEvents` cursor after each durably committed page; resume
  from it. Keep `(txHash, eventId, childIndex)` idempotency keys.
- API data supplies candidates; current contract views decide eligibility.

For each candidate:

1. `account_exists(account_id)` is true.
2. `is_liquidatable(account_id)` is true (on-chain health factor below `1e18`).
3. `get_health_factor == i128::MAX` is ineligible (no debt or saturated
   ratio); it does not prove debt is absent. A missing account fails both
   views with `AccountNotFound` (`controller #24`).

`HealthFactorTooHigh` (`controller #101`) is legitimate when eligibility
changes or no target debt remains; identify the panicking contract first.

`liquidate` prices every leg, so one failing price reverts it. If that asset
is a dust debt leg, repay it in full first (`repay` is permissionless and
reads no price; a paused listing rejects it), then liquidate.

## 2. Select the seize mode

| Mode | Result | Estimate units | Main constraint |
|---|---|---|---|
| `Transfer` | Underlying collateral to liquidator | Asset base units | Pool cash for every generated leg; recipient may need a classic-asset trustline |
| `Credit(0)` | New normal-mode receiving account | RAY supply shares | No governed position limit; no pool-cash requirement |
| `Credit(id)` | Supply shares credited to `id` | RAY supply shares | Receiver authorized, same spoke, normal mode, not the target |

### Listing flags

Seizure is pro rata, but the planner drops legs that round to zero.

| Listing state | Debt repayment | Seizure |
|---|---|---|
| paused | rejected | permitted |
| frozen | permitted | permitted |
| `no_seize` | permitted | rejected on a generated nonzero leg; a dropped leg does not block |
| missing spoke listing | permitted | `Transfer` permitted; `Credit` needs a listing for a new receiver position (existing ones keep stored risk parameters) |

### Credit receivers

The estimate checks a `Credit` receiver's spoke, mode, position capacity and
low-decimal isolation, not authorization or pool cash. `Credit(id)` requires
current NFT ownership or an active manager delegated by the owner. Simulate
the complete write for every mode.

Use Credit when Transfer cannot draw pool cash. A liquidation leaving at most
$5 of collateral below remaining debt socializes that debt in the same call,
and a Credit receiver in a written-down market shares the loss pro rata
(unseen by the estimate). If the account also owes a seized asset, use
Transfer, or a partial size leaving more than $5 of collateral.

Convert Credit estimate shares to tokens before valuation:

```text
value_ray   = floor(shares * supply_index / RAY)
token_units = floor(value_ray / 10^(27 - asset_decimals))
```

## 3. Build debt payments safely

Offer only positive debt legs the account owes. The controller pulls accepted
amounts (whole offer for a full-debt quote, excess refunded in debt tokens).
A direct transfer to the pool is a donation.

### Unique debt-token addresses for contract liquidators

A contract liquidator must use a unique token address per payment leg, even
for one token borrowed in several hubs, because:

- estimates and refunds are keyed by token address, not `(hub_id, asset)`;
- `authorize_as_current_contract` binds the nested token call by token
  contract, function, and transfer arguments, not the controller's hub id;
- authorization entries are consumed against ordered token-transfer
  sub-invocations.

Repeated addresses make exact per-leg authorizations impossible.

### Full-close and insolvent plans

When `D ≤ C < D × (1 + base bonus)`, the quote covers the debt at a capped
bonus and accepts partial offers (rounding can change the ratio). While the
plan stays full-close, the controller pulls each merged offer and the pool
refunds excess: hold the full offer. Accrual can change the plan; re-simulate.

When `C < D`, the quote is `floor(C / (1 + base))` at the base bonus. The
planner trims larger offers from the last leg backward, flooring retained
amounts and dropping zeros; no leg left → `InvalidPayments`. Authorize the
final accepted transfers; a competing liquidation can invalidate them.

**Collateral below 3 decimals seizes whole base units.** The controller can
raise a partial quote or make it a full close; an offer backing less than one
unit reverts with `InvalidPayments` (`controller #16`). Estimate an offer up
to the debt with `get_liquidation_estimate` (not the curve formula), read
`max_payment_wad` and `refunds`, and sign only accepted amounts. Value the
seized unit at its price, not `D * (1 + bonus)`. Rules:
[math](../xoxno-lending/math.md#low-decimal-collateral-quotes).

Do every controller/token read first, offer exactly the accepted amounts,
authorize each planned `transfer(liquidator, pool, amount)`, then call
`liquidate` with no outbound call in between
([contract composition](../xoxno-lending-contracts/composing.md#contract-liquidation)).

## 4. Gate on actual net proceeds

Never reconstruct gross collateral as `repayment × (1 + bonus)` or cap it by
account collateral; the estimate holds planned legs, not the final payout.

Snapshot the ordered supply-position list before estimating.
`seized_collaterals` and `protocol_fees` must have equal lengths and the same
asset at each index. They are **not** aligned with the snapshot: legs
rounding to zero tokens or shares are dropped
([math](../xoxno-lending/math.md#seizure-and-fees-per-collateral)), so the
estimate is an ordered **subsequence**. Do not require one leg per position;
valid dust would stop the bot.

Estimate tuples omit hub IDs. Match assets to positions in stored order; a
unique match identifies the hub. Keep repeated collateral addresses across
hubs (do not deduplicate). If dropped legs allow several matches, use the
lowest compatible value, or stop if unbounded.

Reject empty `seized_collaterals`: the contract accepts a liquidation that
repays debt and seizes nothing.

For every paired collateral `(hub_id, asset)`:

1. Reject a present non-positive seizure leg and a leg matching no position;
   an absent leg is fine.
2. Require `0 <= protocol_fee <= seized_amount`; reject an omitted fee leg.
   A Credit dust leg can carry a fee equal to its seizure: value it at zero.
   A Transfer fee never exceeds the whole units paid above the leg's
   repayment share.
3. Transfer: `net_amount = min(seized_amount, floorHeldTokenClaim) - protocol_fee`
   (a half-up full-close estimate can show 2 base units for a 1.6-unit claim
   the pool pays as 1). Reject a negative net.
4. Credit: `net_amount = seized_amount - protocol_fee` in shares, converted
   once with the formula above; never convert gross and fee separately.
5. Price each net amount with its contract-address `PriceKey` and decimals;
   reject stale/unsafe prices.
6. Subtract accepted debt cost, Soroban resource/inclusion fees,
   trustline/reserve costs, and conservative exit slippage from the USD sum.

Execute only above the configured margin; re-simulate immediately before
signing.

## 5. Submit one envelope through a terminal state

Persist network, signed XDR, and hash before sending; follow the
[submission lifecycle](../xoxno-lending-troubleshooting/SKILL.md#submission-lifecycle)
until the hash is terminal. A replacement also needs a fresh estimate.
Archived entries:
[troubleshooting](../xoxno-lending-troubleshooting/SKILL.md#archived-entries).

## 6. Confirm and reconcile

Confirmation is the original hash reaching `SUCCESS`. Decode the return
(`0` for Transfer, receiving account id for Credit) and reconcile:

- controller liquidation and target position batch;
- optional Credit receiver batch and NFT mint;
- optional bad-debt cleanup, pool market snapshots, and NFT burn;
- token transfers for Transfer mode;
- resulting account and market views.

Events are evidence, not complete state: zero-delta batches can be missing
and pool/NFT/token events interleave. Compare with post-transaction views.
Ordering and payloads:
[events](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/events.md).

## Bad debt

Permissionless `clean_bad_debt` requires debt above collateral and
collateral at most 5 USD WAD ([gate](../xoxno-lending/math.md#bad-debt-socialization));
otherwise `CannotCleanBadDebt` (`controller #114`) is a legitimate result.
Identify the panicking contract first. Post-liquidation cleanup skips instead
of reverting.

A controller flash loan cannot fund a liquidation: the host rejects re-entry,
and `liquidate` rejects an active flash loan with `FlashLoanOngoing`. Use
inventory or external liquidity.

## Completion checks

- [ ] Candidate came from successful events or an explicit API seed.
- [ ] Cursor and idempotency keys were persisted durably.
- [ ] Exact payments/mode were estimated and the final write was simulated.
- [ ] Net profit used conservative per-asset net proceeds, gas,
      trustline/reserve cost, and exit slippage.
- [ ] A contract liquidator used
      [unique debt-token addresses](#unique-debt-token-addresses-for-contract-liquidators).
- [ ] The original signed envelope/hash was persisted and confirmed.
- [ ] Events were reconciled with resulting account and market state.
