# Curator

Registry that lets third-party registrars run hubs, spokes, and markets through
governance. Governance owns this registry and stays the controller's owner.
This contract holds the governance `CURATOR` role, plus `PROPOSER` to queue
registrar proposals and `CANCELLER` to cancel them.

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
- A hub or spoke a registrar creates is bound to it once. Only governance moves
  it, with `reassign_hub` and `reassign_spoke`.

## What a registrar does

| Call | Role | Speed |
| --- | --- | --- |
| `create_hub`, `add_spoke` | `ROLE_SPOKE` | Immediate |
| `create_market` on an owned hub | `ROLE_MARKET` | Immediate |
| `lower_caps` on an owned spoke | `ROLE_MARKET` | Immediate, lowering only |
| `tighten_flags` on an owned spoke | `ROLE_FLAGS` | Immediate, tightening only |
| `close_listing`: zero caps and `frozen` | `ROLE_MARKET`, spoke owner or listed-hub owner | Immediate |
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
grantor still owns the hub and is not suspended. The hub owner limits a
grantee immediately:

- `revoke_cap_grant` deletes the grant.
- `cancel_grantee_proposal` cancels the grantee's queued proposals against the
  hub.
- `close_listing` zeroes the grantee's caps for the asset and freezes the
  listing. Other flags remain unchanged: existing `paused` or `no_seize`
  restrictions still apply to exits and liquidations.

Grants and suspension are checked when proposing. Revocation, suspension, or
ownership reassignment does not cancel proposals already queued in governance;
use the cancellation calls explicitly. `pending_proposal` records attribution,
not current governance state: execution or direct governance cancellation does
not clear it. Check `get_operation_state` before treating it as pending.

## Governance control

| Action | Who | Path |
| --- | --- | --- |
| Open a registrar | Owner | `RegisterCuratorRegistrar(curator, admin)`, sensitive |
| Suspend a registrar | `GUARDIAN` | `suspend_curator_registrar`, immediate |
| Resume a registrar | Owner | `SuspendCuratorRegistrar(curator, id, false)`, sensitive |
| Cancel a suspended registrar's proposals | Anyone | `cancel_suspended` |
| Move a hub or spoke | Owner | `ReassignCuratorHub`, `ReassignCuratorSpoke`, sensitive |
| Replace a registrar admin | Owner | `SetCuratorRegistrarAdmin`, sensitive |
| Upgrade this contract | Owner | `UpgradeCurator`, sensitive |
| Stop immediate governance forwarding | Owner | Timelocked revocation of this contract's `CURATOR` role; queued operations remain |

A suspended registrar's calls revert with `RegistrarSuspended`, so it cannot
halt a listing again after governance relaxes it. Revoking only `CURATOR` does
not remove `PROPOSER` or cancel queued operations; suspend registrars and cancel
their pending operations explicitly when containing an incident.

## Operations

```bash
make testnet deployCurator
```

```bash
make testnet grantGovRole <curator> CURATOR
```

Grant `PROPOSER` and `CANCELLER` the same way. Execute all three grants after
their timelocks, then open the first registrar:

```bash
make testnet registerCuratorRegistrar <admin_address>
```

`make help-deploy` lists the curator verbs. `curator.wasm` is not in the release set yet, so deploy and
upgrade it from a local build.
