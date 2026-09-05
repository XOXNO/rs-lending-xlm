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

Aquarius LP oracles are exempt from the smoothing check (and from the tolerance
check): an LP source is always flagged as an unsmoothed market leg and is
sole-source by construction, so the smoothing rule would reject every LP
configuration. For those oracles the sanity band is the backstop instead,
tightened by `validate_lp_sanity_band`.

The policy separates source availability from price validity: a readable source
may still be rejected for staleness, bounds, or disagreement.

### Dependent revalidation is structural (amended 2026-09-05)

When `set_oracle` changes a key, `revalidate_dependents` re-runs
`validate_asset_oracle` on every registered oracle that transitively depends
on it: cycle, depth, source count, missing dependency, staleness envelope,
smoothing, tolerance, and independence, all from registry reads. It does not
live-probe those dependents. A live probe costs one VM instantiation per
provider call, charged to the transaction memory budget, and on mainnet the
XLM key with three Aquarius LP dependents exceeded the 40 MiB limit, which
left XLM and USDC unconfigurable. The probe's only check that validation lacks
is `UnsupportedAquariusPool`; that is a property of the pool contract, not of
the changed key, and every LP oracle is hard-probed for it when it is admitted.
The changed key itself is still attested and probed.

## Guarantees

- A new source cannot become a price leg without governance review.
- Configured sources have an explicit independence and fallback policy.
- Price changes remain subject to sanity and smoothing constraints.

## Auditor focus

Review admission, source replacement, configuration changes, role authority,
and the relationship between source metadata and runtime validation.
