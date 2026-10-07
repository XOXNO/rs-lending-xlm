---
name: xoxno-lending-liquidations
description: "Use when building a liquidation bot, keeper, or risk monitor for XOXNO Lending on Stellar, or when a task mentions liquidate, is_liquidatable, get_liquidation_estimate, SeizeMode, liquidation bonus, health factor below 1, no_seize, bad debt, or liquidating from a Soroban contract."
user-invocable: true
argument-hint: "[liquidation task]"
---

# XOXNO Lending liquidation runbook

`liquidate(liquidator, account_id, debt_payments, seize_mode)` sizes the close,
pulls accepted debt tokens, seizes collateral pro rata, and pays underlying
tokens (`Transfer`) or supply credit (`Credit`). Simulate
`get_liquidation_estimate` with the exact same payments and mode immediately
before building the write.

Canonical references:

- Contract signatures: [contracts ABI](../xoxno-lending-contracts/abi.md)
- Transaction lifecycle: [SDK transactions](../xoxno-lending-sdk/transactions.md)
- Liquidation arithmetic: [math](../xoxno-lending/math.md#liquidation-bonus-close-amount-seizure-fees)
- Error definitions: [error reference](../../docs/reference/errors.md)
- Event shapes and order: [event reference](../../docs/reference/events.md)
- Operational failure diagnosis: [troubleshooting](../xoxno-lending-troubleshooting/SKILL.md)

## 1. Discover candidates

Do not scan account ids. Build a watchlist from successful controller
`position:batch_update` events. Use archived successful events to recover an
older inventory. For known owners, the v1 wallet positions route can seed IDs.
Follow every pagination link. Recheck candidates on a timer and on pool index
updates. Prices can change without a controller event.

- Ignore every event with `inSuccessfulContractCall === false`; reverted calls
  are not candidates or state changes.
- Persist the `getEvents` cursor after each durably committed page and resume
  from that cursor. Keep `(txHash, eventId, childIndex)` idempotency keys.
- Indexed API data supplies candidates. Current contract views decide eligibility.

For each candidate:

1. `account_exists(account_id)` must be true.
2. `is_liquidatable(account_id)` must be true, equivalently on-chain health
   factor below `1e18`.
3. Treat `get_health_factor == i128::MAX` as ineligible. It can mean a
   missing account, no debt, or a saturated positive ratio. It does not prove
   that debt is absent.

`HealthFactorTooHigh` (`controller #101`) is a legitimate liquidation result
when eligibility changes or no target debt remains. Identify the panicking
contract before mapping the number.

## 2. Select the seize mode

| Mode | Result | Estimate units | Main constraint |
|---|---|---|---|
| `Transfer` | Underlying collateral to the liquidator | Asset base units | Every generated Transfer seizure leg needs sufficient pool cash and the recipient may need a classic-asset trustline |
| `Credit(0)` | New normal-mode receiving account | RAY supply shares | Position limit; no pool-cash requirement |
| `Credit(id)` | Supply shares credited to `id` | RAY supply shares | Receiver is authorized, same spoke, normal mode, and not the liquidated account |

Seizure is pro rata, but the planner drops legs that round to zero.
`no_seize` blocks only a generated nonzero seizure leg. A dropped leg does
not block liquidation. Debt repayment rejects a paused listing but permits
a frozen listing. Seizure permits paused or frozen listings, but rejects
`no_seize` on a generated leg.

A missing spoke listing permits repayment and Transfer seizure. Credit needs
a listing when it creates a new receiver supply position. An existing receiver
position uses its stored risk parameters.

The estimate does not prove receiver authorization, position capacity,
low-decimal isolation, or pool cash availability. For `Credit(id)`, require current NFT ownership or an active registered
manager delegated by that owner. Require the same spoke, `Normal` mode,
and a receiver distinct from the target. Simulate the complete
write for every mode.

Use Credit when Transfer cannot draw pool cash. Convert Credit estimate shares
to token units with the matching market supply index and asset decimals before
valuation:

```text
value_ray   = floor(shares * supply_index / RAY)
token_units = floor(value_ray / 10^(27 - asset_decimals))
```

## 3. Build debt payments safely

Offer only positive debt legs the account actually owes. The controller pulls accepted amounts. For a full-debt quote, it pulls the
whole offer and the pool refunds excess. A direct repayment transfer to the
pool is a donation. Refunds are in debt-token units.

For a contract liquidator, require each payment leg to have a unique token
address even when the same token is borrowed in multiple hubs. Estimates and
refunds are keyed by token address only, not by `(hub_id, asset)`.

This is an authorization limit. `authorize_as_current_contract` authorizes the
nested token call by token contract, function, and transfer arguments. It does
not include the controller's hub id. With repeated token addresses, asset-only
refunds cannot determine the accepted amount for each hub leg, while
authorization entries are consumed against ordered token-transfer
sub-invocations. The contract cannot safely construct exact per-leg transfer
authorizations.

### Full-close and insolvent plans

When `D ≤ C < D × (1 + base bonus)`, the quote covers the debt at a capped bonus.
A full-close quote can accept a partial offer. Rounding can change the account
ratio. While the final plan remains a full-close plan, the controller pulls
each merged offer. The pool refunds excess debt tokens. Hold the full offer.
Accrual can change the plan. Re-simulate before signing.

When `C < D`, the quote is `floor(C / (1 + base))` at the base bonus.
The planner trims a larger offer from the last leg backward.
It floors retained token amounts and drops zero amounts.
A plan without a payment leg fails with `InvalidPayments`.
Authorize the final accepted transfers. A stale authorization can fail if a
competing liquidation changes the plan.

**Collateral below 3 decimals seizes base units.** Such a leg is the account's
only supply position. One unit means `10^-decimals` tokens, not one whole token.
At 2 decimals, one base unit is 0.01 token. On a solvent account whose leg holds at least one base
unit, the controller can change a partial quote:

- When one unit's USD value divided by `1 + bonus` covers the whole debt plus
  one base unit of each debt leg, the quote is the whole debt. The liquidator
  repays all debt and receives one unit.
- Otherwise, check whether the curve quote seizes less than one base unit
  plus a `1e-6` margin. The quote rises to the repayment for that amount only
  if the raised repayment stays below the whole debt. The seizure takes one
  unit and refunds the margin, rounded down to debt-token base units.
- Otherwise the curve quote stays.

The raised quote is a ceiling. A smaller offer is not raised. An offer that
backs less than one unit seizes nothing and reverts with `InvalidPayments`
(`controller #16`). A raised quote is not promoted to full debt, so it can
leave debt below $5. If neither rule changes the quote, check whether it backs one base unit.
If it does not, every offer fails until accrual or a price change ends the condition.

In the full-close case the liquidator pays the debt `D` and receives one unit
worth `U`. Its effective bonus is `U / D - 1`, not the reported
`bonus_rate_bps`. With `k` held units and liquidation threshold `LT`, it is at
most about `1 / (k * LT) - 1`. The borrower loses `U - D * (1 + bonus)` above
the normal bonus. The listing has no liquidation fee. Value the seized unit at
its price, not at `D * (1 + bonus)`.

Do not size such an offer from the curve formula. Estimate an offer up to the
debt with `get_liquidation_estimate`, read `max_payment_wad` and `refunds`, and
sign only the accepted amounts. See
[math](../xoxno-lending/math.md#seizure-and-fees-per-collateral).

Perform every controller/token read first. Offer exactly the accepted amounts. Authorize each planned
`transfer(liquidator, pool, amount)`. Call `liquidate` immediately.
Do not make another outbound contract call between authorization and liquidation. See [contract composition](../xoxno-lending-contracts/composing.md#contract-liquidation).

## 4. Gate on actual net proceeds

Never reconstruct gross collateral as
`repayment × (1 + bonus)` or cap that reconstruction by account collateral.
The estimate contains planned legs. It does not prove the final payout.

Snapshot the account's canonical ordered supply-position list before
estimating. `seized_collaterals` and `protocol_fees` are index-aligned with
each other: require equal lengths between those two vectors and the same asset
at each index.

They are **not** index-aligned with the supply snapshot. The contract drops a
leg whose seizure rounds to zero tokens or zero shares (see
[math](../xoxno-lending/math.md#seizure-and-fees-per-collateral)), so the estimate is an ordered
**subsequence** of the supply positions. A borrower can hold a one-unit leg, and
every partial liquidation of that account then returns fewer legs than
positions. Do not reject an account because an estimate omits a supply leg.
Requiring one estimate leg per position can stop the bot on valid dust positions.

Estimate tuples omit hub IDs. Match assets to supply positions in stored
order. A unique ordered match identifies the hub. If dropped legs permit
several matches, keep the hub ambiguous. Use the lowest value across
compatible matches. Stop if you cannot bound the value.

Reject an estimate whose `seized_collaterals` is empty. The contract accepts a
liquidation that repays debt and seizes nothing, so the liquidator would pay
and receive no collateral.

Repeated collateral token addresses across different hubs are legitimate:
preserve the hub from each ordered supply-position entry and do not deduplicate
those estimate legs. If a repeated token has fewer estimate legs than supply positions, the
estimate alone cannot identify which hub lost a leg. Use the lower value
across compatible hub assignments.

Repeated debt-payment token addresses have a separate problem for contract
liquidators. Refunds and nested transfer authorization cannot identify the
hub-specific repayment legs.

For every paired collateral `(hub_id, asset)`:

1. Reject a non-positive seizure leg that is present in the estimate, and a
   leg that matches no supply position. An absent leg is not an error.
2. Require `0 <= protocol_fee <= seized_amount`; reject an omitted fee leg
   instead of silently treating it as zero. In Credit mode a dust leg can carry
   a fee equal to its seizure; value it at zero instead of rejecting the
   estimate. In Transfer mode the fee never exceeds the whole units the pool
   pays above the leg's repayment share.
3. In Transfer mode, clamp gross tokens to the floor of the held token claim.
   Compute `net_amount = min(seized_amount, floorHeldTokenClaim) - protocol_fee`.
   A half-up full-close estimate can show 2 base units for a 1.6-unit claim,
   while the pool pays only 1. Reject a negative conservative net estimate.
4. In Credit mode, compute `net_amount = seized_amount - protocol_fee`. These
   amounts are shares. Subtract fee shares first. Convert the net shares once
   with the matching market supply index and token decimals. Do not convert gross and fee independently.
5. Price each net amount with the corresponding contract-address `PriceKey`
   and decimals, rejecting stale/unsafe prices.
6. Sum the USD value of conservative net legs, then subtract accepted debt cost,
   Soroban resource/inclusion fees, trustline/reserve costs, and conservative
   exit-swap slippage.

Execute only when the resulting net profit exceeds the configured margin.
Re-simulate immediately before signature because prices, indexes, and received
token amounts can change.

## 5. Submit one envelope through a terminal state

Persist the original network, signed XDR, and hash before sending.

- For `PENDING` or `DUPLICATE`, poll the original hash.
- For `TRY_AGAIN_LATER`, check the hash and retry the identical signed XDR.
- For `NOT_FOUND`, keep checking the original hash. Retry only the original
  envelope while its timebounds remain valid.
- Use ledger close time to check expiry. Expiry prevents future inclusion;
  it does not prove that the transaction was never included.
- After expiry, resolve the original outcome from retained or complete
  transaction history. Reconcile the resulting state. If the outcome remains
  unknown, stop replacement submission.
- After a resolved failure or proven non-inclusion, refresh sequence,
  re-estimate, re-simulate, and sign the replacement.

Archived footprint entries are restored inline by protocol 23 when an invoke
is submitted after simulation. Follow the single canonical rule in
[troubleshooting](../xoxno-lending-troubleshooting/SKILL.md#archived-entries).

## 6. Confirm and reconcile

Confirmation is the original transaction hash reaching `SUCCESS`, not a send
status. Decode its return (`0` for Transfer, receiving account id for Credit)
and reconcile:

- controller liquidation and target position batch;
- optional Credit receiver batch and NFT mint;
- optional bad-debt cleanup, pool market snapshots, and NFT burn;
- token transfers for Transfer mode;
- resulting account and market views.

Events are operational evidence, not complete state truth. Missing zero-delta
batches and interleaved pool/NFT/token events are expected; compare events with
post-transaction views. Use the canonical ordering and payload definitions in
[events](../../docs/reference/events.md).

## Bad debt

`clean_bad_debt` is permissionless. It succeeds only when debt exceeds
collateral and collateral is at most `BAD_DEBT_USD_THRESHOLD`, a fixed 5 USD
in WAD. `CannotCleanBadDebt` (`controller #114`) is a legitimate result when
those conditions do not hold; it is not inherently a collision or a non-user
error. Post-liquidation cleanup applies the same gate but skips instead of
reverting. The owner-only `force_socialize_bad_debt` has no collateral cap.
Always identify the panicking contract before interpreting #114.

The controller's flash loan cannot fund its own liquidation call.
The Soroban host rejects re-entry. `liquidate` also rejects an active
flash loan with `FlashLoanOngoing`. Use inventory or external liquidity.

## Completion checks

- [ ] Candidate came from successful events or an explicit API seed.
- [ ] Cursor and idempotency keys were persisted durably.
- [ ] Exact payments/mode were estimated and the final write was simulated.
- [ ] Net profit used conservative net proceeds per asset,
      including gas, trustline/reserve cost, and exit slippage.
- [ ] Repeated debt-payment token addresses were rejected for a contract
      liquidator.
- [ ] The original signed envelope/hash was persisted and confirmed.
- [ ] Events were reconciled with resulting account and market state.
