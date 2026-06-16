# ADR 0003: Oracle Dual-Source Pricing With Tolerance Bands

- Status: Accepted
- Date: 2026-05-05
- Revised: 2026-06-02
- Deciders: XOXNO Lending contract team
- Supersedes: none

> [!NOTE]
> Originally recorded on 2026-05-05 against the `ExchangeSource` model
> (`SpotOnly` / `SpotVsTwap` / `DualOracle`, `OracleProviderConfig`), which
> assumed Reflector as the only provider and encoded source diversity as a
> fixed strategy. The implementation has since generalized to the
> `OracleStrategy` model (`Single` / `PrimaryWithAnchor`) with two
> interchangeable providers: **Reflector** (SEP-40) and **RedStone**
> (price-feed), composed as a `primary` source and an optional `anchor`
> source that are deviation-checked against each other. The load-bearing
> decision, dual-source pricing validated by tolerance bands, is unchanged;
> the body below has been updated to the current model. See the Revisions
> section for the change record, and `SCF_BUILD_ARCHITECTURE.md` §9 for the
> matching reference description.

## Context

A lending protocol depends on price honesty for each solvency-relevant
operation: borrow, withdraw with debt, liquidation, and account-threshold
migration. A stale, wrong, or manipulated single price source can create bad debt
or wrongful liquidations.

On Soroban the protocol can price an asset through two independent providers:

- **Reflector** (SEP-40): CEX-aggregated and DEX-derived feeds, queried as
  spot or as a TWAP over a requested record count.
- **RedStone** (price-feed): a pull-based feed identified by a `feed_id`,
  carrying its own staleness bound and dual publish/write timestamps.

Two practical risks dominate:

1. Spot manipulation: a transient spike or dump that lasts long enough to
   trigger a borrow or liquidate.
2. Source outage: a stale or missing feed that would block all activity without
   policy gates.

## Decision

Resolve prices through `oracle::token_price`
(`contracts/controller/src/oracle/price.rs`, re-exported from
`contracts/controller/src/oracle/mod.rs`), normalized to USD WAD. Each market
declares a `MarketOracleConfig` (`common/src/types/oracle.rs`) with a
`strategy`, a `primary` source, and an optional `anchor` source.

**Two sources across two providers.** A source is an
`OracleSourceConfig::Reflector(..)` or `OracleSourceConfig::RedStone(..)`
(`OracleProviderKind::{ReflectorSep40, RedStonePriceFeed}`). A Reflector source
reads `Spot` or `Twap(records)` (`OracleReadMode`); RedStone reads spot.
`validate_oracle_config_shape` enforces two diversity rules on a
`PrimaryWithAnchor` pair:

1. **Different feeds.** Primary and anchor must not read the same feed (else
   `GenericError::InvalidExchangeSrc`), compared by feed identity: for
   Reflector the contract, asset, and read mode; for RedStone the contract and
   feed id, ignoring policy-only fields such as RedStone `max_stale_seconds`.
   The validator rejects two RedStone configs on the same contract and feed even
   when only their staleness bounds differ.
2. **Different providers (production).** In non-`testing` builds the primary and
   anchor must come from *different* providers: one Reflector, one RedStone
   (else `GenericError::InvalidExchangeSrc`). This places the two sources behind
   independent trust boundaries: a single provider's failure (bad feed,
   signer/contract compromise, feed-mapping error) moves only one side, so the
   deviation check trips instead of both prices sliding together. A same-provider
   pair (e.g. Reflector spot vs Reflector TWAP) shares one trust boundary and is
   invalid as a production `PrimaryWithAnchor`; temporal-only diversity
   is available instead through `Single` + Reflector TWAP (which carries no
   cross-check).

Markets still choose the specific Reflector and RedStone feeds and which one is
primary; operators must also choose economically independent feeds, not distinct
providers alone.

**Two validation strategies.** Configured per market by
`OracleStrategy` (`common/src/types/oracle.rs`):

- `PrimaryWithAnchor`: read both sources and cross-check them against the
  market's tolerance bands. An anchor is required, and in production the
  primary/anchor pair must cross providers (one Reflector, one RedStone); see
  the two diversity rules above.
- `Single`: use the primary source without a cross-check. In
  non-`testing` builds a `Single` market's primary must carry temporal
  diversity, so only a Reflector `Twap` primary is accepted; a Reflector
  `Spot` primary or any RedStone primary (RedStone reads spot) is
  rejected at listing time (`GenericError::SpotOnlyNotProductionSafe`). A
  RedStone source must therefore be used under `PrimaryWithAnchor` (paired with
  an anchor) in production.

**Tolerance bands and final-price selection.** Each market stores an
`OraclePriceFluctuation` with a tight `first` band and a wider `last` band,
each as upper/lower BPS ratio bounds. The bands are derived once from the
configured `first_tolerance` / `last_tolerance` inputs by
`tolerance::validate_and_calculate_tolerances`, which enforces
`MIN_FIRST_TOLERANCE`/`MAX_FIRST_TOLERANCE`,
`MIN_LAST_TOLERANCE`/`MAX_LAST_TOLERANCE` (`common/src/constants/oracle.rs`)
and `first < last` (`oracle::tolerance::require_last_tolerance_gt_first`).

Composition happens in `oracle::compose::resolve_components`. The `primary`
price is the **safe** price; the `anchor` is the **aggregator**. Under
`PrimaryWithAnchor`, `tolerance::calculate_final_price` selects:

1. Within the first band → use the **safe (primary)** price.
2. Within the last band (but outside the first) → use the **midpoint** of the
   two prices.
3. Outside the last band → revert with `OracleError::UnsafePriceNotAllowed`
   unless the caller's policy permits it
   (`OraclePolicy::allows_unsafe_deviation`), in which case fall back to the
   safe price (see ADR 0004).

`is_within_anchor` computes the ratio `safe * RAY / aggregator`, rescales it to
BPS, and checks it against the band bounds.

**Anchor degradation.** If the anchor source is unconfigured, missing, or
stale-but-policy-permits, `resolve_components` degrades to the primary price
via `fallback_to_primary`, gated by `OraclePolicy::allows_degraded_dual_source`
(otherwise `OracleError::NoLastPrice`). A missing primary reverts.

**Sanity bounds.** After composition, `token_price` rejects any final price
outside the market's inclusive `[min_sanity_price_wad, max_sanity_price_wad]`
window, or an unconfigured (`max_sanity_price_wad <= 0`) window, with
`OracleError::SanityBoundViolated`. The `pending_for` self-pointer sentinel and
non-active market statuses are rejected before pricing
(`OracleError::OracleNotConfigured`, `GenericError::PairNotActive`).

**Unconditional clock-skew gate.** `observation::check_not_future_at` rejects
oracle timestamps more than `MAX_FUTURE_SKEW_SECONDS` (60s) in the future with
`OracleError::PriceFeedStale`, regardless of policy. Staleness on the past side
is bounded per source by `observation::is_stale`: a Reflector source uses the
market-level `max_price_stale_seconds`; a RedStone source carries its own
`max_stale_seconds`. Both are clamped to
`[MIN_PRICE_STALE_SECONDS, MAX_PRICE_STALE_SECONDS]` (60s..86_400s) at listing
time.

**Listing-time bounds.** Governance `propose_configure_market_oracle` resolves
and validates the config before scheduling controller `set_market_oracle_config`.
The controller re-checks quote-market invariants at execution before it
persists the config. The validation path covers:
strategy/anchor coherence (`PrimaryWithAnchor` ⇔ an anchor is configured);
primary/anchor diversity: different feeds, and in production different
providers (`GenericError::InvalidExchangeSrc`); the production
naked-spot-`Single` rejection (a `Single` primary that reads spot: Reflector
`Spot` or any RedStone); token decimals fetched from the token contract;
staleness and sanity bounds; and, per source, a live feed read plus
provider-specific checks. For a Reflector source: `base() == USD`
(`InvalidOracleBase`), oracle decimals in `[1, 18]` (`InvalidOracleDecimals`),
resolution in `[MIN_ORACLE_RESOLUTION_SECONDS, max_stale]`
(`InvalidOracleResolution`), a live `lastprice`, and, for a TWAP read, at
most `MAX_TWAP_RECORDS` (12) records with sufficient non-empty history
(`TwapInsufficientObservations`, `ReflectorHistoryEmpty`). For a RedStone
source: a per-source staleness bound, fixed `REDSTONE_DECIMALS`, and a live
`read_price_data` validated on both its package and write timestamps.
`propose_edit_oracle_tolerance` only re-validates the first/last tolerance
inputs (`validate_and_calculate_tolerances`) and schedules the rewritten band
fields; it does not re-probe the configured sources.

## Alternatives Considered

- **Single CEX spot price.** Rejected: no manipulation defense; a single
  manipulated tick can trigger a liquidation or under-collateralized
  borrow. Production markets cannot run a naked-spot `Single` source: a
  `Single` primary must be a Reflector TWAP, and RedStone (spot) must be
  paired with an anchor.
- **TWAP-only.** Rejected: TWAP lags real moves and exposes the protocol
  to predictable arbitrage during fast price drops; users cannot react to
  threshold migrations either. TWAP remains available as a Reflector read
  mode and as an anchor.
- **A fixed, named provider topology (the old `SpotVsTwap` / `DualOracle`
  strategies).** Rejected: baking specific source roles into named strategy
  enums removed per-market feed choice and assumed a single provider. The
  generic `primary`/`anchor` model is kept instead: each market picks its own
  Reflector and RedStone feeds and which one is primary, with one production
  constraint layered on top: the pair must cross providers (above), so the
  cross-check spans two independent trust boundaries rather than one.
- **Manual circuit breaker (off-chain pause on deviation).** Rejected as
  the only line of defense; off-chain monitors are still useful but cannot
  be a load-bearing oracle gate.
- **Custom oracle aggregator contract.** Rejected for launch: adds
  another upgradeable surface and another trust assumption. The chosen
  design reads Reflector and RedStone in the controller and validates tolerance,
  staleness, and sanity bounds in-contract, and rejects a primary/anchor pair
  that reads the same feed; ensuring the two distinct feeds are also
  economically independent remains an operator listing-time responsibility.

## Consequences

Positive:

- Risk-bearing decisions on a `PrimaryWithAnchor` market are gated by a
  cross-*provider* deviation check, so a single provider's compromise or failure
  moves only one side and trips the band rather than passing unnoticed.
- The midpoint band absorbs small honest deviations without halting the
  protocol; the first band prefers the safe (primary) price.
- Anchor unavailability degrades to the primary under permissive policies.
  Strict policies reject out-of-band divergence.
- The clock-skew gate rejects future-dated feeds in all modes, and sanity
  bounds reject absurd prices even when a single source is corroborated.

Negative / accepted costs:

- Up to two oracle cross-contract reads per priced asset per touched cache
  slot.
- Tolerance bounds, staleness limits, and sanity bounds are configuration, not
  code; they must be set conservatively per market and audited at listing time.
- A correlated outage of both sources will revert risk-increasing flows;
  this is the intended fail-safe (ADR 0004 explains how risk-decreasing
  flows degrade).
- The cross-provider check defends against *one* provider failing. It does not
  catch a correlated failure that moves both providers the same way (e.g. a
  shared upstream exchange both read from), nor a manipulation small
  enough to stay inside the tolerance bands. Those residual risks are bounded by
  the band widths and the sanity bounds, not eliminated; selecting
  independent feeds and conservative bands remains an operator responsibility.

## Revisions

### 2026-06-02: Generalized from the `ExchangeSource` model to the `OracleStrategy` model with Reflector + RedStone

The original 2026-05-05 decision used `ExchangeSource`
(`SpotOnly` / `SpotVsTwap` / `DualOracle`) and `OracleProviderConfig`, assuming
Reflector as the sole provider and encoding source diversity as a fixed
strategy. The oracle subsystem was refactored (`contracts/controller/src/oracle/`)
to the current shape:

- Strategy is now `OracleStrategy::{Single, PrimaryWithAnchor}`; diversity is
  expressed by a `primary` source and an optional `anchor` source rather than a
  named topology.
- Each source is `OracleSourceConfig::{Reflector, RedStone}`
  (`OracleProviderKind::{ReflectorSep40, RedStonePriceFeed}`); RedStone is new.
  Primary and anchor can be any two distinct sources, so RedStone and Reflector
  deviation-check each other when configured as a pair.
- Final-price selection (`tolerance::calculate_final_price`) and the deviation
  gate keep the same shape (first band → safe, last band → midpoint, beyond
  → policy-gated), but the out-of-band behavior is now driven by
  `OraclePolicy::allows_unsafe_deviation` (see ADR 0004, which also reverses
  liquidation's deviation tolerance).
- New since 2026-05-05: per-market `[min_sanity_price_wad, max_sanity_price_wad]`
  bounds (`OracleError::SanityBoundViolated`); the `check_not_future_at` clock
  gate (renamed/relocated to `observation`); and the primary/anchor diversity
  guard.
- The production `Single`-strategy gate (`SpotOnlyNotProductionSafe`) now
  rejects each naked-spot primary: Reflector `Spot` and RedStone. The original
  check predated RedStone and covered Reflector; a `Single` market must use a
  Reflector TWAP, and RedStone requires an anchor.
- The primary/anchor diversity guard now compares feed *identity* (provider +
  contract + feed key, ignoring policy-only fields like RedStone
  `max_stale_seconds`) rather than whole-struct equality, so two RedStone
  sources on the same contract and feed can no longer be paired by varying only
  their staleness bound. Same pre-RedStone-era origin as the `Single` gate
  above.
- Production `PrimaryWithAnchor` now requires the primary and
  anchor to cross providers (one Reflector, one RedStone), enforced in
  `validate_oracle_config_shape` (`#[cfg(not(feature = "testing"))]`). The
  feed-identity guard alone still allowed same-provider pairs (e.g. Reflector
  spot vs Reflector TWAP) that share one trust boundary, so a single-provider
  compromise could move both sources together and pass the deviation check.
  Cross-provider pairing makes "both providers per market" a code-enforced
  invariant rather than an operator convention. (Raised by Codex
  adversarial-review, 2026-06-02.)

## References

- `SCF_BUILD_ARCHITECTURE.md` §9 (Oracle Pricing), `architecture/INVARIANTS.md`
  §4.2 to §4.3 (Oracle Configuration, Price Resolution).
- `contracts/controller/src/oracle/price.rs::token_price`
- `contracts/controller/src/oracle/compose.rs::resolve_components`
- `contracts/controller/src/oracle/tolerance.rs::{calculate_final_price, is_within_anchor, validate_and_calculate_tolerances}`
- `contracts/controller/src/oracle/observation.rs::{check_not_future_at, is_stale, validate_timestamp}`
- `contracts/controller/src/oracle/providers/{mod.rs::read_source, reflector/, redstone/}`
- `contracts/controller/src/oracle/validation/{config.rs, oracle.rs::validate_market_oracle_sources}`
- `contracts/governance/src/forward.rs::{propose_configure_market_oracle, propose_edit_oracle_tolerance}`
- `contracts/controller/src/governance/config.rs::{set_market_oracle_config, set_oracle_tolerance}`
- `common/src/types/oracle.rs` (`OracleStrategy`, `OracleSourceConfig`, `MarketOracleConfig`, `OraclePriceFluctuation`, `OracleReadMode`, `OracleAssetRef`)
- `common/src/constants/oracle.rs` (`MIN_FIRST_TOLERANCE`, `MAX_FIRST_TOLERANCE`, `MIN_LAST_TOLERANCE`, `MAX_LAST_TOLERANCE`)
