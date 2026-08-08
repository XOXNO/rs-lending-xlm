# 0005. Money-moving flows fail closed on prices

**Status:** Accepted

## Decision

Every state-changing flow that depends on valuation obtains a complete,
consistent price snapshot before risk is evaluated. A missing, stale,
disagreeing, or otherwise invalid price aborts the operation.

The protocol deliberately prefers temporary unavailability to using a
potentially wrong price for borrowing, withdrawing, liquidating, or strategy
settlement.

## Guarantees

- No risk decision proceeds with a partial price set.
- One transaction observes one coherent valuation snapshot.
- Price failure cannot be converted into a permissive fallback.

## Auditor focus

Look for routes that add, remove, settle, or value positions without taking the
same fail-closed path, especially liquidation and keeper operations.
