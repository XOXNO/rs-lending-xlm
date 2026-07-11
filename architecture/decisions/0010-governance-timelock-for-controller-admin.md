# ADR 0010: Governance Timelock For Protocol Admin

- Status: Accepted
- Date: 2026-06-13
- Revised: 2026-07-02
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
governance contract and expose scheduling only through one typed entrypoint
backed by a closed `AdminOperation` enum. Do not expose a generic scheduler
that takes an arbitrary target/function/args triple from the caller.

## Controller-Targeted Operations

Every controller-targeted admin action is an `AdminOperation` variant
(`contracts/governance/src/op.rs`), and one entrypoint schedules any of them:

```text
Governance::propose(proposer, op: AdminOperation, salt) -> operation_id
```

`propose`:

1. renews governance instance TTL and requires `PROPOSER` auth
   (`begin_proposal`);
2. resolves the operation via `op::resolve_op`, which validates that variant's
   typed arguments inline (risk bounds, cap bounds, non-zero WASM hash, live
   oracle probes, and so on) and returns its `(target, function, args, DelayTier)`;
3. computes the schedule delay for that operation's `DelayTier` (see Delay And
   Roles) and calls `schedule_operation`.

`execute(executor, target, function, args, predecessor, salt)` executes ready
controller-targeted operations. If `executor` is `Some(address)`, that address
must authorize and hold `EXECUTOR`. If `executor` is `None`, execution is open
once ready.

`AdminOperation` covers hub, spoke, spoke-asset, position-limit,
minimum-borrow-collateral, token and Blend-pool approval, central-pool
template and deployment, market creation, pool params and caps, pool upgrade,
controller upgrade/migration/ownership, position manager, aggregator,
accumulator, oracle configuration, and oracle tolerance
(`contracts/governance/src/op.rs::AdminOperation`).

## Governance-Self Operations

Governance-self operations are `AdminOperation` variants too, applied inline
through a second typed entrypoint:

```text
Governance::execute_self(executor, op: AdminOperation, salt)
```

`execute_self` resolves the operation the same way as `propose`, asserts the
resolved target is the governance contract itself, then applies the mutation
inline via `op::apply_self_op`. It shares `propose`'s operation hash, delay,
ready-ledger, executor authorization, and expiry check
(`contracts/governance/src/timelock.rs`, `contracts/governance/src/access.rs`).

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
- revoke a governance role (`revoke_role_immediate`) — emergency
  de-authorization of a compromised immediate-role key must be at least as
  fast as the powers it holds; grants stay timelocked;
- accept already scheduled governance ownership transfer;
- read-only views.

Role-gated immediate operations (added 2026-07-11) bypass the timelock for
containment actions that cannot move funds or loosen risk:

- `GUARDIAN`: per-listing `paused`/`frozen` flags
  (`set_spoke_asset_flags`) and instant hub/spoke registry creation
  (`create_hub`, `add_spoke`) — new registries are inert until assets are
  listed through the timelocked path;
- `ORACLE`: sanity-band moves (`set_oracle_sanity_bounds`) — the controller
  proves the new band contains the current live price by resolving it
  under the new band, so a band move can only re-admit the market's real
  price, never smuggle a fabricated one. Asset listings, caps, risk
  params, and full oracle configs remain timelocked.

Testing-only immediate forwarders are behind `#[cfg(any(test, feature =
"testing"))]` and are not part of the production admin path.

## Delay And Roles

- Delay unit is ledgers.
- Every `AdminOperation` resolves to a `DelayTier`
  (`contracts/governance/src/timelock.rs`): `Standard` schedules at the
  current `get_min_delay()`; `Sensitive` schedules at
  `max(get_min_delay(), TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS)`. `Sensitive`
  gates governance WASM upgrade, controller WASM upgrade, pool WASM upgrade,
  and governance/controller ownership-transfer initiation
  (`contracts/governance/src/op.rs::resolve_op`); every other operation is
  `Standard`.
- Mainnet minimum delay is `TIMELOCK_MIN_DELAY_LEDGERS = 34_560`.
- The `Sensitive` floor is `TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS = 120_960`
  ledgers (~7 days), applied even when `get_min_delay()` is lower.
- Delay updates are bounded by `TIMELOCK_MAX_DELAY_LEDGERS = 241_920` ledgers
  (~14 days) and must be non-decreasing (`validate_delay_update`).
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
- **One typed `propose_*` function per admin action.** Superseded: this grew
  the governance ABI surface linearly with every new admin action and
  duplicated the validate/build-operation/schedule boilerplate per function.
  A single `propose(op: AdminOperation)` entrypoint keeps per-operation typed
  arguments (each `AdminOperation` variant carries its own typed struct,
  validated in `resolve_op`) without a combinatorial entrypoint count.
- **Timelock emergency pause.** Rejected because pause is an emergency brake.
- **Delay reductions.** Rejected because shortening delay is itself a governance
  risk action.

## Consequences

Positive:

- Protocol-affecting controller changes have enforced delay.
- Governance-self changes also have enforced delay.
- `resolve_op` validates each operation's typed inputs before scheduling.
- Sensitive operations (code upgrades, ownership transfer) get a longer floor
  delay than routine configuration changes.
- Bad proposals can be cancelled before execution.

Accepted costs:

- Governance code owns ABI encoding for each admin operation, centralized in
  one `resolve_op` match rather than spread across per-operation functions.
- Routine admin changes are slower.
- Emergency scope must stay narrow and auditable.

## References

- `contracts/governance/src/op.rs` (`AdminOperation`, `resolve_op`, `apply_self_op`)
- `contracts/governance/src/timelock.rs` (`propose`, `execute`, `execute_self`, `cancel`, `DelayTier`, `operation_delay`)
- `contracts/governance/src/access.rs`
- `contracts/governance/src/constants.rs`
