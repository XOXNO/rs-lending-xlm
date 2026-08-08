# 0015. Caps are literal asset limits

**Status:** Accepted

## Decision

Supply and borrow caps are configured in native asset units and converted to
scaled shares at the live index when exposure grows. A cap of zero permits no
new exposure; it is not an unlimited sentinel. Exits reduce usage and are
never blocked by the cap.

## Guarantees

- Governance configures a human-scale value, independent of share precision.
- Interest-index changes cannot make a configured cap ambiguous.
- Cap checks cannot trap a user attempting to reduce exposure.

## Auditor focus

Test zero, boundary, index-change, multi-leg, and exit cases. Check that every
entry path consumes the same usage accounting and that no exit underflows it.
