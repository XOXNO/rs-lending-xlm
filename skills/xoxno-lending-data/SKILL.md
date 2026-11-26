---
name: xoxno-lending-data
description: "Use when consuming XOXNO Lending data off-chain: indexing controller, pool, position-NFT, token, or oracle events; tracking account lifecycle; deriving liquidation accounting; building analytics; or calling the public Stellar lending REST API."
user-invocable: true
argument-hint: "[indexing or analytics task]"
---

# XOXNO Lending data operations

The wire catalog
[docs/reference/events.md](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/events.md)
defines topics, payload layouts, enum discriminants, units, ordering, and
inherited events. Do not copy the catalog into an indexer. This skill covers
how to consume the catalog. For REST, see [api.md](api.md). For deployment
coordinates, see [addresses.md](../xoxno-lending/addresses.md).

## Wallet and application reads

Portfolio and asset screens use the [v1 arrays](api.md#integrator-v1-arrays):
balances, prices, token labels and logos, APYs, capacity estimates, and one
position per indexed owned NFT, with no event indexing or joins. The optional
SDK read entry paginates and exports the public types. Plain HTTP works from
any language.

Keep raw strings and nullable fields intact. Indexed data and cached prices
can lag the ledger. Recheck NFT ownership and prepare through the selected
network's RPC before signing. See the
[wallet guide](https://xoxno.com/docs/stellar-lending/dev/wallet-integration).

## Subscribe by contract address

Resolve the addresses of the selected network, `testnet` or `mainnet`, from
`configs/networks.json`. Subscribe to at least these contracts:

- controller: account deltas, liquidation, bad debt, configuration
- pool: market indexes, cash, shares, parameters
- position NFT: mint, transfer, burn
- relevant tokens: transfers and approvals, when cash-flow evidence is needed
- price aggregator (use the `price_aggregator` key, not the swap router's
  `aggregator` key): oracle configuration changes

Match the emitting contract address and the ordered topic vector. Topic text
alone is not an identity.

## Asset and market identity

Discover assets in this order, with the contract address first:

1. Start from the token contract address (configuration, event, or explicit
   user selection).
2. Resolve every v1 asset row for that address.
3. Select an explicit `(spoke_id, hub_id, asset)` tuple.
4. Use symbol and name only as display metadata afterward.

Never take the first symbol or search match. Symbols are non-unique, and one
asset contract can sit in several hubs and spokes. Key account legs by
`(account_id, hub_id, asset)`, key reserve state by
`(spoke_id, hub_id, asset)`, and key pool state by `(hub_id, asset)`.

## Ingestion loop

For every `getEvents` page, follow these steps:

1. Request by contract ids and, where useful, exact topics.
2. Drop events with `inSuccessfulContractCall === false`. Reverted diagnostic
   events are not state transitions, activities, or liquidation candidates.
3. Decode known events. Keep unknown events for later decoders instead of
   failing the page.
4. Apply the page transactionally, with the idempotency key
   `(txHash, eventId, childIndex)`.
5. Commit derived rows and the response cursor durably together.
6. Resume with `{ cursor }`. Never mix cursor and ledger-range modes.

Order by ledger, transaction index, operation index, and event order from the
RPC event id. Give each child record of a batched event a stable child index.
Never order by pool timestamp, because updates can share it.

Before RPC retention, REST history can seed derived balances, charts, or
candidate sets. It cannot seed raw events, transaction order, or missing
payloads unless an independent archival source supplies them. Label
REST-seeded state with provenance. Reconcile it once raw history is available.

## Decoding pitfalls

- Soroban map fields are symbol-keyed. Never decode by Rust declaration order.
- Vec and tuple payloads are positional. Single-value event data is the value
  itself, not a map holding the Rust field name.
- Preserve unknown action discriminants as data.
- Amounts and cash are token base units unless the event reference marks RAY, WAD,
  or BPS.
- Pool timestamps are milliseconds. Ledger close times are seconds.
- Position `scaled_amount` is the resulting RAY share balance, not a delta.
- Position `amount` is the account's token movement. Some events report a
  requested amount, not a measured receipt, for example
  `FlashPositionEvent.amount` and `InitialMultiplyPaymentEvent.amount`.
- `decodeStellarLendingEvent` (`@xoxno/sdk-js/stellar-lending`) returns `null`
  for an undecoded topic, not an invalid chain event.

## State is broader than events

Events are incomplete state. These cases show the gaps:

- a position batch with no legs is not emitted
- bad-debt cleanup emits no cleanup position batch
- constructors and inherited helpers can mutate state without a custom event
- token and downstream contract state need their own events or direct reads
- prices resolve at call time and change without controller events

Treat events as ordered change evidence. Reconcile account, market, NFT, and
token state with views. Controller events alone cannot reconstruct state.

## Account lifecycle

The position NFT token id is the lending account id. Track these events:

- mint: account creation, including `Credit(0)` liquidation receivers
- transfer: owner change
- burn: account deletion
- controller position batches: owner, spoke, mode, position legs

No controller `AccountCreated` event exists. Emptying debt need not burn the
account, and an absent batch does not prove nothing changed.

## Liquidation accounting

Group the successful transaction's controller events by the event reference's
ordering. Pool, NFT, and token events can interleave.

- `LiquidationEvent`: measured repayment value capped at each planned debt leg
  (`repaid_usd_wad`, USD WAD) and bonus (`bonus_bps`, BPS). It does not report
  collateral proceeds.
- Target `LiqSeize` legs: gross collateral movement.
- Credit receiver `LiqCredit` legs: net credited movement. Zero-net legs are
  omitted.
- Transfer mode emits no fee leg. `LiqSeize.amount` is gross, the pool sends
  the net to the liquidator, and the fee is the difference. Before execution,
  `get_liquidation_estimate` reports the fee in `protocol_fees`.

In Credit mode, approximate the per-asset fee from gross and net token
movement. For exact share fees, process batches in event order, earlier account
changes first. Use the balances immediately before each liquidation's target
and receiver batches, never the transaction-start balances. An omitted receiver leg
credits zero shares. Key prior balances by `(hub_id, asset)`. Each emitted
`scaled_amount` is a resulting balance:

```text
target_seized_share_delta =
    target_prior_scaled[(hub_id, asset)] - target_resulting_scaled[(hub_id, asset)]

receiver_credited_share_delta =
    receiver_resulting_scaled[(hub_id, asset)] - receiver_prior_scaled[(hub_id, asset)]

exact_fee_shares[(hub_id, asset)] =
    target_seized_share_delta - receiver_credited_share_delta
```

Value the share delta with the matching `(hub_id, asset)` supply index and
directed rounding. A new `Credit(0)` receiver's prior balance is zero. Without
both keyed prior balances, the exact fee is not derivable.

Never reconstruct gross seizure from repayment times bonus. Estimate net
proceeds conservatively. Transfer estimates can round a full claim above the
pool payout, so clamp gross tokens to the floor of the held claim before you
subtract the fee. Credit estimates use RAY supply shares, so subtract fee
shares before conversion. Simulate the complete write and reconcile confirmed
effects as the [liquidation runbook](../xoxno-lending-liquidations/SKILL.md)
describes.

## Candidate discovery

Take account ids only from successful `position:batch_update` events, and seed
from REST if needed. Re-evaluate on a timer and on pool index changes. Drop
missing or debt-free accounts after on-chain checks. Persist the event cursor
before advancing the external checkpoint.

## Completion checks

- [ ] Contract addresses from the selected network configuration.
- [ ] Every asset selection has explicit spoke, hub, and contract address.
- [ ] Reverted events skipped.
- [ ] Rows and cursor committed durably and idempotently.
- [ ] REST-seeded records labeled derived, not raw events.
- [ ] Exact liquidation share fees use prior scaled balances.
- [ ] Event-derived results reconciled with current views.
