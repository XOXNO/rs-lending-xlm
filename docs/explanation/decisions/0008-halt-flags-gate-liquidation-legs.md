# 0008. Pause and freeze have different exit semantics

**Status:** Accepted

## Decision

A global pause blocks risk-increasing activity while preserving exits where
possible. At listing level, frozen blocks new exposure but permits exits;
paused blocks the listing entirely, including a liquidation leg that needs that
asset.

This is intentional: pausing represents an unsafe asset state, while freezing
represents a closed-to-new-risk market.

## Guarantees

- Operators can stop new risk without necessarily trapping users.
- A paused debt asset cannot be repaid or liquidated until governance resolves
  the condition.
- The distinction is visible and testable rather than an implicit convention.

## Auditor focus

Model each verb against global pause, frozen, paused, delisted, and liquidation
entry and exit legs. Liveness consequences are security-relevant.
