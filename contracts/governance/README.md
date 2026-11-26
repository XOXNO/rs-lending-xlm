# Governance

Governance is the timelocked owner of the controller and the price aggregator.
Every privileged change to them goes through this contract.

| | |
| --- | --- |
| Owner | OZ `Ownable`, two-step |
| Roles | `PROPOSER`, `CANCELLER`, `GUARDIAN`, `ORACLE`, and grantable `CURATOR_ROLE` |
| Client | [`interfaces/governance`](../../interfaces/governance). It drops `Env` |

The [endpoint reference](../../docs/reference/endpoints.md#governance) lists
signatures and role gates. Each entrypoint's rustdoc states the delay and
Recovery rules.

## Timelock model

- Sensitive operations wait at least 120960 ledgers (about seven days). The
  floor is compiled in, and a longer configured minimum delay wins. The
  `short-timelock` feature lowers the floor to 12 ledgers for the end-to-end
  build only. No deploy or release artifact enables it. Its stamp makes the
  candidate check refuse it.
- `propose` stores the ready ledger in `OperationLedger`. `RevokeGovRole` adds
  a `RoleRevocationTarget` sidecar. A canceller reset adds a `RecoveryOp`
  sidecar.
- The operation id is `hash_operation(target, function, args, predecessor, salt)`.
  `predecessor` is always 32 zero bytes. The salt only separates concurrently
  pending ops. Nothing records that an id ran, so the same op and salt can run
  again after a new delay. Only the delay and the cancellers stop a repeat.
- An op runs between ready and expiry. `execute` runs ops for other targets.
  `execute_self` runs ops for governance. Execution takes no caller and checks
  no role. The delay and the cancellers protect it, not executor identity.
- Owner-only proposals are listed under `propose` in the endpoint reference.
  Any proposer can schedule a position-manager deactivation. A GUARDIAN can
  also apply it immediately.
- A canceller reset (`propose_canceller_reset`) uses the Recovery delay and
  cannot be cancelled.

## Entrypoint notes

- `__constructor(admin, min_delay)` makes `admin` owner and access-control
  admin with every default role. `CURATOR_ROLE` is not a default role.
- `deploy_price_aggregator` also registers the aggregator with an existing
  controller.
- `accept_ownership` moves access-control admin and default roles to the new
  owner.
- To resume a registrar after `suspend_curator_registrar`, the owner proposes
  the sensitive `SuspendCuratorRegistrar`. Position-manager activation and Blend
  approval stay timelocked.
- `set_sanity_band` reverts `SanityBandMustTighten` on a wider band.
- `resolve_asset_oracle` fills `asset_decimals`. For `PriceKey::Token`, it uses
  the stored oracle's decimals if the aggregator has one, and the token's
  decimals otherwise. For `PriceKey::Ref`, it uses `0`.

## Halt controls

| Control | Immediate (`GUARDIAN`) | Clear (timelocked) |
| --- | --- | --- |
| Global controller pause | `pause` | `AdminOperation::Unpause` |
| Listing flags `paused`, `frozen`, `no_seize` | `set_spoke_asset_flags`, tighten only | `AdminOperation::RelaxSpokeAssetFlags` at the listing's flags epoch |

`set_spoke_asset_flags` also accepts `CURATOR_ROLE`. Clearing a flag through it
or `EditAssetInSpoke` (which rewrites the whole listing) reverts
`SpokeAssetFlagRelaxation`. Each flag change advances the epoch, so a
relaxation proposed before a later guardian action reverts
`SpokeFlagsEpochMismatch`.

## Storage and TTL

| Key | Tier | Holds |
| --- | --- | --- |
| `GovernanceKey::Controller`, `GovernanceKey::PriceAggregator` | instance | Deployed addresses |
| `TimelockStorageKey::MinDelay` | instance | Configured minimum delay, in ledgers |
| `TimelockStorageKey::OperationLedger(id)` | persistent | Ledger at which a pending op becomes ready |
| `GovernanceKey::RoleRevocationTarget(id)` | persistent | Account a pending `RevokeGovRole` targets |
| `GovernanceKey::RecoveryOp(id)` | persistent | Marker for a pending canceller reset |
| OpenZeppelin `stellar-access` keys | instance, persistent | Owner, pending owner, admin, role members |

State-writing entrypoints renew the instance TTL (`renew_instance`,
`TTL_THRESHOLD_INSTANCE`, `TTL_BUMP_INSTANCE`). Sidecars extend with
`TTL_THRESHOLD_SHARED` / `TTL_BUMP_SHARED` when written. OpenZeppelin extends
`OperationLedger` by 30 days on each read. `execute`, `execute_self`,
`execute_canceller_reset` and `cancel` remove the entry and its sidecars.

## Events

Timelock, ownership, and role changes publish stock OpenZeppelin events
([governance events](../../docs/reference/events.md#governance-events)).

## Layout

```text
contracts/governance/src/
  lib.rs          # crate root; Governance contract
  api.rs          # GovernanceInterface entrypoints
  access.rs       # roles, owner and admin transfer, constructor
  deploy.rs       # one-time controller and aggregator deployment
  op.rs           # AdminOperation to target, function, args, delay tier
  constants.rs    # timelock delay floors and grace period
  storage.rs      # addresses and per-operation sidecars
  events.rs       # deploy events
  timelock/       # lifecycle, immediate, recovery; testing (feature-gated)
  validate/       # asset onboarding and oracle tolerance checks
```

## References

- Errors: [`docs/reference/errors.md`](../../docs/reference/errors.md)
- Curator registry: [`contracts/curator`](../curator/README.md)
- Controller entrypoints it calls: [`contracts/controller`](../controller/README.md)
