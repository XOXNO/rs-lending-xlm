# 0011. The router is untrusted; balances are authoritative

**Status:** Accepted

**Implemented by:** contracts/controller/src/strategies/swap.rs (`swap_tokens`, `call_router_with_reentrancy_guard`, `verify_router_output`, `RouterOverspend`, `NoSwapOutput`), contracts/controller/src/risk/validation.rs.

The snapshot and settle steps were once separate helpers in a four-file
`swap/` module; they are now inline in `swap_tokens`. The mechanism is
unchanged: both sides are snapshotted before the external call, and the
output is verified against that baseline afterwards.

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
