# Governance

Timelocked admin of the lending controller and the price aggregator. It owns
both contracts; every privileged change to them goes through here.

| | |
| --- | --- |
| Owner | OZ `Ownable`, two-step |
| Roles | `PROPOSER`, `EXECUTOR`, `CANCELLER`, `GUARDIAN`, `ORACLE` |
| Client | [`interfaces/governance`](../../interfaces/governance) |

Full signatures are in `contracts/governance/src/`; the generated client drops
the `Env` argument. The role gate, delay and Recovery rules of each entrypoint
are in its rustdoc.

## Timelock model

- `propose` stores an `AdminOperation` in `OperationLedger` with the ledger at
  which it becomes ready. `RevokeGovRole` also writes a `RoleRevocationTarget`
  sidecar and a canceller reset writes a `RecoveryOp` sidecar. Execute and
  cancel remove the entry and its sidecars.
- The operation id is `hash_operation(target, function, args, predecessor,
  salt)`. `predecessor` is always 32 zero bytes; a new `salt` gives a
  re-proposed op a new id.
- An op executes once ready and before it expires. `execute` runs ops that
  target another contract; `execute_self` runs ops that target governance.
- `TransferGovOwnership` with `live_until_ledger` 0 cancels a nomination. It
  writes a `CancelledNomination` sidecar holding the nomination nonce, which
  every nomination advances. At execution it clears the pending owner only if
  no nomination has been made since and `new_owner` is still pending;
  otherwise it completes as a no-op and is consumed.
- An expired op stays `Ready` until its id is proposed again: `propose` and
  `propose_canceller_reset` remove an expired entry with the same id and its
  sidecars, emit `ExpiredOperationClearedEvent`, then schedule afresh.
- The owner must be the proposer for: ownership, upgrade, migration, delay
  (`UpdateGovDelay`), oracle, swap-aggregator, Blend approval, accumulator and
  role-grant ops. Such an op writes a `ProposalOwnerEpoch` sidecar;
  `accept_ownership` advances the owner epoch, and executing an op recorded
  under an earlier epoch reverts `NotAuthorized`.

## Entrypoints

### Timelock

| Entrypoint | Caller | Does |
| --- | --- | --- |
| `propose(proposer, op, salt) -> id` | `PROPOSER` (owner for the ops above) | Schedules `op` |
| `execute(executor, target, function, args, predecessor, salt) -> Val` | anyone (see below) | Runs a ready op against `target` (not governance) |
| `execute_self(executor, op, salt)` | anyone (see below) | Runs a ready op that targets governance |
| `cancel(canceller, operation_id)` | `CANCELLER` | Vetoes a pending op. Cannot cancel a Recovery op or a revocation of the canceller |
| `propose_canceller_reset(new_cancellers, salt) -> id` | owner | Schedules a Recovery reset of the canceller set |
| `execute_canceller_reset(executor, new_cancellers, salt)` | anyone (see below) | Runs a ready canceller reset |

With `executor = None` anyone may execute a ready op; with `Some(address)`
that address must authorize and hold `EXECUTOR`. A canceller reset uses the
Recovery delay and cannot be cancelled. It records the owner epoch like an
owner-only op; a reset carrying only the older bare `RecoveryOp` marker is
treated as recorded at epoch 0, so any later handover voids it. `CANCELLER` holds at most
`MAX_CANCELLERS` (32) accounts, the owner included, so a reset of a full
council fits one transaction's event limit.

### Immediate (no delay)

| Entrypoint | Caller | Does |
| --- | --- | --- |
| `pause(caller)` | `GUARDIAN` | Pauses the controller |
| `set_spoke_asset_flags(caller, spoke_id, hub_asset, paused, frozen, no_seize)` | `GUARDIAN` | Tightens a listing's halt flags (see below) |
| `create_hub(caller) -> u32`, `add_spoke(caller) -> u32` | `GUARDIAN` | Creates a controller hub or spoke |
| `set_sanity_band(caller, key, min_wad, max_wad)` | `ORACLE` | Narrows an aggregator price band; a wider band reverts `SanityBandMustTighten` |
| `revoke_role_immediate(account, role)` | owner | Strips `GUARDIAN` or `ORACLE` from `account` |

### Setup and ownership

| Entrypoint | Caller | Does |
| --- | --- | --- |
| `__constructor(admin, min_delay)` | deployer, once | Makes `admin` owner and access-control admin, grants it every default role, sets the minimum delay |
| `deploy_controller(wasm_hash) -> Address` | owner, once | Deploys the controller and records it |
| `deploy_price_aggregator(wasm_hash) -> Address` | owner, once | Deploys the aggregator, records it, and registers it with the controller if one exists |
| `accept_ownership()` | pending owner | Completes the transfer and moves the access-control admin and default roles to the new owner |

### Views

| View | Returns |
| --- | --- |
| `controller()`, `price_aggregator()` | Deployed addresses |
| `get_min_delay()` | Minimum delay, in ledgers |
| `get_operation_state(id)` | `OperationState` of an op |
| `get_operation_ledger(id)` | Ledger at which the op becomes ready |
| `hash_operation(target, function, args, predecessor, salt)` | The op id |
| `has_role(account, role)` | Whether `account` holds `role` |
| `resolve_oracle_tolerance(tolerance)` | Validated `OracleTolerance` bounds |
| `resolve_asset_oracle(key, oracle)` | `oracle` with `asset_decimals` filled: for `PriceKey::Token`, the stored oracle's decimals if the aggregator has one, else the token's; `0` for `PriceKey::Ref` |

## Halt controls

| Control | Immediate (`GUARDIAN`) | Clear (timelocked) |
| --- | --- | --- |
| Global controller pause | `pause` | `AdminOperation::Unpause`, proposed during the pause it ends |
| Listing flags `paused`, `frozen`, `no_seize` | `set_spoke_asset_flags`, tighten only | `AdminOperation::RelaxSpokeAssetFlags` at the listing's flags epoch |

- A call that would clear a flag through `set_spoke_asset_flags` or
  `EditAssetInSpoke` reverts `SpokeAssetFlagRelaxation`. `EditAssetInSpoke`
  rewrites the full listing, but its flags can only keep or tighten.
- `RemoveAssetFromSpoke` retains the listing's set flags. An
  `AddAssetToSpoke` for the same asset and spoke that clears one reverts
  `SpokeAssetFlagRelaxation`.
- `Unpause` and `UpgradeController` cannot be pending together; proposing
  one while the other is waiting or ready reverts
  `ConflictingOperationPending`. Execution order is otherwise free, and an
  `Unpause` run before a pending upgrade would reopen the old code.
- `Unpause` can only be proposed while the controller is paused. It writes
  an `ExecutionGuard::PauseEpoch` sidecar from `get_pause_epoch`, and
  execution reverts `PauseEpochMismatch` if a later pause advanced the
  epoch.
- `RelaxSpokeAssetFlags` carries the `expected_epoch` from
  `get_spoke_asset_flags_epoch`. Every flag change advances the epoch, so a
  relaxation proposed before a later guardian action reverts
  `SpokeFlagsEpochMismatch`.

## References

- Errors: [`docs/reference/errors.md`](../../docs/reference/errors.md)
- Events: [`docs/reference/events.md`](../../docs/reference/events.md)
- Controller entrypoints it drives: [`contracts/controller`](../controller/README.md)
