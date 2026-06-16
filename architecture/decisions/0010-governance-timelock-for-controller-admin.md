# ADR 0010: Governance Timelock for Protocol Admin

- Status: Accepted
- Date: 2026-06-13
- Revised: 2026-06-16
- Deciders: XOXNO Lending contract team
- Supersedes: ADR 0009's original off-chain-notice posture

## Context

Protocol-admin actions can change market risk, oracle wiring, contract code,
the central pool template, controller roles, and governance itself. A multisig
alone does not give users an on-chain warning window. The protocol therefore
needs a timelock enforced by contract state, not by operator process.

The production ownership chain is:

```text
governance owner -> governance contract -> controller contract -> central pool
```

Governance owns the controller. The controller owns the central pool. That
creates a single admin choke point: governance can validate and schedule the
operations that later invoke the controller's thin owner-gated setters.

Soroban prohibits generic self-reentry. A contract cannot use
`invoke_contract` to call itself as if it were an external target. The code
therefore uses two execution paths:

- controller-targeted operations execute by cross-contract call through the
  generic timelock `execute`;
- governance-self operations execute inline through typed `execute_*`
  entrypoints after the same timelock state machine marks them ready.

## Decision

Embed the OpenZeppelin `stellar-governance` timelock storage/state machine in
the governance contract and expose only typed scheduling entrypoints. The
generic OZ scheduler is not public.

### Controller-targeted operations

Controller-targeted operations live in `contracts/governance/src/forward.rs`.
Each `propose_*` function:

1. renews governance instance TTL,
2. requires proposer auth,
3. checks the caller has `PROPOSER`,
4. validates the operation input,
5. builds an `Operation` targeting the controller,
6. schedules it with `get_min_delay()`.

The generic `execute(executor, target, function, args, predecessor, salt)`
in `contracts/governance/src/timelock.rs` executes a ready controller-targeted
operation. If `executor` is `Some(address)`, that address must authorize and
hold `EXECUTOR`. If `executor` is `None`, execution is open once the operation
is ready.

The generic `execute` rejects `target == current_contract_address()` because
generic self-reentry is not allowed.

Controller-targeted proposers cover market config, e-mode config, position
limits, minimum borrow collateral, token approval, central-pool deployment,
market creation, pool params, pool upgrade, controller role grants/revokes,
controller upgrade/migration/ownership, aggregator, accumulator, and oracle
config/tolerance.

### Governance-self operations

Governance-self operations live in
`contracts/governance/src/self_timelock.rs`. They use the same operation hash,
minimum delay, ready ledger, executor authorization, expiry check, and OZ
`set_execute_operation` state transition, but apply the mutation inline in the
same frame.

The scheduled governance-self surface is:

- `propose_governance_upgrade` / `execute_governance_upgrade`
- `propose_update_delay` / `execute_update_delay`
- `propose_grant_governance_role` / `execute_grant_governance_role`
- `propose_revoke_governance_role` / `execute_revoke_governance_role`
- `propose_transfer_gov_own` / `execute_transfer_gov_own`

`update_delay` is monotonic in production: `validate_delay_update` rejects zero
and rejects any new delay below the current delay.

### Immediate surface

The immediate owner-gated surface is small:

- `deploy_controller(wasm_hash)`: one-time bootstrap before there is a
  controller to govern.
- `pause()` / `unpause()`: emergency brakes forwarded to the controller.
- `accept_ownership()`: the pending owner accepts an already scheduled
  governance ownership transfer.
- Read-only views such as `controller`, `has_role`, `get_min_delay`,
  `get_operation_state`, `get_operation_ledger`, and `hash_operation`.

Testing-only immediate forwarders remain behind
`#[cfg(any(test, feature = "testing"))]` so the harness can configure markets
in one frame. They are not part of the production admin path.

### Delay, expiry, and roles

- The delay unit is ledgers. The mainnet invariant is
  `TIMELOCK_MIN_DELAY_LEDGERS = 34_560`, about 48 hours at 5 seconds per
  ledger.
- Ready operations expire after
  `TIMELOCK_OPERATION_GRACE_LEDGERS = 120_960`, so abandoned proposals cannot
  be executed long after their context changed.
- Governance roles are `PROPOSER`, `EXECUTOR`, `CANCELLER`, and `ORACLE`.
  The constructor grants them to the admin.
- Delegated `EXECUTOR` and `CANCELLER` roles must be separated. The owner can
  hold the full role set for recovery, but normal delegated accounts cannot
  both execute and cancel.

## Alternatives Considered

- **Off-chain notice only.** Rejected: it depends on operator process and gives
  no on-chain guarantee.
- **Expose generic OZ `schedule`.** Rejected: it could queue unvalidated calls.
  Typed proposers keep validation and ABI construction inside governance.
- **Generic self-targeted operations.** Rejected by Soroban's self-reentry
  model. Governance-self admin uses inline typed execution instead.
- **Timelock emergency pause.** Rejected: a delayed halt during an exploit or
  oracle incident is not an emergency brake.
- **Allow delay reductions.** Rejected: shortening the delay is itself a
  governance-risk action. Current code permits equal or longer delays.

## Consequences

Positive:

- Enforced on-chain state delays protocol-affecting controller changes.
- Governance-self changes are also delayed, despite Soroban's generic
  self-reentry restriction.
- Bad proposals fail at schedule time when validation is pure input validation.
- Oracle config proposal probes feeds before scheduling; controller execution
  re-checks quote-market invariants after the delay.
- Emergency pause remains immediate.
- Operation expiry prevents stale ready operations from lingering forever.

Negative / accepted costs:

- Admin changes require at least one schedule transaction and one later execute
  transaction.
- Governance code carries both controller-targeted proposers and governance-self
  typed execute paths.
- Feed availability can change between oracle proposal and execution. The
  controller read path remains fail-closed for risk-increasing use after
  activation.
- The governance owner still has recovery power and immediate pause/unpause.
  That key must be a multi-party custody setup in production.

## References

- `contracts/governance/src/timelock.rs`
- `contracts/governance/src/self_timelock.rs`
- `contracts/governance/src/forward.rs`
- `contracts/governance/src/access.rs`
- `contracts/governance/src/constants.rs`
- `interfaces/governance/src/lib.rs`
- `vendor/openzeppelin/stellar-governance/src/timelock/`
- `tests/test-harness/tests/governance/timelock.rs`
- ADR 0009 (Mainnet Launch Hardening and Operational Control)
