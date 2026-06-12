# Reflector per-asset reads — investigation result

Question (queued after the bulk-endpoint measurement): can the 20 per-asset
Reflector `prices()` frames in a wide liquidation be collapsed, the way the
RedStone and pool reads were?

## Finding: no — Reflector's ABI forces one frame per asset

The deployed oracle (testnet CEX instance `CCYO…RN63`, production shape)
exposes only per-asset reads, verified from the on-chain contract spec:
`lastprice(asset)`, `price(asset, timestamp)`, `prices(asset, records)`,
plus admin/config fns. There is no multi-asset endpoint and no cross-price
helper on this contract. The bulk-read lever does not exist on their side.

## Measured cost of the per-feed pipeline (run 20260611-bulkpool ladders)

- Single-source (Reflector primary only): **~10.05M instructions per feed**
  (probe ladder 11→20 feeds, R² visually linear).
- Dual-source (Reflector + bulked RedStone anchor): **~11.17M/feed** — the
  entire RedStone payload parse adds only ~1.1M/feed, confirming the earlier
  bulking removed its frame cost.
- At 10+10 width the per-feed pipeline is ~200M of the 366M total (~55%).
  The ~10M/feed bundles the Reflector call frame, the oracle's own storage
  reads (3 TWAP records), and the controller's compose/median/tolerance math
  — not separable from these ladders alone.

## Options (both are product decisions, not pure optimizations)

1. **Keeper-pushed price mirror**: a self-owned contract the keeper refreshes
   from Reflector; the controller reads all prices in ONE call (and could
   skip per-feed TWAP math if the mirror stores final composed prices).
   Largest possible saving — plausibly 100M+ at width 20 — but it inserts
   the keeper into the oracle trust path (freshness liveness, a new staleness
   window, and the mirror becomes the de-facto oracle). This is a protocol
   trust-model change, not a refactor.
2. **Reduce per-feed work**: Twap(3) → Twap(2) or Spot cuts the oracle's
   storage reads and the median math. Cheaper but weakens manipulation
   resistance — explicitly rejected earlier for liquidation paths.
3. **Accept**: realistic mainnet accounts (5–8 distinct feeds) sit at
   ~60–90M instructions of oracle pipeline, nowhere near the 400M cap; only
   max-width (10+10) accounts approach it, and those clear with 7M declared
   headroom today.

## Recommendation

Accept (option 3) for launch. The walls that mattered are solved: events
12KB/16KB, memory 61% of cap, CPU 92% of cap at the absolute worst case the
position limits allow. Revisit the mirror (option 1) only if position limits
are raised beyond 10+10 or mainnet telemetry shows real accounts clustering
at max width — and treat it then as an oracle-architecture RFC, not an
optimization ticket.
