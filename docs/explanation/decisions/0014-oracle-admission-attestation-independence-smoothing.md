# 0014. Oracle composition is governed at admission

**Status:** Accepted

## Decision

Oracle sources enter the system only after governance attests their intended
use, checks required independence, and applies a smoothing policy. Runtime
reads consume this prevalidated configuration rather than making ad hoc trust
decisions.

The policy separates source availability from price validity: a readable source
may still be rejected for staleness, bounds, or disagreement.

## Guarantees

- A new source cannot become a price leg without governance review.
- Configured sources have an explicit independence and fallback policy.
- Price changes remain subject to sanity and smoothing constraints.

## Auditor focus

Review admission, source replacement, configuration changes, role authority,
and the relationship between source metadata and runtime validation.
