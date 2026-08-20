---
name: indexing-lending-events
description: Use when indexing XOXNO Lending — consuming controller/pool contract events from Soroban RPC getEvents, decoding position updates, liquidations, market snapshots, or building an off-chain database, analytics, or notification pipeline.
---

# Indexing XOXNO Lending Events

**REQUIRED BACKGROUND:** the `lending-protocol-fundamentals` skill (units,
HubAssetKey, account model).

## Overview

The protocol emits structured Soroban events for every state change.
`@xoxno/sdk-js/stellar-lending` ships a decoder for raw base64-XDR
topics/data from Soroban RPC `getEvents` — use it instead of hand-parsing
XDR (payload shapes are versioned with the contracts).

```ts
import {
  decodeStellarLendingEvent, STELLAR_LENDING_TOPICS,
} from '@xoxno/sdk-js/stellar-lending'

for (const ev of rpcEventsPage.events) {
  const decoded = decodeStellarLendingEvent(ev.topic, ev.value)
  if (!decoded) continue // topic this SDK does not decode — skip, don't throw
  // decoded.topic is the dispatch key, decoded.data the typed payload
}
```

`null` is expected and routine: besides third-party events (e.g.
access-control roles), the decoder intentionally skips several protocol
topics (`config:spoke_asset`, `config:remove_spoke_asset`, `config:hub`,
`config:approve_blend_pool`, `config:min_borrow_collateral`,
`strategy:blend_migration`). `STELLAR_LENDING_TOPICS` lists exactly what
decodes.

## Decodable event topics

| Topic | Meaning |
|---|---|
| `position:batch_update` | account position deltas (the workhorse — see actions below) |
| `position:liquidation` | liquidation summary for an account |
| `position:flash_loan` | flash loan executed |
| `position:flash_position` | `FlashPositionEvent`: debt mint summary (`amount`, `amount_received`, `fee` always 0). Collateral is **not** in this payload. |
| `debt:bad_debt` | bad debt cleaned/socialized |
| `strategy:initial_payment` / `strategy:fee` | strategy verb legs |
| `market:create` / `market:params_update` | market lifecycle |
| `market:batch_state_update` / `market:batch_params_update` | market snapshots |
| `config:spoke` / `config:swap_aggregator` / `config:price_aggregator` / `config:accumulator` / `config:position_limits` | governance config changes (controller) |
| `config:asset_oracle` | an asset's oracle configuration changed — emitted by the **price-aggregator** contract, not the controller, so subscribe to that address too |

There is no `config:oracle` topic. No contract emits it; the oracle-config
topic is `config:asset_oracle`.

## Position-NFT ownership events

Account ownership lives in the position-NFT contract, not the controller, so a
position changing hands produces no controller event at all. Index the NFT
contract to follow ownership. It uses the stock OpenZeppelin `stellar-tokens`
non-fungible events, and the token id is the controller account id.

| Topic | Data | Meaning |
|---|---|---|
| `transfer`, `from: Address`, `to: Address` | `token_id: u32` | the position moved to a new owner |
| `mint`, `to: Address` | `token_id: u32` | controller created the account |
| `burn`, `from: Address` | `token_id: u32` | controller deleted the account (liquidation cleanup, bad-debt socialization) |
| `approve`, `approver: Address`, `token_id: u32` | `approved`, `live_until_ledger` | per-token approval |
| `approve_for_all`, `owner: Address` | `operator`, `live_until_ledger` | collection-wide approval |

## Position actions

On the wire each `position:batch_update` delta carries a **u32 action
discriminant**; the SDK maps it to a frozen legacy string table:

The discriminants, in contract order:

| Value | Variant |
|---|---|
| 0 | `Supply` |
| 1 | `Borrow` |
| 2 | `Withdraw` |
| 3 | `Repay` |
| 4 | `LiqRepay` |
| 5 | `LiqSeize` |
| 6 | `Multiply` |
| 7 | `ParamUpd` |
| 8 | `SwDebtR` (debt swap) |
| 9 | `SwColWd` (collateral swap) |
| 10 | `RpColWd` (repay-with-collateral withdraw leg) |
| 11 | `RpColR` (repay-with-collateral repay leg) |
| 12 | `CloseWd` (close-out withdraw) |
| 13 | `Migrate` |
| 14 | `RpColNet` |
| 15 | `LiqCredit` |
| 16 | `FlashPos` |

The SDK maps 0-12 to a frozen legacy string table (`supply`, `borrow`,
`withdraw`, `repay`, `liq_repay`, `liq_seize`, `multiply`, `param_upd`,
`sw_debt_r`, `sw_col_wd`, `rp_col_wd`, `rp_col_r`, `close_wd`) and surfaces
the newer ones as raw discriminant strings (`'13'`, `'16'`, ...), so don't
treat unknown action strings as errors.

`FlashPos` (16) is the **debt mint** of `flash_position`. Callback collateral
in the same batch is still `Supply`.

### `LiqSeize` (5) versus `LiqCredit` (15)

A `SeizeMode::Credit` liquidation writes two accounts and publishes **two**
`position:batch_update` events: the liquidated account's batch first, the
receiving account's second. The liquidated account's legs are `LiqSeize` and
are **gross** of the protocol fee. The receiving account's legs are
`LiqCredit` and are **net** of it. The two tags exist precisely so the fee is
not double-counted: in credit mode the protocol fee is
`LiqSeize.amount - LiqCredit.amount`. An indexer that treats 15 as unknown
loses the liquidator's credited collateral entirely.

## Pipeline design notes

- **Track accounts, not addresses.** Positions key on the `u64` account id;
  liquidation legs are emitted on the liquidated account.
- **Market snapshots ride on every mutation.** `market:batch_state_update`
  carries a tuple per market: `hub_id`, `asset`, `timestamp`, `supply_index`,
  `borrow_index`, `cash`, `supplied`, `borrowed`, `revenue`. Only `cash` is in
  asset units. `supplied`, `borrowed` and `revenue` are **RAY-scaled share
  totals** (27 decimals): multiply by the accompanying `supply_index` /
  `borrow_index`, divide by RAY, and rescale to asset decimals before
  reporting them or deriving utilisation or TVL. `revenue` is a subset of
  `supplied`. The events carry no rates; derive rates from this state.
- **Event ids.** Soroban RPC event ids are two-segment (`<toid>-<index>`);
  the second segment is the in-transaction event index. For multi-delta
  events derive per-child ordering with the SDK's
  `syntheticEventOrder(baseOrder, childIndex)` (stride 10_000) and
  `extractEventOrder(eventId)`.
- **Idempotency.** Key rows on `(txHash, eventId, childIndex)` and upsert —
  re-scans and RPC retries deliver duplicates.
- **Historical topics.** The decoder keeps legacy keys (e.g.
  `config:oracle_disabled`) so replays of old ledgers still decode.

## Common mistakes

- **Importing from `@xoxno/sdk-js/stellar`** — the subpath is
  `stellar-lending`.
- **Throwing on unknown topics or action strings** — `null` decodes and raw
  discriminant strings (`'13'`, `'14'`) are expected; skip or map them.
- **Keying market tables by asset address** — include `hubId` or two hubs
  collapse into one corrupted market row.
- **Reading `supplied` / `borrowed` / `revenue` as asset amounts** — they are
  RAY-scaled shares; scale by the index first.
- **Assuming one `position:batch_update` per liquidation** — credit-mode
  liquidations publish a second one, on the receiving account.
- **Missing ownership changes** — they are position-NFT `transfer` events, not
  controller events.
- **Expecting rates in snapshots** — derive rates from the accounting fields;
  only indexes and state are emitted.
- **Assuming one event per transaction** — strategy verbs emit several
  position deltas plus market snapshots; order with `syntheticEventOrder`.

## A note on the TypeScript surface

Topics, discriminants and field units above come from the contracts. The
decoder names, exported symbols and helper signatures (`decodeStellarLendingEvent`,
`STELLAR_LENDING_TOPICS`, `syntheticEventOrder`, `extractEventOrder`) live in
the `@xoxno/sdk-js` repository, not in the protocol repository, and are not
verifiable here. Check them against `@xoxno/sdk-js/stellar-lending` before
relying on them.
