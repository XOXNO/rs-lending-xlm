# 0014. Oracle composition is governed at admission

**Status:** Accepted

**Implemented by:** contracts/price-aggregator/src/admin.rs (`set_oracle`, `validate_asset_oracle`, `attest_sources`, `set_tolerance`, `set_sanity_band`), contracts/price-aggregator/src/validation.rs (`smoothing`, `independence`), contracts/price-aggregator/src/properties.rs (`SourceProperties`, `has_unsmoothed_market_leg`), contracts/price-aggregator/src/registry.rs (`store_oracle`), contracts/governance/src/timelock/immediate.rs (`ORACLE_ROLE`).

## Decision

Oracle sources enter the system only after governance attests their intended
use, checks required independence, and applies a smoothing policy. Runtime
reads consume this prevalidated configuration rather than making ad hoc trust
decisions.

Smoothing runs inside `validate_asset_oracle` at admission time: it constrains
which sources may be configured together and rejects a set whose legs are all
unsmoothed market feeds. It never transforms a served price, so it does not
compose with the midpoint blend (ADR-0004).

The policy separates source availability from price validity: a readable source
may still be rejected for staleness, bounds, or disagreement.

## Guarantees

- A new source cannot become a price leg without governance review.
- Configured sources have an explicit independence and fallback policy.
- Price changes remain subject to sanity and smoothing constraints.

## Auditor focus

Review admission, source replacement, configuration changes, role authority,
and the relationship between source metadata and runtime validation.
