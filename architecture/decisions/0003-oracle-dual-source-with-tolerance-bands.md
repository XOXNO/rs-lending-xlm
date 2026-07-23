# ADR 0003: Oracle Dual-Source Pricing With Tolerance Bands

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team

## Context

Solvency-critical paths (borrow, debt-bearing withdraw, liquidation, threshold
refresh) need honest USD prices. A single manipulated or stale feed can create
bad debt or wrongful liquidations.

Providers on Soroban:

| Provider | Shape | Notes |
|----------|-------|--------|
| **Reflector** (SEP-40) | Spot or TWAP | CEX/DEX feeds |
| **RedStone** | Spot feed by `feed_id` | Pull-based; fixed decimals |
| **Xoxno** (`xoxno-oracle`) | RedStone wire ABI | Protocol N-of-M signers; decimals via SEP-40 at listing |

Risks: short-lived spot manipulation, and source outage. Outage handling is
which flows call the oracle (ADR 0004), not weaker price validation.

## Decision

Resolve prices through the price-aggregator (`resolve_usd_price` /
`prices` / soft `prices_status` for views) to USD WAD. Each asset stores an `AssetOracleConfig`
(storage key `AssetOracle(asset)`): strategy, primary source, optional anchor,
tolerance, sanity band, and staleness bounds.

### Sources and diversity

A source is `OracleSourceConfig::{Reflector, RedStone, Xoxno}`. For
`PrimaryWithAnchor`, governance `validate_oracle_config_shape` requires:

1. **Different feeds** (`reads_same_feed_as`) — same Reflector contract/asset/mode
   or same RedStone/Xoxno `(contract, feed_id)` counts as one feed.  
2. **Different providers** (production).  
3. **Different contracts** (production) — the Xoxno dual ABI cannot back both legs.  

Operators also choose economically independent feeds, not only distinct providers.

### Strategies

| Strategy | Rules |
|----------|--------|
| **PrimaryWithAnchor** | Anchor required. Production: non-spot primary (typically Reflector TWAP) and a different provider/contract as anchor. Spot primary reverts with `SpotOnlyNotProductionSafe`. |
| **Single** (PrimaryOnly; storage discriminant kept as `Single`) | Primary only. Spot is allowed. Sanity band capped at ±10% midpoint-relative (`MAX_SINGLE_SOURCE_SANITY_BAND_BPS`). Wider bands require the anchored strategy. |

### Tolerance

One `OracleTolerance` from `tolerance_bps` in
`[MIN_TOLERANCE, MAX_TOLERANCE]` (150..2_500 BPS). Under `PrimaryWithAnchor`,
both legs are read and freshness-checked, then `midpoint_if_in_band` compares
the **primary/anchor ratio in BPS** to `[lower_ratio_bps, upper_ratio_bps]`:

1. Inside the band → integer midpoint `(primary + anchor) / 2`  
2. Outside the band → revert `UnsafePriceNotAllowed`  

There is no primary-only fallback. A stale or missing primary or anchor reverts.
`Single` stores a tolerance field but does not use it at compose time.

Production dual-source diversity and non-spot primary rules apply under
`#[cfg(not(feature = "testing"))]` (test builds may relax them).

### Sanity and time

The composed price must lie in
`[min_sanity_price_wad, max_sanity_price_wad]` or the call reverts
`SanityBoundViolated`. Future timestamps beyond `MAX_FUTURE_SKEW_SECONDS` (60s)
revert. Past staleness: Reflector uses market `max_price_stale_seconds`;
RedStone/Xoxno use per-source `max_stale_seconds`; both are clamped to
`[60, 86_400]` seconds at listing.

### Listing path

Governance schedules `ConfigureMarketOracle` and `EditOracleTolerance` after
validation and a live probe. Execute invokes price-aggregator
`set_oracle_config` and persists `AssetOracle(asset)`. `EditOracleTolerance`
re-validates the band only. Aggregator `set_tolerance` runs
`validate_oracle_tolerance` so a direct owner call cannot store a degenerate
band.

## Alternatives considered

- Unbounded single spot — no manipulation bound.  
- TWAP-only — lag and arbitrage during fast moves.  
- Fixed named topologies (`SpotVsTwap` / `DualOracle`) — too rigid per market.  
- Two-tier tolerance with degraded primary — extra branches; single band with
  binary outcome is simpler and matches listing rules.  
- Off-chain circuit breaker as the only gate — not enforceable on-chain.  
- Custom oracle aggregator contract — extra upgrade and trust surface.  

## Consequences

**Positive:** cross-provider check on anchored markets; midpoint absorbs small
honest noise; fail-closed outside the band; clock skew and sanity reject absurd
prints.

**Costs:** up to two oracle reads per priced asset; operators own feed and band
quality; dual-source outage reverts priced flows (unpriced flows follow ADR 0004);
correlated upstream moves can pass the band if both legs move together.

## References

- [INVARIANTS.md](../INVARIANTS.md) §4.2–4.3  
- [ADR 0004](./0004-cache-permissiveness-policy.md)  
- `contracts/price-aggregator/src/{price,compose,tolerance,observation,providers,prefetch}`  
- `interfaces/price-aggregator`  
- `contracts/governance/src/validate/{oracle_config,oracle_probe,tolerance}.rs`  
- `contracts/governance/src/op.rs`  
- `common/src/types/oracle.rs`, `common/src/constants/shared.rs`  
- `common/src/validation.rs`  
- `contracts/xoxno-oracle`  
