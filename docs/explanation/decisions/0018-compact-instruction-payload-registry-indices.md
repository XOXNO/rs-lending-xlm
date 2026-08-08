# 0018. Swap payloads use compact registries and indexed instructions

**Status:** Accepted

## Decision

The swap aggregator accepts a compact instruction stream over address and
amount registries. Instructions refer to registry indices, enabling
multi-venue routes without repeatedly encoding large values.

The format is an execution language. Its parser validates bounds, sequencing,
token continuity, and split accounting before dispatching a venue action.

## Guarantees

- Malformed indices and inconsistent token chains revert.
- A route cannot invent unregistered addresses or amounts.
- Split and minimum-output rules bind execution to the submitted payload.

## Auditor focus

Fuzz parser boundaries and every instruction transition. Check registry
uniqueness, index validation, continuation semantics, and accounting across
multi-hop and split routes.
