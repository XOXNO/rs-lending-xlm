# ADR 0003: Oracle Dual-Source Pricing With Tolerance Bands

- Status: Accepted
- Date: 2026-05-05
- Revised: 2026-07-10
- Deciders: XOXNO Lending contract team
- Supersedes: none

> [!NOTE]
> Originally recorded on 2026-05-05 against the `ExchangeSource` model
> (`SpotOnly` / `SpotVsTwap` / `DualOracle`, `OracleProviderConfig`), which
> assumed Reflector as the only provider and encoded source diversity as a
> fixed strategy. The 2026-06-02 revision generalized this to the
> `OracleStrategy` model (`Single` / `PrimaryWithAnchor`) with two
> interchangeable providers, **Reflector** (SEP-40) and **RedStone**
> (price-feed). The 2026-07-02 revision simplifies the tolerance model
> further: the two-tier `first`/`last` band and its policy-gated
> degradation path were replaced by a single band with a binary outcome,
> and oracle listing-time validation moved from the controller into
> governance. The 2026-07-10 revision added a third provider variant,
> **Xoxno** (the first-party `xoxno-oracle-adapter`, RedStone wire shape,
> listing-time-probed decimals). See Revisions for the full change record and
> `SCF_BUILD_ARCHITECTURE.md` §9 (Oracle Model) for the matching reference description.

## Context

A lending protocol depends on price honesty for each solvency-relevant
operation: borrow, withdraw with debt, liquidation, and account-threshold
migration. A stale, wrong, or manipulated single price source can create bad debt
or wrongful liquidations.

On Soroban the protocol can price an asset through independent providers:

- **Reflector** (SEP-40): CEX-aggregated and DEX-derived feeds, queried as
  spot or as a TWAP over a requested record count.
- **RedStone** (price-feed): a pull-based feed identified by a `feed_id`,
  carrying its own staleness bound and dual publish/write timestamps.
- **Xoxno** (`contracts/xoxno-oracle-adapter`): the first-party adapter,
  serving the same RedStone price-feed wire shape but backed by a
  protocol-operated N-of-M signer set — an independent trust root next to
  both external providers.

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
`OracleSourceConfig::Reflector(..)`, `OracleSourceConfig::RedStone(..)`, or
`OracleSourceConfig::Xoxno(..)`
(`OracleProviderKind::{ReflectorSep40, RedStonePriceFeed, XoxnoPriceFeed}`). A
Reflector source reads `Spot` or `Twap(records)` (`OracleReadMode`); RedStone
and Xoxno read spot. `validate_oracle_config_shape`
(`contracts/governance/src/validate/oracle_config.rs`)
enforces three diversity rules on a `PrimaryWithAnchor` pair:

1. **Different feeds.** Primary and anchor must not read the same feed (else
   `GenericError::InvalidExchangeSrc`), compared by feed identity via
   `OracleSourceConfigInput::reads_same_feed_as`: for Reflector the contract,
   asset, and read mode; for RedStone/Xoxno the contract and feed id, ignoring
   policy-only fields such as `max_stale_seconds`. RedStone and Xoxno share a
   wire ABI, so the same `(contract, feed_id)` pair counts as the same feed
   even across the two variant names; the validator likewise rejects two
   configs on the same contract and feed when only their staleness bounds
   differ.
2. **Different providers (production).** In non-`testing` builds the primary and
   anchor must come from *different* providers (else
   `GenericError::InvalidExchangeSrc`). This places the two sources behind
   independent trust boundaries: a single provider's failure (bad feed,
   signer/contract compromise, feed-mapping error) moves only one side, so the
   deviation check trips instead of both prices sliding together. A same-provider
   pair (e.g. Reflector spot vs Reflector TWAP) shares one trust boundary and is
   invalid as a production `PrimaryWithAnchor`; temporal-only diversity
   is available instead through `Single` + Reflector TWAP (which carries no
   cross-check).
3. **Different contracts (production).** The primary and anchor must not share
   a contract address, regardless of variant. The Xoxno adapter serves both the
   RedStone and the SEP-40 wire ABI, so without this guard one adapter could be
   listed once as a Reflector-shaped source and once as a Xoxno source — two
   variant names, one aggregation state, no real second opinion.

Markets still choose the specific provider feeds and which one is primary;
operators must also choose economically independent feeds, not distinct
providers alone.

**Two validation strategies.** Configured per market by
`OracleStrategy` (`common/src/types/oracle.rs`):

- `PrimaryWithAnchor`: read both sources and cross-check them against the
  market's tolerance band. An anchor is required, and in production the
  primary/anchor pair must cross providers and contracts; see the diversity
  rules above. Since only Reflector offers a non-spot read, a production
  anchored market is always a Reflector `Twap` primary with a RedStone or
  Xoxno anchor.
- `Single`: use the primary source without a cross-check. Any source shape
  qualifies, spot included: the read-mode-independent defense is the sanity
  band, which `validate_single_source_sanity_band`
  (`common/src/validation.rs`) caps at
  `MAX_SINGLE_SOURCE_SANITY_BAND_BPS` (±10% midpoint-relative) for `Single`
  markets only. `GenericError::SpotOnlyNotProductionSafe` guards the
  *anchored* path instead: a production `PrimaryWithAnchor` market rejects a
  spot primary, so a RedStone or Xoxno source can only ever be its anchor.

**A single tolerance band; binary outcome.** Each market stores one
`OraclePriceFluctuation { upper_ratio_bps, lower_ratio_bps }`
(`common/src/types/oracle.rs`), built from a single `tolerance_bps` input by
`contracts/governance/src/validate/tolerance.rs::validate_and_calculate_tolerances`,
which enforces `MIN_TOLERANCE`/`MAX_TOLERANCE` (150..2,500 BPS,
`common/src/constants/shared.rs`).

Composition happens in `contracts/controller/src/oracle/compose.rs::resolve_components`.
Under `PrimaryWithAnchor`, both the primary and the anchor are read and
freshness-checked, then
`contracts/controller/src/oracle/tolerance.rs::calculate_final_price` computes
the primary/anchor ratio and:

1. Inside the band → returns the midpoint of the two prices.
2. Outside the band → reverts `OracleError::UnsafePriceNotAllowed`.

There is no permissive fallback. An anchor is mandatory for `PrimaryWithAnchor`
at listing time (`validate_oracle_config_shape` rejects a `PrimaryWithAnchor`
config with no anchor), so a market cannot reach runtime with a missing anchor
through valid configuration; `resolve_components` panics
`OracleError::NoLastPrice` only as a defensive invariant check, not as a
degradation branch. A stale anchor reverts `OracleError::PriceFeedStale`
exactly like a stale primary: staleness on either leg is fail-closed under
both strategies.

**Sanity bounds.** After composition, `contracts/controller/src/oracle/price.rs::price_with_config`
rejects any final price outside the market's inclusive
`[min_sanity_price_wad, max_sanity_price_wad]` window, or an unconfigured
(`max_sanity_price_wad <= 0`) window, with `OracleError::SanityBoundViolated`.
The `pending_for` self-pointer sentinel and non-active market statuses are
rejected before pricing (`OracleError::OracleNotConfigured`,
`GenericError::PairNotActive`).

**Unconditional clock-skew gate.** `common::oracle::observation::check_not_future_at`
rejects oracle timestamps more than `MAX_FUTURE_SKEW_SECONDS` (60s, private to
that module) in the future with `OracleError::PriceFeedStale`, called from
`contracts/controller/src/oracle/observation.rs` on every provider read.
Staleness on the past side is bounded per source by
`common::oracle::observation::is_stale`: a Reflector source uses the
market-level `max_price_stale_seconds`; a RedStone source carries its own
`max_stale_seconds`. Both are clamped to
`[MIN_PRICE_STALE_SECONDS, MAX_PRICE_STALE_SECONDS]` (60s..86,400s,
`common/src/oracle/observation.rs`) at listing time.

**Listing-time bounds.** Governance schedules oracle configuration through the
typed `AdminOperation::ConfigureMarketOracle` and `AdminOperation::EditOracleTolerance`
operations (`contracts/governance/src/op.rs::resolve_op`), which resolve and
validate the config before scheduling controller `set_market_oracle_config`.
The controller re-checks quote-market invariants at execution before it
persists the config. The validation path
(`contracts/governance/src/validate/{oracle_config.rs, oracle_probe.rs, tolerance.rs}`)
covers: strategy/anchor coherence (`PrimaryWithAnchor` ⇔ an anchor is configured);
primary/anchor diversity: different feeds, and in production different
providers and different contracts (`GenericError::InvalidExchangeSrc`); the
production spot-primary rejection for anchored markets
(`SpotOnlyNotProductionSafe`) and the `Single` sanity-band cap
(`validate_single_source_sanity_band`); token decimals fetched from the token
contract; staleness and sanity bounds; and, per source, a live feed read plus
provider-specific checks. For a Reflector source: `base() == USD`
(`InvalidOracleBase`), oracle decimals in `[1, 18]` (`InvalidOracleDecimals`),
resolution in `[MIN_ORACLE_RESOLUTION_SECONDS, max_stale]`
(`InvalidOracleResolution`), a live `lastprice`, and, for a TWAP read, at
most `MAX_TWAP_RECORDS` (12) records with sufficient non-empty history
(`TwapInsufficientObservations`, `ReflectorHistoryEmpty`). For a RedStone
source: a per-source staleness bound, fixed `REDSTONE_DECIMALS` (the canonical
adapter exposes no `decimals()` entrypoint), and a live `read_price_data`
validated on both its package and write timestamps. For a Xoxno source: the
same feed probe as RedStone, except decimals are read live from the adapter's
SEP-40 `decimals()` at listing time and stored in the resolved config.
`AdminOperation::EditOracleTolerance` only re-validates the tolerance input
(`validate_and_calculate_tolerances`) and schedules the rewritten band; it
does not re-probe the configured sources.

## Alternatives Considered

- **Single CEX spot price with an unbounded band.** Rejected: no manipulation
  defense; a single manipulated tick can trigger a liquidation or
  under-collateralized borrow. A production `Single` market may read spot, but
  only inside a sanity band capped at ±10%
  (`validate_single_source_sanity_band`), which bounds how far any one feed
  can move the price; wider bands require `PrimaryWithAnchor`.
- **TWAP-only.** Rejected: TWAP lags real moves and exposes the protocol
  to predictable arbitrage during fast price drops; users cannot react to
  threshold migrations either. TWAP remains available as a Reflector read
  mode and as an anchor.
- **A fixed, named provider topology (the old `SpotVsTwap` / `DualOracle`
  strategies).** Rejected: baking specific source roles into named strategy
  enums removed per-market feed choice and assumed a single provider. The
  generic `primary`/`anchor` model is kept instead: each market picks its own
  provider feeds and which one is primary, with one production
  constraint layered on top: the pair must cross providers (above), so the
  cross-check spans two independent trust boundaries rather than one.
- **Two-tier tolerance band with policy-gated degradation (the 2026-06-02
  design).** Rejected on the 2026-07-02 revision: a `first`-band/`last`-band
  split with a policy-gated fallback to the primary price outside the last
  band added branching that no valid configuration exercised — a
  `PrimaryWithAnchor` market already requires an anchor at listing time, so
  the fallback path was unreachable through the listing-time validator. A
  single band with a binary revert-or-blend outcome is simpler to audit and
  matches how `PrimaryWithAnchor` markets are actually configured.
- **Manual circuit breaker (off-chain pause on deviation).** Rejected as
  the only line of defense; off-chain monitors are still useful but cannot
  be the oracle gate on their own.
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
- The midpoint blend absorbs honest deviations within the tolerance band
  without halting the protocol; outside the band the call reverts rather than
  silently preferring one side.
- An anchor is load-bearing for the lifetime of a `PrimaryWithAnchor` market:
  it is never optional at read time, so the deviation check cannot be
  bypassed by a degraded reconfiguration.
- The clock-skew gate rejects future-dated feeds in all modes, and sanity
  bounds reject absurd prices even when one provider is corroborated.

Negative / accepted costs:

- Up to two oracle cross-contract reads per priced asset per touched cache
  slot.
- Tolerance bounds, staleness limits, and sanity bounds are configuration, not
  code; they must be set conservatively per market and audited at listing time.
- A correlated outage of both required sources reverts the call. Flows that do
  not need a price avoid this by not resolving one at all, rather than by
  reading under a weaker check (ADR 0004).
- The cross-provider check defends against *one* provider failing. It does not
  catch a correlated failure that moves both providers the same way (e.g. a
  shared upstream exchange both read from), nor a manipulation small
  enough to stay inside the tolerance band. Those residual risks are bounded by
  the band width and the sanity bounds, not eliminated; selecting
  independent feeds and a conservative band remains an operator responsibility.

## Revisions

### 2026-07-10: Third provider variant — Xoxno (first-party adapter, probed decimals)

- `OracleSourceConfig{,Input}` gained a `Xoxno` variant
  (`OracleProviderKind::XoxnoPriceFeed`), carrying the same
  `RedStoneSourceConfig{,Input}` wire shape: the `xoxno-oracle-adapter`
  implements the `RedStoneMultiFeed` ABI, so the read path, transaction cache,
  and bulk prefetch are shared with RedStone. Only the provider *identity*
  differs — the adapter's trust root is a protocol-operated N-of-M signer set,
  independent of both Reflector and RedStone, so the production
  different-providers rule now accepts a Reflector-TWAP primary with a Xoxno
  anchor. Previously the adapter had to be listed under the `RedStone` variant
  and could never be paired with a real RedStone feed.
- Listing-time decimals: a Xoxno source probes the adapter's SEP-40
  `decimals()` (`reflector_decimals_call`) and stores the result, instead of
  assuming `REDSTONE_DECIMALS`; the RedStone variant keeps the constant since
  the canonical RedStone adapter has no `decimals()` entrypoint.
- Two new diversity guards close the dual-ABI hole: `reads_same_feed_as`
  treats a RedStone and a Xoxno source with equal `(contract, feed_id)` as the
  same feed, and a production `PrimaryWithAnchor` pair must not share a
  contract address across any two variants (one adapter listed under two
  variant names is one aggregation state).

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
- Final-price selection and the deviation gate keep the same shape (band →
  blend, beyond → revert), but the out-of-band behavior and the exact band
  model were revised again on 2026-07-02 (below).
- New since 2026-05-05: per-market `[min_sanity_price_wad, max_sanity_price_wad]`
  bounds (`OracleError::SanityBoundViolated`); the `check_not_future_at` clock
  gate (relocated to `common::oracle::observation`); and the primary/anchor
  diversity guard.
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

### 2026-07-02: Single tolerance band replaces the first/last split; validation moved to governance

- `OraclePriceFluctuation` dropped its two-band shape for one
  `{upper_ratio_bps, lower_ratio_bps}` pair, built from a single
  `tolerance_bps` input instead of separate `first_tolerance`/`last_tolerance`
  values. `MIN_FIRST_TOLERANCE`/`MAX_FIRST_TOLERANCE`/`MIN_LAST_TOLERANCE`/`MAX_LAST_TOLERANCE`
  were replaced by `MIN_TOLERANCE`/`MAX_TOLERANCE` (`common/src/constants/shared.rs`).
- `calculate_final_price` (`contracts/controller/src/oracle/tolerance.rs`)
  dropped the three-tier first-band/last-band/policy-gated selection for a
  binary outcome: inside the band → midpoint, outside → revert. The private
  `is_within_anchor` helper was replaced by inline `anchor_ratio_bps`/`ratio_in_band`
  checks in the same function.
- The `OraclePolicy`-gated anchor-degradation path (`allows_degraded_dual_source`,
  a `fallback_to_primary` helper) was removed; see ADR 0004's 2026-07-02
  revision.
- Listing-time validation (shape checks, live provider probes, tolerance
  calculation) moved out of the controller and into governance:
  `contracts/controller/src/oracle/validation/{config.rs, oracle.rs}` became
  `contracts/governance/src/validate/{oracle_config.rs, oracle_probe.rs, tolerance.rs}`.
  Governance validates before scheduling; the controller still re-checks
  quote-market invariants at execution.
- The oracle provider files flattened from `providers/{reflector/, redstone/}`
  directories to `providers/{reflector.rs, redstone.rs}`, and `read_source`
  was renamed `read_required_source`.
- `contracts/controller/src/positions/liquidation.rs` and
  `liquidation_math.rs` split into the
  `contracts/controller/src/positions/liquidation/` module (`apply.rs`,
  `bad_debt.rs`, `math.rs`, `mod.rs`, `plan.rs`); references below use the new
  paths.

## References

- `SCF_BUILD_ARCHITECTURE.md` §9 (Oracle Model), `architecture/INVARIANTS.md`
  §4.2 to §4.3 (Oracle Configuration, Price Resolution).
- `contracts/controller/src/oracle/price.rs::{token_price, price_with_config}`
- `contracts/controller/src/oracle/compose.rs::resolve_components`
- `contracts/controller/src/oracle/tolerance.rs::calculate_final_price`
- `contracts/controller/src/oracle/observation.rs::{redstone_observation_from_price_data, reflector_observation_from_price_data}`
- `common/src/oracle/observation.rs::{check_not_future_at, is_stale, validate_positive_price_timestamps}`
- `contracts/controller/src/oracle/providers/{mod.rs::read_required_source, reflector.rs, redstone.rs}`
- `contracts/governance/src/validate/{oracle_config.rs::validate_oracle_config_shape, oracle_probe.rs::validate_market_oracle_sources, tolerance.rs::validate_and_calculate_tolerances}`
- `contracts/governance/src/op.rs::resolve_op` (`AdminOperation::{ConfigureMarketOracle, EditOracleTolerance}`)
- `contracts/governance/src/timelock.rs::{propose, resolve_market_oracle_config, resolve_oracle_tolerance}`
- `common/src/types/oracle.rs` (`OracleStrategy`, `OracleSourceConfig`, `MarketOracleConfig`, `OraclePriceFluctuation`, `OracleReadMode`, `OracleAssetRef`)
- `common/src/constants/shared.rs` (`MIN_TOLERANCE`, `MAX_TOLERANCE`)

**Implementation Status (verified 2026-07-13):** The dual-source + single tolerance band design is implemented exactly as described.

- Resolution + band logic (midpoint or `UnsafePriceNotAllowed`, no degradation): `contracts/controller/src/oracle/{compose.rs:30, tolerance.rs:19}`.
- `PrimaryWithAnchor` diversity (different feeds via `reads_same_feed_as`; prod different providers + contracts; non-spot primary): `contracts/governance/src/validate/oracle_config.rs:19` + tests.
- Xoxno provider (distinct `XoxnoPriceFeed` variant; shared RedStone wire + prefetch; SEP-40 decimals probed at listing; dual-ABI guards): `contracts/controller/src/oracle/providers/mod.rs`, `governance/src/validate/oracle_probe.rs:123`, `common/src/types/oracle.rs:88`, `contracts/xoxno-oracle-adapter`.
- Validation split, sanity for Single, staleness, future-skew: governance probe + `common/src/validation.rs`, controller re-checks in `config/oracle.rs`.

See also `contracts/controller/tests/oracle/`, governance validate tests, adapter README, and `INVARIANTS.md` §4.2–4.3. Legacy tolerance error codes remain for ABI stability.
