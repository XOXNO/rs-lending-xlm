# Price Aggregator

Single oracle entry point for the lending protocol. Owns token-rooted
`AssetOracle` configs and all provider reads: composition, primary/anchor
tolerance, staleness, sanity bands. Fail-closed on risk paths.

| | |
| --- | --- |
| Owner | Governance (`#[only_owner]`) |
| Consumers | Controller (and views) |
| Interface | `interfaces/price-aggregator` |
| Providers | Reflector, multi-feed (RedStone / XOXNO), recursive quotes |

## Role

```text
Controller ──prices(assets)──► PriceAggregator ──► Reflector / RedStone / XOXNO
                                    │
                         primary ± anchor, sanity band, max_stale
```

One `prices` call per transaction resolves every asset needed. Any unsafe,
stale, unconfigured, or out-of-band asset reverts the whole tx
([ADR 0003](../../architecture/decisions/0003-oracle-dual-source-with-tolerance-bands.md)).

## Surface

| Call | Behavior |
| --- | --- |
| `price` / `prices` | Fail-closed USD feeds |
| `price_status` / `prices_status` | Soft diagnostics (no revert on stale/deviation) |
| `oracle_config` | Read token-rooted config |
| `set_oracle_config` | Register/replace config (owner) |
| `set_sanity_band` / `set_tolerance` | Live band updates (owner) |

## Layout

```text
src/
  lib.rs         Entrypoints
  price.rs       Resolve USD price (fail-closed)
  status.rs      Soft PriceStatus
  compose.rs     Primary/anchor composition
  tolerance.rs   Deviation math
  providers/     Reflector, multi-feed adapters
  config.rs      Owner config writes + validation
  prefetch.rs    Multi-feed warm for bulk calls
  context.rs     Per-tx resolution cache
  observation.rs Source observations
  storage.rs     Oracle config keys
```

## Related

| Doc | Topic |
| --- | --- |
| [ADR 0003](../../architecture/decisions/0003-oracle-dual-source-with-tolerance-bands.md) | Dual-source + bands |
| `contracts/xoxno-oracle` | Self-hosted multi-signer feed |
| `common` oracle types | `AssetOracleConfig`, `PriceFeedRaw`, `PriceStatus` |
