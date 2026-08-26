# 0004. Dual-source prices require agreement

**Status:** Accepted

**Implemented by:** contracts/price-aggregator/src/engine.rs (`compose`, `blend`, `failure`), contracts/price-aggregator/src/tolerance.rs (`within_tolerance_band`, `midpoint_price_or_zero`), common/src/validation.rs (`validate_oracle_tolerance`), common/src/oracle/observation.rs (`is_stale`, `is_future_at`).

## Decision

A two-source price is usable only when both sources are readable, fresh, and
within a tolerance band. The served value is their midpoint.

Governance stores the band as a reciprocal pair and validates that the two
halves agree: `lower_ratio_bps` must equal `BPS * BPS / upper_ratio_bps`, half
up (`validate_oracle_tolerance`). At runtime only `upper_ratio_bps` is read.
`within_tolerance_band` divides the larger of the two prices by the smaller and
compares that ratio to `upper_ratio_bps`, which is already direction-symmetric,
so `lower_ratio_bps` is never consulted on a price read.

Smoothing (ADR-0014) is an admission-time gate on which sources may be
configured, not a post-blend transform. The sanity band is likewise a pass/fail
check on the already-blended value, never a clamp. A usable two-source price is
therefore exactly the midpoint of the two accepted legs, or the read fails.

A single-source oracle is a separate, admission-gated configuration (ADR-0014):
its one leg is served directly and is still subject to the staleness,
positivity, and sanity-band checks in `Outcome::failure`, with a tighter sanity
band required by `validate_single_source_sanity_band`.

One readable leg is not a fallback price. It is an incomplete observation and
is treated as unusable. Source count is intentionally limited so the policy is
easy to reason about and to operate.

## Guarantees

- A single failed or manipulated source cannot silently become the price.
- The blended value stays inside the two accepted source values.
- Tolerance changes remain bounded by validation and governance policy.

## Auditor focus

Exercise missing, stale, future-dated, disagreeing, and boundary-equal prices;
then verify that every money-moving flow consumes the same outcome.
