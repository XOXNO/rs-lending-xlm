# Curator

Registry that lets third-party registrars run hubs, spokes, and markets through
governance. Governance owns this registry and stays the controller's owner.
This contract holds the governance `CURATOR` role, plus `PROPOSER` to queue
registrar proposals.

| | |
| --- | --- |
| Owner | The governance contract, set once by the constructor; no other wallet |
| Governance | Authorizes `upgrade`, `register_registrar`, and every other governance-only call |
| Roles | Per registrar: an admin, plus `ROLE_MARKET`, `ROLE_SPOKE`, `ROLE_FLAGS` bits |

## Registrars

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

## What a registrar does

| Call | Role | Speed |
| --- | --- | --- |
| `create_hub`, `add_spoke` | `ROLE_SPOKE` | Immediate |
| `create_market` on an owned hub | `ROLE_MARKET` | Immediate |
| `tighten_flags` on an owned spoke, or `frozen` on a listing of an owned hub asset in any spoke | `ROLE_FLAGS` | Immediate, tightening only |
| `propose`: new listing, listing edit, rate change, flag relaxation; a cap-lowering edit of an owned hub asset in any spoke | `ROLE_MARKET` or `ROLE_FLAGS` | Governance timelock |
| `cancel_proposal` on a queued operation that names an owned hub or spoke | `ROLE_MARKET` or `ROLE_FLAGS` | Immediate |

`propose` accepts only `AddAssetToSpoke`, `EditAssetInSpoke`,
`UpgradeLiquidityPoolParams`, and `RelaxSpokeAssetFlags`. Each proposal is
stored under its governance operation id with the hub and the spoke it names.
`tighten_flags` that sets no new flag reverts with `SpokeAssetFlagsUnchanged`,
the same as a guardian write.

A spoke owner cannot pause a listing of a hub asset that another registrar
owns, through `tighten_flags` or through a listing, an edit or a relaxation
(`ForeignPause`). A
pause blocks repayment and liquidation of that hub's debt, and its owner could
not lift it. Freezing and halting seizure stay available.

## Cap grants

A hub owner lets another registrar list its hub asset in one spoke that the
grantee owns, with `set_cap_grant`: caps, collateral and borrow flags, and
maximum LTV, liquidation threshold, bonus, and liquidation fee. A listing,
edit, or flag relaxation for a foreign hub asset needs a live grant: its
grantor still owns the hub, is not suspended, and has the admin that gave the
grant. The hub owner limits a
grantee immediately:

- `revoke_cap_grant` deletes the grant, so no new listing or edit can use it.
- `tighten_flags` with `frozen` on the grantee's listing of the asset stops new
  supply and borrowing there. Withdrawals, repayments, and liquidations stay
  open; the listing's `paused` and `no_seize` are left as the spoke owner set
  them.

- `cancel_proposal` cancels a listing, edit, or flag relaxation the grantee
  queued for the asset. Revoke first, then cancel what is pending.

The grant's `borrow_cap` is the hub owner's whole loss budget in that spoke.
The LTV, threshold, bonus, and fee maxima bound only the granted asset used as
collateral. The grantee chooses the collateral that backs borrowing of the
asset, and can halt seizure of that collateral at once.

Cap changes go through the timelocked `EditAssetInSpoke`. The spoke owner
queues any edit that fits its grant. The hub owner queues an edit of a listing
of its asset in a spoke it does not own only to lower the supply cap or the
borrow cap: every other field must equal the live listing (`NotCapLowering`).
Lower the grant first, so the grantee cannot queue the caps back up. The edit
carries the listing as read when it was queued, so it writes back the risk
parameters of that moment if the grantee changes them before it executes. A
relaxation of any flag goes through the timelocked `RelaxSpokeAssetFlags`.

Grants, ownership, and suspension are checked when proposing and not again at
execution. Anyone can execute a queued operation once its delay has passed and
until it expires. `cancel_proposal` is how a later change reaches it: the
registrar that now owns the hub or the spoke the operation names cancels it.
This needs the curator to hold the governance `CANCELLER` role; the curator
can cancel only operations it queued. A governance `CANCELLER` can cancel any
of them. An operation of a suspended registrar that names only its own hub and
spoke has no other registrar to cancel it, so a governance `CANCELLER` must.

## Governance control

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
halt a listing again after governance relaxes it. Revoking only `CURATOR` does
not remove `PROPOSER` or cancel queued operations; suspend registrars and cancel
their pending operations explicitly when containing an incident.

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

Grant `PROPOSER` and `CANCELLER` the same way. Execute both grants after their timelocks, then
open the first registrar:

```bash
make testnet registerCuratorRegistrar <admin_address>
```

`make help-deploy` lists the curator verbs. `curator.wasm` is not in the release set yet, so deploy and
upgrade it from a local build.
