# ADR 0003: Oracle Dual-Source Pricing With Tolerance Bands

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team
- Supersedes: none

## Context

A lending protocol depends on price honesty for every solvency-relevant
operation: borrow, withdraw with debt, liquidation, isolated-debt
accounting, and account-threshold migration. A single price source that
goes stale, mispublishes, or is manipulated translates directly into bad
debt or wrongful liquidations.

On Soroban the protocol uses Reflector as the price source. Reflector
exposes both CEX-aggregated and DEX-derived feeds, and supports both
spot and TWAP queries. Two practical risks dominate:

1. Spot manipulation: a transient spike or dump that lasts long enough to
   trigger a borrow or liquidate.
2. Source outage: a stale or missing feed that would block all activity if
   handled naively.

## Decision

Resolve every price through `oracle::token_price`
(`controller/src/oracle/mod.rs`), normalized to WAD, with the following
shape:

**Two validation strategies.** Configured per market in
`OracleProviderConfig`:

- `SpotVsTwap` (default): CEX spot vs CEX TWAP from the same Reflector
  contract. This gives temporal diversity inside one oracle provider.
- `DualOracle`: CEX TWAP vs Stellar DEX spot. This gives source diversity
  across CEX-derived and DEX-derived prices. DEX unavailability degrades to
  TWAP-only and never blocks the transaction.
- `SpotOnly`: development-only, rejected at `configure_market_oracle` in
  non-`testing` builds.

**Tolerance bands.** Each market declares two deviation bands in
`OraclePriceFluctuation`: a tight `first` band and a wider `last` band,
each with upper and lower BPS bounds. Selection in
`calculate_final_price`:

1. Aggregator inside the first band → use the safe price.
2. Inside the last band → midpoint of the two prices.
3. Outside the last band → revert if the cache is strict, otherwise fall
   back to the safe price (see ADR 0004).

**Listing-time bounds.** `configure_market_oracle` validates: token
decimals match the token contract, CEX `lastprice` exists, DEX
`lastprice` exists when `dex_oracle` is set, `twap_records ≤ 12`,
`60 ≤ max_price_stale_seconds ≤ 86_400`, and per-band tolerance bounds
(`MIN_FIRST_TOLERANCE`, `MAX_FIRST_TOLERANCE`, `MIN_LAST_TOLERANCE`,
`MAX_LAST_TOLERANCE`) with `first_tolerance < last_tolerance`.

**Unconditional clock-skew gate.** `check_not_future` rejects oracle
timestamps more than 60 seconds in the future regardless of cache mode.
Future-dated oracles are always rejected (`controller/src/oracle/mod.rs:186`).

## Alternatives Considered

- **Single CEX spot price.** Rejected: no manipulation defense; a single
  manipulated tick maps directly into a liquidation or under-collateralized
  borrow.
- **TWAP-only.** Rejected: TWAP lags real moves and exposes the protocol
  to predictable arbitrage during fast price drops; users cannot react to
  threshold migrations either.
- **Manual circuit breaker (off-chain pause on deviation).** Rejected as
  the only line of defense; off-chain monitors are still useful but cannot
  be a load-bearing oracle gate.
- **Custom oracle aggregator contract.** Rejected for launch: adds
  another upgradeable surface and another trust assumption. The chosen
  design uses Reflector directly and validates temporal or source diversity
  according to the configured strategy.

## Consequences

Positive:

- Risk-bearing decisions are gated by either same-provider temporal diversity
  (`SpotVsTwap`) or cross-source diversity (`DualOracle`), depending on market
  configuration.
- The midpoint band absorbs small honest deviations without halting the
  protocol.
- DEX unavailability degrades gracefully under `DualOracle`.
- The clock-skew gate rejects future-dated feeds in every mode, including
  permissive read paths.

Negative / accepted costs:

- Two oracle reads per priced asset per touched cache slot.
- Tolerance bounds are configuration, not code; they must be set
  conservatively per market and audited at listing time.
- A correlated outage of both sources will revert risk-increasing flows;
  this is the intended fail-safe (ADR 0004 explains how risk-decreasing
  flows degrade).

## References

- `SCF_BUILD_ARCHITECTURE.md` §9 (Oracle Pricing).
- `controller/src/oracle/mod.rs::token_price`,
  `controller/src/oracle/mod.rs::calculate_final_price`,
  `controller/src/oracle/mod.rs::check_not_future`
- `controller/src/oracle/reflector.rs`
- `controller/src/config.rs::configure_market_oracle`
- `common/src/constants.rs::{MIN_FIRST_TOLERANCE, MAX_FIRST_TOLERANCE, MIN_LAST_TOLERANCE, MAX_LAST_TOLERANCE}`
