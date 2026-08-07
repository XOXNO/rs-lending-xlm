# 0004. Dual-source oracle: reciprocal tolerance band, midpoint blend, Partial-means-dead

Status: Accepted

## Context

A lending protocol's solvency math is only as good as its prices. A single
oracle source is a single point of manipulation and a single point of failure;
many sources multiply read budget, configuration surface, and provider trust
relationships. Any multi-source design must answer three questions with no
ambiguity: how sources are combined, when they are considered to disagree,
and what happens when only some of them answer. Each answer is adversarial
territory — a combination rule that trusts one leg more than the other
re-creates the single-source problem, and a degraded mode that silently
drops to one source hands an attacker exactly the reduction they want.

## Decision

An asset oracle carries one or two sources
(`common/src/types/composable_oracle.rs::MIN_SOURCES` = 1,
`common/src/types/composable_oracle.rs::MAX_SOURCES` = 2).

With two readable legs, the published price is the arithmetic midpoint
`(anchor + primary) / 2`
(`contracts/price-aggregator/src/tolerance.rs::midpoint_price_or_zero`),
accepted only if the `max/min` ratio in bps stays within
`tolerance.upper_ratio_bps`
(`contracts/price-aggregator/src/tolerance.rs::within_tolerance_band`). The
tolerance pair is validated as a strict reciprocal — the lower bound must
equal `round(BPS² / upper)`
(`common/src/validation.rs::validate_oracle_tolerance`) — and governance
derives both bounds from a single bps input
(`contracts/governance/src/validate/tolerance.rs::validate_and_calculate_tolerances`),
so a mismatched pair cannot be configured.

If exactly one of two configured legs is readable, the outcome is
`Legs::Partial`, which `contracts/price-aggregator/src/engine.rs::blend`
maps to `contracts/price-aggregator/src/engine.rs::Outcome::partial` — price
zero with the deviation flag set. A half-alive dual oracle is treated as
disagreement, never as single-source fallback. The blended price must
additionally sit inside the per-asset sanity bounds
(`min_sanity_price_wad`/`max_sanity_price_wad`, checked in the engine and
validated by `common/src/validation.rs::validate_sanity_bounds`).

`certora/price-aggregator/spec/oracle_rules.rs` proves the blend stays within
`[min(leg), max(leg)]` and that Empty and Partial legs always fail.

## Alternatives

**Median-of-N aggregation (N ≥ 3).** Three or more sources with a median
tolerate one arbitrary outlier without halting. The cost is a third provider
relationship per asset, higher read budget on every price fetch, and a larger
admission/independence configuration surface — for asset listings where a
credible third independent source often does not exist. Two legs with a hard
disagreement band buy most of the manipulation resistance at the read cost
the budget can afford.

**Falling back to the surviving leg when one of two dies.** Graceful
degradation preserves liveness, but it converts an outage into a downgrade to
exactly the single-source trust model the second leg exists to prevent — and
an attacker who can silence one provider chooses which leg survives. Treating
Partial as dead makes source-silencing unprofitable.

**Weighted blend or primary-with-anchor-check.** Asymmetric schemes encode a
belief that one source is better, which re-concentrates trust and makes the
tolerance semantics direction-dependent. The symmetric midpoint plus a
reciprocal band treats both legs identically, so neither provider is a
privileged manipulation target.

## Consequences

No single source can move the published price outside the band around the
other, the blend is provably bounded by its inputs, and disagreement or
partial availability yields no price at all rather than a suspect one — see
../../reference/invariants.md §INV-ORACLE. The single-bps governance input
makes tolerance configuration mistake-resistant. The manipulation cost story
(see ../threat-model.md) rests on this: an attacker must move both
independent legs together, in the same direction, within the band.

What this makes hard: price liveness is coupled to both legs — one stale or
unreadable provider takes the asset's price down with it, and everything
downstream that consumes prices fail-closed stalls with it (that posture is
its own decision: 0005). Widening tolerance or relaxing Partial handling
weakens the manipulation bound and invalidates the Certora suite. What must
stay true: Partial must never become a fallback path, and the tolerance pair
must remain a validated reciprocal derived from one input.
