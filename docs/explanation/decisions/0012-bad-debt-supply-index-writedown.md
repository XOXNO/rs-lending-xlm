# 0012. Residual bad debt is socialized through the supply index

**Status:** Accepted

## Decision

When liquidation cannot clear an account's debt and strict eligibility gates
are met, the remaining loss is written into the affected market's supply
index. Debt shares are removed and suppliers in that market bear the loss
pro-rata. The supply index has a non-zero floor.

Bad debt is not silently moved to another market, the treasury, or future
depositors.

## Guarantees

- Socialization is gated, explicit, and limited to the affected market.
- Suppliers can experience a downward index move; consumers must not assume
  monotonic supply value.
- The floor protects later conversions from a near-zero denominator.

## Auditor focus

Validate eligibility, valuation, repeated socialization, residual debt checks,
rounding, and recapitalization of the resulting shortfall.
