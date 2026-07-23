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
| `debt:bad_debt` | bad debt cleaned/socialized |
| `strategy:initial_payment` / `strategy:fee` | strategy verb legs |
| `market:create` / `market:params_update` | market lifecycle |
| `market:batch_state_update` / `market:batch_params_update` | market snapshots |
| `config:spoke` / `config:oracle` / `config:swap_aggregator` / `config:price_aggregator` / `config:accumulator` / `config:pool_template` / `config:position_limits` | governance config changes |

## Position actions

On the wire each `position:batch_update` delta carries a **u32 action
discriminant**; the SDK maps it to a frozen legacy string table:

`supply`, `borrow`, `withdraw`, `repay`, `liq_repay` (4), `liq_seize` (5),
`multiply`, `param_upd`, `sw_debt_r` (debt swap), `sw_col_wd` (collateral
swap), `rp_col_wd` / `rp_col_r` (repay-with-collateral legs), `close_wd`
(close-out withdraw), plus `Migrate` (13) and `RpColNet` (14) — the SDK
currently surfaces those last two as the raw discriminant strings `'13'` /
`'14'`, so don't treat unknown action strings as errors.

## Pipeline design notes

- **Track accounts, not addresses.** Positions key on the `u64` account id;
  liquidation legs are emitted on the liquidated account.
- **Market snapshots ride on every mutation.** `market:batch_state_update`
  carries accrued indexes plus accounting state (`cash`, `supplied`,
  `borrowed`, `revenue`) — enough to maintain index series and *derive* rates
  off-chain; the events do not carry rates themselves.
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
- **Expecting rates in snapshots** — derive rates from the accounting fields;
  only indexes and state are emitted.
- **Assuming one event per transaction** — strategy verbs emit several
  position deltas plus market snapshots; order with `syntheticEventOrder`.
