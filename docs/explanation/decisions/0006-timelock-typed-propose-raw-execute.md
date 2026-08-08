# 0006. Timelocked governance separates scheduling from execution

**Status:** Accepted

## Decision

Governance validates a typed administrative operation when it is proposed,
records its executable payload and delay tier, then executes only after the
operation is ready. Execution can be permissionless when no executor is named.

Completed operations are removed. Recovery operations have a dedicated,
non-cancellable reset path so a lost canceller cannot permanently deadlock
governance.

## Guarantees

- Invalid administrative changes fail before they enter the queue.
- The approved payload, salt, and readiness period bind execution.
- Immediate emergency powers remain narrower than ordinary governance.

## Auditor focus

Review operation identity, delay calculation, expiry, cancellation, replay
after deletion, self-governance calls, and role separation.
