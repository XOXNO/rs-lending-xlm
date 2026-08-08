# 0011. The router is untrusted; balances are authoritative

**Status:** Accepted

## Decision

Swap routes may be constructed off-chain and are treated as opaque by the
controller. The controller grants the router a narrowly scoped pull authority,
records balances, invokes it behind the reentrancy guard, and settles only
measured input and output deltas.

The router return value does not establish success. Overspending, no output,
or an invalid balance outcome reverts.

## Guarantees

- A malicious router cannot pull beyond the stated input.
- Unspent input is returned and output must be demonstrably positive.
- Every strategy finishes behind ordinary solvency checks.

## Auditor focus

Use adversarial routers: overspend, return lies, retain input, pay dust, call
back, and mutate token balances unexpectedly.
