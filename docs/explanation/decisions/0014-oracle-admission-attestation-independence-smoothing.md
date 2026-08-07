# 0014. Composable oracle sources are admitted only through write-time attestation, independence, and smoothing policy

Status: Accepted

## Context

Price sources compose: a config may hold a direct feed, a scaled source (factor
feed × a nested quote key), or an Aquarius LP valuation whose legs are
themselves priced keys (`common/src/types/composable_oracle.rs::PriceSource`).
Composition creates three hazards. First, operator misconfiguration — wrong
decimals, a staleness budget looser than the provider actually delivers, a
quote-currency mismatch — is silent until a user transaction consumes the bad
price. Second, two "independent" sources can transitively trust the same
provider contract, so the dual-source tolerance band (a separate decision)
would compare a source against itself. Third, a config whose every leg is an
unsmoothed spot market feed is manipulable inside one block. All three must be
caught before a config can influence solvency math, because the controller
consumes aggregator output as validated truth and performs no re-checks of its
own.

## Decision

Admission happens entirely at configuration-write time, inside
`contracts/price-aggregator/src/admin.rs::set_oracle`: validate the shape,
attest every provider by live probe, resolve the config once end-to-end
(`engine::probe`), and only then store it and re-validate every dependent.

Structure is bounded up front: source count is 1 or 2
(`common/src/types/composable_oracle.rs::MAX_SOURCES`), resolution depth is
capped at `common/src/types/composable_oracle.rs::MAX_RESOLUTION_DEPTH` (3),
and the engine detects cycles through a per-session resolving stack —
`session.is_resolving` in `contracts/price-aggregator/src/engine.rs` returns
`OracleCycleDetected` rather than recursing.

Attestation queries the provider itself instead of trusting the operator:
`contracts/price-aggregator/src/providers/reflector.rs::attest` requires the
Reflector base asset to be USD, its reported decimals to match the config, and
its resolution to fit the staleness budget (including the full TWAP span);
`contracts/price-aggregator/src/providers/redstone.rs::attest` pins decimals to
`REDSTONE_DECIMALS` (8); `contracts/price-aggregator/src/providers/xoxno.rs::attest`
requires the adapter's `max_submission_age` to fit inside the configured
staleness window.

Dual-source configs must declare their trust overlap:
`contracts/price-aggregator/src/validation.rs::independence` computes the two
legs' transitive provider-contract sets and rejects the config unless they are
disjoint (`IndependencePolicy::RequireDisjoint`) or exactly equal the declared
overlap (`IndependencePolicy::AllowShared`). Spot-only configs are rejected:
`contracts/price-aggregator/src/validation.rs::smoothing` panics
`SpotOnlyNotProductionSafe` unless at least one leg is TWAP-smoothed or of
`FeedNature::Fundamental` (`ProviderRef::is_unsmoothed_market_leg`).

Because keys reference keys, editing a child could strand a parent:
`contracts/price-aggregator/src/admin.rs::revalidate_dependents` walks the
dependency graph after every write and re-validates and re-probes each config
that transitively depends on the changed key.

## Alternatives

- **Read-time-only validation.** Store whatever the operator submits and let
  the engine's fail-closed read path reject bad configs when consumed. This
  needs no probe machinery, but it converts a governance mistake into a market
  outage discovered by a failing user transaction — and some mistakes (decimals
  mismatch within sanity bounds) would not fail at all, they would misprice.
  Write-time admission surfaces the error in the governance transaction itself.
- **Free-form source graphs.** No depth cap, no cycle stack, no source-count
  limit would allow richer compositions, but resolution cost becomes
  unbounded and attacker-influenceable, and cycle behavior becomes a budget
  exhaustion instead of a typed error. The fixed bounds keep worst-case read
  cost known.
- **Trusting operator-declared provider metadata.** Skipping the live `attest`
  calls simplifies deployment ordering (no live provider needed at config
  time), but decimals or staleness drift between operator belief and provider
  reality is exactly the class of silent mispricing the probe exists to catch.

## Consequences

Easy: a config that stores is a config that resolved once against live
providers with coherent decimals, staleness, independence, and smoothing —
the governance path (`ConfigureAssetOracle`) either succeeds whole or fails
loudly. Dependent configs cannot be silently invalidated by a child edit.

Hard: `set_oracle` requires every referenced provider to be live and readable
at write time, which constrains deployment ordering and makes the write cost
proportional to the dependent set. Loosening any admission rule — in
particular the smoothing rule — weakens the manipulation-resistance posture
that downstream risk math assumes (see ../threat-model.md).

Must stay true: the controller keeps consuming aggregator output without
re-validating provider metadata, so admission remains the sole gate for the
ORACLE-domain invariants (see ../../reference/invariants.md §INV-ORACLE); the
depth/cycle bounds remain fixed so read cost stays budgetable.
