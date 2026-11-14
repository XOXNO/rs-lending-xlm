# Curator

Registry that lets third-party registrars run hubs, spokes, and markets through
governance. Governance owns this registry and stays the controller's owner.
This contract holds the governance `CURATOR` role. It also holds `PROPOSER` to
queue registrar proposals and `CANCELLER` to cancel them.

| | |
| --- | --- |
| Owner | The governance contract, set once by the constructor; no other wallet |
| Governance | Authorizes `upgrade`, `register_registrar`, and every other governance-only call |
| Roles | Per registrar: an admin, plus `ROLE_MARKET`, `ROLE_SPOKE`, `ROLE_FLAGS` bits |
| Client | None. Governance calls this contract by function name |
| Errors | `CuratorError` codes 801–819 |

## Authorization

### Registrars

- Governance opens a registrar with the owner-proposed sensitive
  `RegisterCuratorRegistrar` and binds an existing hub or spoke id with
  `ReassignCuratorHub` and `ReassignCuratorSpoke`.
- The registrar admin holds every role bit. It replaces itself with
  `set_admin` and sets other accounts' bits with `set_roles`; zero bits remove
  an account.
- Every admin change, by `set_admin` or by the governance
  `SetCuratorRegistrarAdmin`, voids the role bits and the cap grants that the
  registrar gave. The new admin sets them again.
- A hub or spoke a registrar creates is bound to it once. Only governance moves
  it, with `reassign_hub` and `reassign_spoke`.

### Registrar actions

| Registrar action | Boundary |
| --- | --- |
| Create hub or spoke | Immediate, `ROLE_SPOKE`. `create_hub` and `add_spoke` bind the returned id to the registrar |
| Create market | Immediate, `ROLE_MARKET`, owned hub. Governance validates the market parameters and requires a configured token oracle |
| Lower caps | Timelocked `EditAssetInSpoke`, `ROLE_MARKET`. On an owned spoke, any edit; a foreign hub asset must fit its grant. On a listing of an owned hub asset in another registrar's spoke, only a supply or borrow cap no higher than the live one; every other field must equal the live listing (`NotCapLowering`) |
| Tighten flags | Immediate `tighten_flags`, `ROLE_FLAGS`. On an owned spoke, any flag, except a new `paused` on another registrar's hub asset (`ForeignPause`). On a listing of an owned hub asset in another registrar's spoke, `frozen` only. The controller rejects flag clearing (`SpokeAssetFlagRelaxation`) and a write that changes no flag (`SpokeAssetFlagsUnchanged`) |
| Close listing | No dedicated call. Freeze the listing with `tighten_flags`, then queue a cap-lowering edit |
| Add or edit listing, change rates, relax flags | `propose`, governance timelock. The registry checks the role, ownership, and foreign-hub grant before it queues the operation |
| Cancel a queued operation | Immediate `cancel_proposal`, `ROLE_MARKET` or `ROLE_FLAGS`. The registrar must own the hub or the spoke the operation names |

`propose` accepts only `AddAssetToSpoke`, `EditAssetInSpoke`,
`UpgradeLiquidityPoolParams`, and `RelaxSpokeAssetFlags`. Any other operation
reverts with `Unsupported`. Each proposal is stored under its governance
operation id with the hub and the spoke it names.

A spoke owner cannot pause a listing of a hub asset that another registrar
owns (`ForeignPause`). This applies to `tighten_flags`, a new listing, an edit,
and a relaxation. A pause blocks repayment and liquidation of that hub's debt,
and the hub owner could not lift it. Freezing and halting seizure stay
available.

The registry exposes no immediate editor for arbitrary listing fields. Risk
parameters and permissions change only through the timelock.

### Governance control

| Action | Who | Path |
| --- | --- | --- |
| Open a registrar | Owner | `RegisterCuratorRegistrar(curator, admin)`, sensitive |
| Suspend a registrar | `GUARDIAN` | `suspend_curator_registrar`, immediate |
| Resume a registrar | Owner | `SuspendCuratorRegistrar(curator, id, false)`, sensitive |
| Move a hub or spoke | Owner | `ReassignCuratorHub`, `ReassignCuratorSpoke`, sensitive |
| Replace a registrar admin | Owner | `SetCuratorRegistrarAdmin`, sensitive |
| Upgrade this contract | Owner | `UpgradeCurator`, sensitive |
| Stop immediate governance forwarding | Owner | `revoke_role_immediate(curator, CURATOR)`, immediate; queued operations remain |

A suspended registrar's calls revert with `RegistrarSuspended`, so it cannot
halt a listing again after governance relaxes it. Revoking only `CURATOR`
does not remove `PROPOSER` or cancel queued operations. To contain an
incident, suspend the registrars and cancel their pending operations.

## Entrypoints

All entrypoints are in [`src/api.rs`](src/api.rs). Registrar calls take
`caller` and `registrar` first; `caller` must authorize.

| Entrypoint | Caller | Does |
| --- | --- | --- |
| `__constructor(governance)` | deployer, once | Stores the governance contract |
| `upgrade(wasm_hash)` | governance | Replaces the Wasm; rejects a zero hash |
| `register_registrar(admin) -> u32` | governance | Opens a registrar and returns its id |
| `set_registrar_suspended(registrar, suspended)` | governance | Suspends or resumes a registrar |
| `force_set_admin(registrar, admin)` | governance | Replaces the admin; voids roles and grants |
| `reassign_hub(hub_id, registrar)`, `reassign_spoke(spoke_id, registrar)` | governance | Binds the id to a registrar, or unbinds it with `None` |
| `set_admin(caller, registrar, new_admin)` | admin | Replaces the admin; voids roles and grants |
| `set_roles(caller, registrar, account, bits)` | admin | Replaces `account`'s role bits; zero clears them |
| `create_hub(caller, registrar) -> u32` | `ROLE_SPOKE` | Creates a hub and binds it |
| `add_spoke(caller, registrar) -> u32` | `ROLE_SPOKE` | Creates a spoke and binds it |
| `create_market(caller, registrar, hub_id, params) -> Address` | `ROLE_MARKET` | Registers a market on an owned hub |
| `tighten_flags(caller, registrar, spoke_id, hub_asset, paused, frozen, no_seize)` | `ROLE_FLAGS` | Raises listing flags, as in [Registrar actions](#registrar-actions) |
| `set_cap_grant(caller, registrar, grantee, hub_id, spoke_id, asset, grant)` | `ROLE_MARKET` | Stores a cap grant for a grantee spoke |
| `revoke_cap_grant(caller, registrar, grantee, hub_id, spoke_id, asset)` | `ROLE_MARKET` | Deletes a cap grant |
| `propose(caller, registrar, op, salt) -> BytesN<32>` | `ROLE_MARKET`, or `ROLE_FLAGS` for a relaxation | Queues the operation in governance |
| `cancel_proposal(caller, registrar, op_id)` | `ROLE_MARKET` or `ROLE_FLAGS` | Cancels an operation this contract queued |
| `governance()`, `registrar_admin(registrar)`, `registrar_suspended(registrar)`, `roles(registrar, account)`, `hub_registrar(hub_id)`, `spoke_registrar(spoke_id)`, `cap_grant(grantee, hub_id, spoke_id, asset)` | anyone | Views |

## Cap grants

A hub owner lets another registrar list its hub asset in one spoke that the
grantee owns, with `set_cap_grant`. A grant sets caps, collateral and borrow
flags, and maximum LTV, liquidation threshold, bonus, and liquidation fee.

A listing, edit, or flag relaxation for a foreign hub asset needs a live
grant. A grant is live while its grantor still owns the hub, is not suspended,
and has the admin that gave the grant. The hub owner limits a grantee
immediately:

- `revoke_cap_grant` deletes the grant, so no new listing or edit can use it.
- `tighten_flags` with `frozen` on the grantee's listing of the asset stops new
  supply and borrowing there. Withdrawals, repayments, and liquidations stay
  open. The listing's `paused` and `no_seize` keep the values the spoke owner
  set.
- `cancel_proposal` cancels a listing, edit, or flag relaxation the grantee
  queued for the asset. Revoke first, then cancel what is pending.

The grant's `borrow_cap` is the hub owner's whole loss budget in that spoke.
The LTV, threshold, bonus, and fee maxima bound only the granted asset used as
collateral. The grantee chooses the collateral that backs borrowing of the
asset, and can halt seizure of that collateral at once.

To lower a grantee's caps, lower the grant first, so the grantee cannot queue
the caps back up. Then queue the cap-lowering edit. The edit carries the
listing as read when it was queued. If the grantee changes the risk parameters
before the edit executes, the edit writes back the values of that moment.

Grants, ownership, and suspension are checked when proposing and not again at
execution. Anyone can execute a queued operation once its delay has passed and
until it expires. `cancel_proposal` is how a later change reaches it: the
registrar that now owns the hub or the spoke the operation names cancels it.
The curator can cancel only operations it queued. A governance `CANCELLER` can
cancel any of them. An operation of a suspended registrar that names only its
own hub and spoke has no other registrar to cancel it, so a governance
`CANCELLER` must cancel it.

## Storage and TTL

| Key | Tier | Holds |
| --- | --- | --- |
| `DataKey::Governance` | instance | The governance contract |
| `DataKey::NextRegistrar` | instance | Last issued registrar id; the first registrar is id 1 |
| `DataKey::RegistrarAdmin(registrar)` | persistent | `(admin, epoch)`; each admin change advances the epoch |
| `DataKey::Roles(registrar, epoch, account)` | persistent | Role bits, valid only under the current admin epoch |
| `DataKey::HubRegistrar(hub_id)`, `DataKey::SpokeRegistrar(spoke_id)` | persistent | Owning registrar |
| `DataKey::CapGrant(grantee, hub_id, spoke_id, asset)` | persistent | `(grantor, epoch, CapGrant)`; reads as absent after the grantor's admin changes |
| `DataKey::Suspended(registrar)` | persistent | Present only while suspended |
| `DataKey::Proposal(op_id)` | persistent | Hub and spoke a queued operation names; removed by `cancel_proposal` |

Every authenticated call renews the instance TTL with `renew_instance`. Every
persistent read and write extends the entry with `TTL_THRESHOLD_SHARED` and
`TTL_BUMP_SHARED`.

## Events

Every registry write publishes one `["curator", ...]` event from the storage
setter that makes it. `propose` publishes `ProposedEvent` and
`cancel_proposal` publishes `CancelledEvent`, each with the registrar and the
governance op id. See [events](../../docs/reference/events.md#curator-events).

## Operations

```bash
make testnet deployCurator
```

```bash
make testnet grantGovRole <curator> CURATOR
```

Grant `PROPOSER` and `CANCELLER` the same way. Execute the grants after their
timelocks, then open the first registrar:

```bash
make testnet registerCuratorRegistrar <admin_address>
```

`make help-deploy` lists the curator verbs. `curator.wasm` is not in the
release set. Deploy and upgrade it from a local build, without `RELEASE_TAG`.

## Layout

```text
contracts/curator/src/
  lib.rs       # crate root; role bits, CuratorError, CapGrant, DataKey
  api.rs       # #[contractimpl] entrypoints
  auth.rs      # caller, admin, role, hub and spoke ownership checks
  ops.rs       # registrar actions, propose and cancel, grant checks
  storage.rs   # registry storage, admin epochs, TTL
  events.rs    # ["curator", ...] events
contracts/curator/tests/
  registry.rs  # integration tests against governance and the controller
```

Governance side: [`timelock/immediate.rs`](../governance/src/timelock/immediate.rs)
(immediate `CURATOR` and `GUARDIAN` calls, `revoke_role_immediate`).

## References

- Events: [`docs/reference/events.md`](../../docs/reference/events.md#curator-events)
- Governance roles and timelock: [`contracts/governance/README.md`](../governance/README.md)
- Flag ratchet (ADR-0007): [`docs/explanation/decisions.md`](../../docs/explanation/decisions.md#adr-0007)
- Threat model: [`docs/explanation/threat-model.md`](../../docs/explanation/threat-model.md)
