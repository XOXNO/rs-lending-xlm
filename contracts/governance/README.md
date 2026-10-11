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
  which it becomes ready, and an `OperationGuard` sidecar holding the state the
  op is bound to (see below). Every proposal writes one; an op without one was
  scheduled before guards existed. The guard holds values only: the key of
  each counter is read back from the scheduled function and arguments.
  `RevokeGovRole` also writes a `RoleRevocationTarget` sidecar,
  `UpgradeController` a `PendingControllerUpgrade` marker, and a canceller
  reset a `RecoveryOp` sidecar. Execute, cancel and expiry clearing remove the
  entry and its sidecars.
- The operation id is `hash_operation(target, function, args, predecessor,
  salt)`. `predecessor` is always 32 zero bytes; a new `salt` gives a
  re-proposed op a new id.
- An op executes once ready and before it expires. `execute` runs ops that
  target another contract; `execute_self` runs ops that target governance.
- `TransferGovOwnership` with `live_until_ledger` 0 cancels a nomination. Its
  guard holds the nomination epoch, which every nomination advances. At
  execution it clears the pending owner only if no nomination has been made
  since and `new_owner` is still pending; otherwise it completes as a no-op
  and is consumed. `TransferCtrlOwnership` with `live_until_ledger` 0 does the
  same on the controller leg: its guard holds the controller nomination
  epoch, which every executed controller nomination advances, and the
  controller's own zero-deadline arm is a no-op when nothing matching is
  pending. A cancel proposed before its nomination executes is therefore a
  no-op; cancel the queued nomination by id instead.
- An expired op stays `Ready` until its id is proposed again: `propose` and
  `propose_canceller_reset` remove an expired entry with the same id and its
  sidecars, emit `ExpiredOperationClearedEvent`, then schedule afresh.
- The owner must be the proposer for: ownership, upgrade, migration, delay
  (`UpdateGovDelay`), oracle, swap-aggregator, Blend approval, accumulator and
  role-grant ops, and for liquidation terms: `SetSpokeLiquidationCurve`,
  `ForceSocializeBadDebt`, `SetPositionManager(_, true)` and an
  `EditAssetInSpoke` that lowers the live threshold, raises the bonus or
  lowers the fees. A non-owner `EditAssetInSpoke` guard holds the listing's
  live threshold, bonus and fees, and execution reverts
  `ListingTermsChangedAfterProposal` unless the controller still holds them.
- Every guard holds its proposer's epoch, which advances whenever that
  address loses `PROPOSER`: `revoke_role_immediate`, an executed
  `RevokeGovRole`, or `accept_ownership` to a different address for the
  previous owner. Executing an op recorded under an earlier epoch reverts
  `NotAuthorized`. Ops other proposers queued survive a handover, and a
  self-transfer keeps the epoch.
- `Unpause` guards hold the emergency epoch, which `pause` and an executed
  `UpgradeController` advance; a stale op reverts `EmergencyEpochMismatch`.
  An `Unpause` also reverts `ControllerUpgradePending` while any
  `UpgradeController` is pending, a voided or expired one included, until it
  executes or a canceller cancels it.
- Upgrade guards hold the upgrade epoch of the contract they replace, which
  every executed upgrade of that contract advances; a stale op reverts
  `UpgradeSupersededAfterProposal`.
- `AddAssetToSpoke` guards hold the token's flag epoch, which every
  `set_spoke_asset_flags` on that token, in any spoke or hub, advances; a
  stale op reverts `AssetFlagsChangedAfterProposal`.
- `GrantGovRole` guards hold the revocation epoch of the account and role,
  which `revoke_role_immediate` and an executed `RevokeGovRole` of that pair
  advance; a stale op reverts `RoleRevokedAfterProposal`.
- A `ConfigureAssetOracle` guard holds the key's band epoch, which every
  `set_sanity_band` on the key advances, and the key's current sanity band
  read from the aggregator's `oracle` view, or that the key has none yet;
  execution reverts `OracleBandChangedAfterProposal` unless both are
  unchanged.
- A `SetPriceAggregator` guard holds the any-key band epoch, which every
  `set_sanity_band` advances, so a switch proposed before a narrowing
  reverts `OracleBandChangedAfterProposal` instead of leaving the narrowing
  behind on the old aggregator.
- An op scheduled without a guard (before the upgrade that added them) is
  held to the counters its kind binds today at 0, the value every counter had
  at that upgrade: the owner epoch for every op, plus the emergency, flag,
  revocation, band, upgrade or nomination epoch its kind binds. It is bound
  to no proposer or listing terms and an `UpgradeController` among them is not
  counted as pending: the upgrade sweep cancels and re-proposes those.

## Entrypoints

### Timelock

| Entrypoint | Caller | Does |
| --- | --- | --- |
| `propose(proposer, op, salt) -> id` | `PROPOSER` (owner for the ops above) | Schedules `op` |
| `execute(executor, target, function, args, predecessor, salt) -> Val` | anyone (see below) | Runs a ready op against `target` (not governance) |
| `execute_self(executor, op, salt)` | anyone (see below) | Runs a ready op that targets governance |
| `cancel(canceller, operation_id)` | `CANCELLER` | Vetoes a pending op. Cannot cancel a live Recovery op or a revocation of the canceller |
| `propose_canceller_reset(new_cancellers, salt) -> id` | owner | Schedules a Recovery reset of the canceller set |
| `execute_canceller_reset(executor, new_cancellers, salt)` | anyone (see below) | Runs a ready canceller reset |

With `executor = None` anyone may execute a ready op; with `Some(address)`
that address must authorize and hold `EXECUTOR`. A canceller reset uses the
Recovery delay and cannot be cancelled while it can still execute. Its guard
holds the owner epoch and each listed account's `CANCELLER` revocation epoch;
a reset carrying only the bare `RecoveryOp` marker is treated as recorded at
epoch 0. A later handover voids it, and any canceller may then cancel it to
free its id. A listed account whose `CANCELLER` role an executed
`RevokeGovRole` removed after the proposal is left out at execution; the rest
of the list is seated. `CANCELLER` holds at most `MAX_CANCELLERS`
(24) accounts, the owner included: replacing a full council writes 3N + 6 =
75 ledger entries and keeps more than 5 KB of event headroom for classic
accounts, inside soroban-sdk 28's mainnet profile (200 writes, 400 entries,
16,384 event bytes per transaction).

### Immediate (no delay)

| Entrypoint | Caller | Does |
| --- | --- | --- |
| `pause(caller)` | `GUARDIAN` | Pauses the controller and advances the emergency epoch |
| `set_spoke_asset_flags(caller, spoke_id, hub_asset, paused, frozen, no_seize)` | `GUARDIAN` | Tightens a listing's halt flags (see below) and advances the token's flag epoch |
| `create_hub(caller) -> u32`, `add_spoke(caller) -> u32` | `GUARDIAN` | Creates a controller hub or spoke |
| `set_sanity_band(caller, key, min_wad, max_wad)` | `ORACLE` | Narrows an aggregator price band; a wider band reverts `SanityBandMustTighten`. Advances the key's band epoch and the any-key band epoch, so a pending `ConfigureAssetOracle` for the key, and every pending `SetPriceAggregator`, then reverts `OracleBandChangedAfterProposal` |
| `revoke_role_immediate(account, role)` | owner | Strips `GUARDIAN`, `ORACLE`, `PROPOSER` or `EXECUTOR` from `account` (not `CANCELLER`) and advances that pair's revocation epoch; revoking `PROPOSER` also voids every op `account` proposed |

### Setup and ownership

| Entrypoint | Caller | Does |
| --- | --- | --- |
| `__constructor(admin, min_delay)` | deployer, once | Makes `admin` owner and access-control admin, grants it every default role, sets the minimum delay |
| `deploy_controller(wasm_hash) -> Address` | owner, once | Deploys the controller and records it |
| `deploy_price_aggregator(wasm_hash) -> Address` | owner, once | Deploys the aggregator, records it, and registers it with the controller if one exists |
| `accept_ownership()` | pending owner | Completes the transfer, moves the access-control admin and default roles to the new owner, and, when the owner changes, advances the owner epoch and the previous owner's proposer epoch |

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
| Global controller pause | `pause` | `AdminOperation::Unpause`, proposed after the last emergency action |
| Listing flags `paused`, `frozen`, `no_seize` | `set_spoke_asset_flags`, tighten only | `AdminOperation::RelaxSpokeAssetFlags` at the listing's flags epoch |

- A call that would clear a flag through `set_spoke_asset_flags` or
  `EditAssetInSpoke` reverts `SpokeAssetFlagRelaxation`. `EditAssetInSpoke`
  rewrites the full listing, but its flags can only keep or tighten.
- `RemoveAssetFromSpoke` removes an empty listing whether or not a flag is
  set. A removal and re-listing queued before a freeze cannot clear it: the
  freeze advances the token's flag epoch and the re-listing reverts
  `AssetFlagsChangedAfterProposal`.
- `Unpause` reverts `EmergencyEpochMismatch` if a guardian `pause` or an
  executed `UpgradeController` followed its proposal, so reopening needs an
  `Unpause` proposed after the last emergency action, and it reverts
  `ControllerUpgradePending` while a controller upgrade is pending.
- `RelaxSpokeAssetFlags` carries the `expected_epoch` from
  `get_spoke_asset_flags_epoch`. Every flag change advances the epoch, so a
  relaxation proposed before a later guardian action reverts
  `SpokeFlagsEpochMismatch`.

## References

- Errors: [`docs/reference/errors.md`](../../docs/reference/errors.md)
- Events: [`docs/reference/events.md`](../../docs/reference/events.md)
- Controller entrypoints it drives: [`contracts/controller`](../controller/README.md)
