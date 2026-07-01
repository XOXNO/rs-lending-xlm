# ADR 0010: Governance Timelock For Protocol Admin

- Status: Accepted
- Date: 2026-06-13
- Revised: 2026-06-30
- Deciders: XOXNO Lending contract team

## Context

Protocol-admin actions can change market risk, oracle wiring, contract code,
central pool configuration, controller ownership, and governance itself.
Multisig custody alone does not provide an on-chain warning window.

Production ownership chain:

```text
governance owner -> governance contract -> controller contract -> central pool
```

Governance owns the controller. The controller owns the central pool. Soroban
does not allow generic self-reentry, so governance needs separate paths for
controller-targeted operations and governance-self operations.

## Decision

Embed the OpenZeppelin `stellar-governance` timelock state machine in the
governance contract and expose only typed scheduling entrypoints. Do not expose a
generic public scheduler.

## Controller-Targeted Operations

Controller-targeted proposers live in `contracts/governance/src/forward.rs`.
Each `propose_*` function:

1. renews governance instance TTL;
2. requires proposer auth;
3. checks `PROPOSER`;
4. validates typed input;
5. builds an operation targeting the controller;
6. schedules it with `get_min_delay()`.

`execute(executor, target, function, args, predecessor, salt)` executes ready
controller-targeted operations. If `executor` is `Some(address)`, that address
must authorize and hold `EXECUTOR`. If `executor` is `None`, execution is open
once ready.

Controller-targeted proposers cover hub, spoke, spoke-asset, position-limit,
minimum-borrow-collateral, token approval, central-pool deployment, market
creation, pool params, pool upgrade, controller upgrade/migration/ownership,
aggregator, accumulator, oracle config, and oracle tolerance.

## Governance-Self Operations

Governance-self operations live in `contracts/governance/src/self_timelock.rs`.
They use the same operation hash, delay, ready ledger, executor authorization,
expiry check, and state transition, but apply the mutation inline in the
governance frame.

Governance-self operations include:

- governance WASM upgrade;
- delay update;
- role grant/revoke;
- ownership transfer initiation.

## Immediate Operations

Immediate owner operations are limited to:

- deploy controller;
- pause;
- unpause;
- accept already scheduled governance ownership transfer;
- read-only views.

Testing-only immediate forwarders are behind `#[cfg(any(test, feature =
"testing"))]` and are not part of the production admin path.

## Delay And Roles

- Delay unit is ledgers.
- Mainnet minimum delay is `TIMELOCK_MIN_DELAY_LEDGERS = 34_560`.
- Ready operations expire after `TIMELOCK_OPERATION_GRACE_LEDGERS = 120_960`.
- Governance roles are `PROPOSER`, `EXECUTOR`, `CANCELLER`, and `ORACLE`.
- Delegated `EXECUTOR` and `CANCELLER` roles must be separated. Owner can retain
  recovery authority, but normal delegated accounts cannot both execute and
  cancel.

## Alternatives Considered

- **Off-chain notice only.** Rejected because it is process, not enforcement.
- **Public generic schedule.** Rejected because unvalidated calls could be
  queued.
- **Generic self-targeted operations.** Rejected because Soroban disallows the
  required self-reentry pattern.
- **Timelock emergency pause.** Rejected because pause is an emergency brake.
- **Delay reductions.** Rejected because shortening delay is itself a governance
  risk action.

## Consequences

Positive:

- Protocol-affecting controller changes have enforced delay.
- Governance-self changes also have enforced delay.
- Typed proposers validate inputs before scheduling.
- Bad proposals can be cancelled before execution.

Accepted costs:

- Governance code owns ABI encoding for each admin operation.
- Routine admin changes are slower.
- Emergency scope must stay narrow and auditable.

## References

- `contracts/governance/src/forward.rs`
- `contracts/governance/src/timelock.rs`
- `contracts/governance/src/self_timelock.rs`
- `contracts/governance/src/access.rs`
