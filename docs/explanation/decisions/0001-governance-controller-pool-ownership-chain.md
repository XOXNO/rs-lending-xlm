# 0001. Governance, controller, and pool form one ownership chain

**Status:** Accepted

## Decision

Governance owns the controller. The controller owns the pool. Users interact
with the controller, never with the pool's state-changing interface.

This separation gives one place to enforce authorization, prices, account risk,
pauses, and liquidation policy before a pool balance changes.

## Guarantees

- Pool mutations cannot bypass controller risk checks.
- Governance changes retain a single administrative root.
- The pool owner cannot be redirected through a separate ownership-transfer
  route.

## Auditor focus

Confirm the deployed ownership chain, the absence of alternative pool mutators,
and the behavior of controller upgrade paths.

## Trade-off

The controller is a critical trust boundary. A controller defect or a
misconfigured ownership handoff affects every market in the pool.
