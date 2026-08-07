# 0006. Timelock with typed propose, raw execute, delete-on-execute, and a non-cancellable recovery reset

Status: Accepted

## Context

Every privileged mutation in the system — controller configuration, pool deployment and
upgrade, oracle configuration, governance's own roles — flows through the governance
timelock. Several forces pull on its shape:

- Operator mistakes are far cheaper to catch when an operation is scheduled than when it
  fires days later: a malformed argument discovered at execution wastes the whole delay.
- Execution liveness must not depend on one key staying available: once the community has
  had its review window, anyone should be able to fire a Ready operation.
- The governance contract must not be able to smuggle a call to itself through the generic
  raw-execution path, or the typed validation could be bypassed.
- Retaining executed operations forever costs persistent storage and rent, and stale
  `Done` entries complicate the state machine.
- A captured or lost canceller council can veto every proposal, including the proposal to
  replace itself — the deadlock needs an escape hatch that is itself not vetoable, but slow
  enough that honest cancellers and users can react.

## Decision

Scheduling is typed and validated up front. `contracts/governance/src/timelock/lifecycle.rs::Governance::propose`
takes a typed `interfaces/governance/src/lib.rs::AdminOperation` (one variant per
administrative verb; there is deliberately no `Pause` variant — see ADR 0007) and resolves
it through `contracts/governance/src/op.rs::resolve_op`, which fully validates arguments
and derives the concrete `(target, function, args)` plus a delay tier (Standard,
Sensitive, or Recovery, floored by `contracts/governance/src/timelock/mod.rs::operation_delay`).
`contracts/governance/src/timelock/mod.rs::operation_for_admin_op` pins `predecessor` to
the all-zero hash, so the caller-chosen salt is the only uniquifier.

Execution is raw and optionally permissionless. `lifecycle.rs::Governance::execute` takes
the raw `(target, function, args, predecessor, salt)` tuple, refuses any self-targeted
operation, and accepts `executor: Option<Address>`: `Some` demands the `EXECUTOR` role
(`timelock/mod.rs::authorize_executor`), `None` makes executing a Ready operation
permissionless. Self-targeted operations go through `lifecycle.rs::Governance::execute_self`,
which re-derives the operation from the typed `AdminOperation` and applies it inline via
`contracts/governance/src/op.rs::apply_self_op` — the raw path can never call governance.
Ready operations expire after a grace window
(`timelock/mod.rs::require_operation_not_expired`).

Executed operations are deleted, not archived. `contracts/governance/src/timelock/mod.rs::finish_execute`
removes the persistent `OperationLedger` entry and both sidecars, so an executed
operation returns to `Unset` (never `Done`) and an identical tuple can be re-proposed
under the same salt.

Cancellation is a `CANCELLER` veto with two carve-outs
(`lifecycle.rs::Governance::cancel`): recovery-marked operations are permanently
non-cancellable, and a canceller cannot cancel its own pending role revocation. The
deadlock escape is `contracts/governance/src/timelock/recovery.rs::propose_canceller_reset`
(owner-only, Recovery tier — floored at 518,400 ledgers, roughly thirty days), whose
`recovery.rs::execute_canceller_reset` is permissionless once Ready.
`contracts/governance/src/access.rs::require_executor_canceller_separation` keeps the
executor and canceller roles disjoint on any non-owner account, so no single delegate
holds both the trigger and the veto.

## Alternatives

- **Typed execute mirroring typed propose.** Execution would accept the same
  `AdminOperation` and re-resolve it, sparing operators the raw-tuple bookkeeping. But
  re-resolution at execution time can produce a different tuple than was reviewed if any
  referenced state changed during the delay, and it widens the execution surface. The raw
  path executes exactly the bytes that were scheduled; convenience lives off-chain.
- **Keeping executed operations in a `Done` state.** This enables predecessor chaining
  and on-chain replay detection, at the cost of an ever-growing persistent ledger with
  rent obligations. With `predecessor` pinned to zero, chaining is unused anyway;
  delete-on-execute keeps storage bounded and the state machine two-phase.
- **Cancellable recovery operations, or an owner backdoor that skips the wait.** The
  first leaves council capture unsolvable — the captured council vetoes its own
  replacement. The second collapses the timelock's guarantee for the most sensitive
  operation in the system. The thirty-day non-cancellable floor gives users a full exit
  window while keeping recovery credible.

## Consequences

- Argument errors surface at `propose` time, not after the delay; off-chain tooling and
  runbooks schedule typed operations and only need the raw tuple at execution.
- Liveness does not hinge on any single key: any party can fire a Ready operation, and a
  Ready operation left unfired expires rather than lingering as a live threat.
- There is no predecessor ordering between operations; sequencing discipline lives in
  runbooks, and salt reuse across re-proposals is legal by design — operational tooling
  must treat the salt as the identity of a scheduling intent.
- A captured canceller council can delay but not permanently brick governance; the
  thirty-day recovery window is the accepted cost (see ../threat-model.md for the
  captured-council and compromised-owner scenarios).
- The authorization chain this preserves — timelocked owner actions, role separation,
  no self-call through raw execute — is pinned by the AUTH domain (see
  ../../reference/invariants.md §AUTH); the operation ledger's storage lifecycle falls
  under the STOR domain.
