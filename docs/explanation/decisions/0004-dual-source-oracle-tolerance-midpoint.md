# 0004. Dual-source prices require agreement

**Status:** Accepted

## Decision

A two-source price is usable only when both sources are readable, fresh, and
within a reciprocal tolerance band. The served value is their midpoint.

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
