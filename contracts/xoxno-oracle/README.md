# XOXNO Oracle

Self-hosted multi-signer price oracle for assets without a native
Reflector/RedStone feed (e.g. RWA). Registered signers push submissions; the
contract stores the latest per signer per feed and recomputes a median aggregate
at write time. N-of-M threshold keeps reads O(1).

| | |
| --- | --- |
| Owner | OZ `Ownable` |
| Reads | RedStone-style + SEP-40 / Reflector-style |
| Provider kind | `OracleProviderKind::XoxnoPriceFeed` |

## Role

Listed as `OracleSourceConfig::Xoxno` on the price-aggregator. Independent
second opinion next to Reflector/RedStone; primary and anchor must never share
this contract address.

## Freshness

| Knob | Purpose |
| --- | --- |
| `MaxSubmissionAgeSeconds` | Absolute age for inclusion in the median |
| `MaxRelativeSkewSeconds` | Drop lagging peers vs freshest (default = age window) |
| `MaxStaleSeconds` | How long a cached aggregate may be served |

Keep submission age ≤ consumer `max_stale_seconds`. Below threshold: aggregate
cleared; reads fail closed (`NoDataForFeed` / `StaleData` / SEP-40 `None`).

## Surface

| Area | Notes |
| --- | --- |
| Submit | `submit_price` / `submit_prices` — signer auth, known feed, non-decreasing timestamps |
| Reads | `read_price_data*`; SEP-40 `lastprice` / `price` / `prices` / TWAP |
| Admin | Signers, threshold, feeds, windows, skew, upgrade |
| Hygiene | `purge_feed` clears stale per-signer state |

Production: threshold ≥ 2; register feeds before submit; tight sanity band when
used as sole `Single` source.

## Layout

```text
src/
  lib.rs          Entrypoints + constructor
  submit.rs       Signer submissions + threshold
  reads.rs        RedStone + SEP-40 surfaces
  admin.rs        Owner admin
  aggregation.rs  Median + freshness windows
  storage.rs      Feed / signer / aggregate keys
```
