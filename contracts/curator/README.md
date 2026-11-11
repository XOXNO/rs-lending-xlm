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
| `propose`: new listing, listing edit, rate change, flag relaxation | `ROLE_MARKET` or `ROLE_FLAGS` | Governance timelock |

`propose` accepts only `AddAssetToSpoke`, `EditAssetInSpoke`,
`UpgradeLiquidityPoolParams`, and `RelaxSpokeAssetFlags`. Each proposal is
recorded with its registrar and hub. `tighten_flags` that sets no new flag
reverts with `SpokeAssetFlagsUnchanged`, the same as a guardian write.

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

Cap changes go through the timelocked `EditAssetInSpoke`; a relaxation of any
flag goes through the timelocked `RelaxSpokeAssetFlags`. Grants and suspension
are checked when proposing. A proposal already queued in governance is stopped
only by a governance `CANCELLER`, or by letting it expire; the curator keeps no
record of queued proposals.

## Governance control

| Action | Who | Path |
| --- | --- | --- |
| Open a registrar | Owner | `RegisterCuratorRegistrar(curator, admin)`, sensitive |
| Suspend a registrar | `GUARDIAN` | `suspend_curator_registrar`, immediate |
| Resume a registrar | Owner | `SuspendCuratorRegistrar(curator, id, false)`, sensitive |
| Move a hub or spoke | Owner | `ReassignCuratorHub`, `ReassignCuratorSpoke`, sensitive |
| Replace a registrar admin | Owner | `SetCuratorRegistrarAdmin`, sensitive |
| Upgrade this contract | Owner | `UpgradeCurator`, sensitive |
| Stop immediate governance forwarding | Owner | Timelocked revocation of this contract's `CURATOR` role; queued operations remain |

A suspended registrar's calls revert with `RegistrarSuspended`, so it cannot
halt a listing again after governance relaxes it. Revoking only `CURATOR` does
not remove `PROPOSER` or cancel queued operations; suspend registrars and cancel
their pending operations explicitly when containing an incident.

## Events

Every registry write publishes one `["curator", ...]` event from the storage
setter that makes it, and `propose` publishes `ProposedEvent` with the
registrar and the governance op id. See [events](../../docs/reference/events.md#curator-events).

## Operations

```bash
make testnet deployCurator
```

```bash
make testnet grantGovRole <curator> CURATOR
```

Grant `PROPOSER` the same way. Execute both grants after their timelocks, then
open the first registrar:

```bash
make testnet registerCuratorRegistrar <admin_address>
```

`make help-deploy` lists the curator verbs. `curator.wasm` is not in the release set yet, so deploy and
upgrade it from a local build.
