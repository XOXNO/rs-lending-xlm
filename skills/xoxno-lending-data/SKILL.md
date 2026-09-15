---
name: xoxno-lending-data
description: Use when consuming XOXNO Lending data off-chain — indexing controller, pool, position-NFT, token, or oracle events; tracking account lifecycle; deriving liquidation accounting; building analytics; or calling the public Stellar lending REST API.
user-invocable: true
argument-hint: "[indexing or analytics task]"
---

# XOXNO Lending data operations

The canonical wire catalog is
[docs/reference/events.md](../../docs/reference/events.md). It defines event
topics, payload layouts, enum discriminants, units, ordering, and inherited
events. Do not copy that catalog into an indexer guide. This skill covers the
operational rules needed to consume it.

Use [api.md](api.md) for the public REST surface and
[addresses.md](../xoxno-lending/addresses.md) for deployment coordinates.

## Subscribe by contract address

Resolve addresses from the selected network's `configs/networks.json`, then
subscribe to at least:

- controller: account deltas, liquidation, bad debt, and configuration;
- pool: market indexes, cash, shares, and parameters;
- position NFT: mint, transfer, and burn lifecycle;
- relevant token contracts: transfers and approvals when cash-flow evidence is
  required;
- price aggregator: oracle configuration changes.

Match both emitting contract address and ordered topic vector. Topic text alone
is not an identity.

## Asset and market identity

Discover assets contract-address-first:

1. Start with the token contract address from configuration, an event, or an
   explicit user selection.
2. Resolve every reserve row for that address.
3. Select an explicit `(spoke_id, hub_id, asset)` tuple.
4. Use symbol/name only as display metadata after identity is fixed.

Never select the first display-symbol match or the first API/search result.
Symbols are non-unique and one asset contract can exist in several hubs and
spokes. Key account legs by `(account_id, hub_id, asset)` and reserve state by
`(spoke_id, hub_id, asset)`; key pool state by `(hub_id, asset)`.

## Ingestion loop

For every `getEvents` page:

1. Request by contract ids and, where useful, exact topics.
2. Drop events where `inSuccessfulContractCall === false`. Reverted diagnostic
   events are not state transitions, activities, or liquidation candidates.
3. Decode known events; preserve unknown events for later decoder upgrades
   instead of crashing the page.
4. Apply the page transactionally with idempotency key
   `(txHash, eventId, childIndex)`.
5. Persist derived rows and the response cursor in the same durable commit.
6. Resume with `{ cursor }`; do not combine cursor and ledger-range modes.

Sort by ledger, transaction index, operation index, and event order parsed from
the RPC event id. A batched event contains ordered child records; assign a
stable child index. Never order by pool timestamp because several updates can
share it.

If a requested start ledger is older than RPC retention, REST history can seed
derived balances, charts, or candidate sets. It cannot reconstruct canonical
raw events, transaction ordering, or missing event payloads unless an
independent archival source proves and supplies those records. Label REST-
seeded state with provenance and reconcile it when raw history becomes
available.

## Decoding pitfalls

- Soroban map fields are keyed by symbols; never decode map values by Rust
  declaration order.
- Vec and tuple payloads are positional. Single-value event data is the value
  itself, not a map containing the Rust field name.
- Preserve unknown action discriminants as data.
- Amounts/cash use token base units unless the canonical event reference marks
  RAY, WAD, or BPS.
- Pool timestamps are milliseconds; ledger close times are seconds.
- Position `scaled_amount` is the resulting RAY share balance, not the delta.
- Position `amount` is the operation's token-unit movement, but some events
  intentionally report requested rather than measured receipt.
- A decoder returning `null` means unsupported/unrecognized for that decoder,
  not an invalid chain event.

## State is broader than events

Events are not complete state truth:

- zero-delta position batches can be suppressed;
- bad-debt cleanup does not emit a cleanup position batch;
- constructors and inherited helpers can mutate state without a protocol
  custom event;
- token behavior and downstream contract state require their own event
  standards or direct reads;
- prices are resolved at call time and can change without controller events.

Use events as ordered change evidence, then reconcile account, market, NFT,
and token state with views. Do not claim canonical state reconstruction from
controller events alone.

## Account lifecycle

The position NFT token id equals the lending account id. Track:

- mint: account creation, including `Credit(0)` liquidation receivers;
- transfer: owner change;
- burn: account deletion;
- controller position batches: current owner, spoke, mode, and position legs.

There is no controller `AccountCreated` event. A repayment that empties debt
does not necessarily burn the account, and an absent batch does not prove no
state changed elsewhere.

## Liquidation accounting

Group the successful transaction's controller events using the ordering in the
canonical event reference. Pool, NFT, and token events can interleave.

- The liquidation summary reports measured debt retired and bonus, not
  collateral proceeds.
- Target `LiqSeize` legs report gross collateral movement.
- Credit receiver `LiqCredit` legs report net credited movement and omit
  zero-net legs.
- Transfer-mode protocol fee is not separately recoverable from controller
  position events; use the estimate/transaction context or authoritative
  state/accounting source.

For Credit mode, approximate per-asset fee from gross and net token movement.
For exact share-fee derivation, snapshot the **prior scaled balance of both the
liquidated target and the Credit receiver**, keyed by `(hub_id, asset)`, before
applying transaction events. Every emitted `scaled_amount` is a resulting
balance, not a delta:

```text
target_seized_share_delta =
    target_prior_scaled[(hub_id, asset)] - target_resulting_scaled[(hub_id, asset)]

receiver_credited_share_delta =
    receiver_resulting_scaled[(hub_id, asset)] - receiver_prior_scaled[(hub_id, asset)]

exact_fee_shares[(hub_id, asset)] =
    target_seized_share_delta - receiver_credited_share_delta
```

Then value the share delta using the matching `(hub_id, asset)` supply index
and the protocol's directed rounding. A newly created `Credit(0)` receiver has
a prior balance of zero. Without both keyed prior balances, exact fee
derivation is impossible from the resulting `scaled_amount` values alone.

Do not reconstruct gross seizure from repayment times bonus. For profitability
use the pre-execution estimate's actual
`seized_collaterals - protocol_fees` legs and follow the
[liquidation runbook](../xoxno-lending-liquidations/SKILL.md).

## Candidate discovery

Collect account ids only from successful `position:batch_update` events, seed
from REST if needed, and re-evaluate on a timer and pool index changes. Drop
missing/debt-free accounts after on-chain checks. Persist the event cursor
before advancing the external checkpoint.

## Completion checks

- [ ] Contract addresses came from the selected network configuration.
- [ ] Every asset selection includes explicit spoke, hub, and contract address.
- [ ] Reverted events were skipped.
- [ ] Rows and cursor were committed durably and idempotently.
- [ ] REST-seeded records are labeled derived, not canonical raw events.
- [ ] Liquidation fees use prior scaled balance when exact share deltas matter.
- [ ] Event-derived results were reconciled with current state views.
