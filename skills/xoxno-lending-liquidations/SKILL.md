---
name: xoxno-lending-liquidations
description: Use when building a liquidation bot, keeper, or risk monitor for XOXNO Lending on Stellar, or when a task mentions liquidate, is_liquidatable, get_liquidation_estimate, SeizeMode, liquidation bonus, health factor below 1, no_seize, bad debt, or liquidating from a Soroban contract.
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
`position:batch_update` events, seed it from
`GET /stellar-lending/positions`, and recheck it on a timer plus pool index
updates. Prices can move without a controller event.

- Ignore every event with `inSuccessfulContractCall === false`; reverted calls
  are not candidates or state changes.
- Persist the `getEvents` cursor after each durably committed page and resume
  from that cursor. Keep `(txHash, eventId, childIndex)` idempotency keys.
- The REST leaderboard is edge-cached and uses a different health-factor
  presentation. It seeds candidates; on-chain views decide eligibility.

For each candidate:

1. `account_exists(account_id)` must be true.
2. `is_liquidatable(account_id)` must be true, equivalently on-chain health
   factor below `1e18`.
3. `get_health_factor == i128::MAX` means missing or debt-free for bot
   purposes, not evidence of a safely collateralized debt position.

`HealthFactorTooHigh` (`controller #101`) is a legitimate liquidation result
when eligibility changes or no target debt remains. Identify the panicking
contract before mapping the number.

## 2. Select the seize mode

| Mode | Result | Estimate units | Main constraint |
|---|---|---|---|
| `Transfer` | Underlying collateral to the liquidator | Asset base units | Every collateral market needs sufficient pool cash and the recipient may need a classic-asset trustline |
| `Credit(0)` | New normal-mode receiving account | RAY supply shares | Position limit; no pool-cash requirement |
| `Credit(id)` | Supply shares credited to `id` | RAY supply shares | Receiver is authorized, same spoke, normal mode, and not the liquidated account |

Seizure is pro rata across all collateral. Before estimating, load every
supplied `(hub_id, asset)` through `get_spoke_asset`; one `no_seize` leg makes
the entire liquidation fail. A missing listing is a separate invalid-state
condition, not a halted leg.

Use Credit when Transfer cannot draw pool cash. Convert Credit estimate shares
to token units with the matching market supply index and asset decimals before
valuation:

```text
value_ray   = floor(shares * supply_index / RAY)
token_units = floor(value_ray / 10^(27 - asset_decimals))
```

## 3. Build debt payments safely

Offer only positive debt legs the account actually owes. The controller pulls
accepted amounts; transferring repayment directly to the pool is a donation.
Refunds are informational and remain in debt-token units.

For a contract liquidator, require each payment leg to have a unique token
address even when the same token is borrowed in multiple hubs. Reject repeated
addresses until estimates and refunds are keyed by `(hub_id, asset)`.

This is an authorization limitation, not merely a bookkeeping preference.
`authorize_as_current_contract` authorizes the nested token call by token
contract, function, and transfer arguments. It does not include the
controller's hub id. With repeated token addresses, asset-only refunds cannot
determine the accepted amount for each hub leg, while authorization entries
are consumed against ordered token-transfer sub-invocations. The contract
cannot safely construct exact per-leg transfer authorizations.

Perform every controller/token read first. Then authorize each exact accepted
`transfer(liquidator, pool, amount)` and call `liquidate` immediately; no
outbound contract call may occur between authorization and the controller
call. See [contract composition](../xoxno-lending-contracts/composing.md#liquidating-from-a-contract).

## 4. Gate on actual net proceeds

Never reconstruct gross collateral as
`repayment × (1 + bonus)` or cap that reconstruction by account collateral.
The estimate already contains the executable legs.

Snapshot the account's canonical ordered supply-position list before
estimating. Estimate tuples omit hub ids, so pair `seized_collaterals` and
`protocol_fees` positionally against that ordered snapshot to recover each
leg's `(hub_id, asset)`. Require equal vector lengths and require each tuple's
asset to match its paired supply position.

Repeated collateral token addresses across different hubs are legitimate:
preserve the hub from each ordered supply-position entry and do not deduplicate
those estimate legs. This is distinct from repeated **debt payment** token
addresses for a contract liquidator, which remain unsafe because refunds and
nested transfer authorization cannot be assigned to hub-specific repayment
legs.

For every paired collateral `(hub_id, asset)`:

1. Reject missing, extra, zero, or non-positive seizure legs.
2. Require `0 <= protocol_fee < seized_amount`; reject an omitted fee leg
   instead of silently treating it as zero.
3. Compute `net_amount = seized_amount - protocol_fee`. In Credit mode these
   are shares: subtract fee shares from gross seized shares first, then convert
   the single net share amount once using that hub market's supply index and
   asset decimals. Do not convert gross and fee independently.
4. Price each net amount with the corresponding contract-address `PriceKey`
   and decimals, rejecting stale/unsafe prices.
5. Sum the USD value of actual net legs, then subtract accepted debt cost,
   Soroban resource/inclusion fees, trustline/reserve costs, and conservative
   exit-swap slippage.

Execute only when the resulting net profit exceeds the configured margin.
Re-simulate immediately before signature because prices, indexes, and received
token amounts can change.

## 5. Submit one envelope through a terminal state

Persist the signed original envelope and its hash before sending.

- `PENDING` or `DUPLICATE`: poll that original hash.
- `TRY_AGAIN_LATER`: the RPC did not establish a replacement-safe terminal
  state. Check the original hash, resubmit the identical signed envelope when
  appropriate, and keep polling.
- `getTransaction NOT_FOUND`: it means unknown *now*, not failed. Continue
  checking/resubmitting the original envelope until it confirms or its
  timebounds expire.
- Build a new envelope only after the original is confirmed terminal or its
  timebounds have expired. Refresh sequence, re-estimate, re-simulate, and
  re-sign then.

Do not discard a signed envelope on `TRY_AGAIN_LATER` or a bounded run of
`NOT_FOUND`; rebuilding early can submit two distinct liquidations.

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

`clean_bad_debt` is permissionless only when debt exceeds collateral and
collateral is at most the configured $5 WAD dust gate. `CannotCleanBadDebt`
(`controller #114`) is a legitimate result when those conditions do not hold;
it is not inherently a collision or a non-user error. Automatic post-
liquidation cleanup and governance force-socialization have different entry
conditions. Always identify the panicking contract before interpreting #114.

The controller's own flash loan cannot fund a liquidation on that controller:
the flash guard rejects guarded reentry. Use inventory or external liquidity.

## Completion checks

- [ ] Candidate came from successful events or an explicit API seed.
- [ ] Cursor and idempotency keys were persisted durably.
- [ ] Exact payments/mode were estimated and the final write was simulated.
- [ ] Net profit used actual `seized_collaterals - protocol_fees` per asset,
      including gas, trustline/reserve cost, and exit slippage.
- [ ] Repeated token addresses were rejected for contract-liquidator legs.
- [ ] The original signed envelope/hash was persisted and confirmed.
- [ ] Events were reconciled with resulting account and market state.
