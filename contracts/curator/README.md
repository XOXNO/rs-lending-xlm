# Curator

The curator is a registry. Third-party registrars use it to run hubs, spokes,
and markets through governance. Governance owns it and stays the controller's
owner. This contract holds the governance `CURATOR` role. It also holds
`PROPOSER` and `CANCELLER` for registrar proposals.

| | |
| --- | --- |
| Owner | Governance contract, set once by the constructor |
| Governance | Authorizes `upgrade`, `register_registrar` and every governance-only call |
| Roles | Per registrar, an admin and the `ROLE_MARKET`, `ROLE_SPOKE`, and `ROLE_FLAGS` bits |
| Client | None. Governance calls it by function name |
| Errors | `CuratorError` 801–819, in the [error reference](../../docs/reference/errors.md#curator) |

## Authorization

### Registrars

- Governance opens a registrar with owner-proposed sensitive
  `RegisterCuratorRegistrar`. It binds an existing hub or spoke with
  `ReassignCuratorHub` or `ReassignCuratorSpoke`.
- The admin holds every role bit, replaces itself with `set_admin`, and sets the
  bits of other accounts with `set_roles`. Zero bits remove an account.
- Any admin change (`set_admin` or governance `SetCuratorRegistrarAdmin`)
  voids the registrar's role bits and cap grants. The new admin resets them.
- A created hub or spoke is bound to its registrar once. Only governance moves
  it (`reassign_hub`, `reassign_spoke`).

### Registrar actions

The endpoint reference lists the authority, the
[flag rules](../../docs/reference/endpoints.md#curator-flag-rules), and the
[proposal rules](../../docs/reference/endpoints.md#curator-proposal-rules).
These actions are immediate: hub, spoke, and market creation, `tighten_flags`,
`revoke_cap_grant`, and `cancel_proposal`. Listings, edits, rate changes, and
flag relaxation go through `propose` and the timelock. No immediate editor
exists for listing fields. `create_market` needs a configured token oracle.
Governance validates the parameters. To close a listing, freeze it with
`tighten_flags`, then queue a cap-lowering edit.

`ForeignPause` stops a spoke owner from pausing another registrar's hub asset
in `tighten_flags`, a new listing, an edit, or a relaxation. A pause blocks
repayment and liquidation of that hub's debt, and the hub owner could not lift
it. Freezing and halting seizure stay available.

### Governance control

| Action | Who | Path |
| --- | --- | --- |
| Open a registrar | Owner | `RegisterCuratorRegistrar(curator, admin)`, sensitive |
| Suspend a registrar | `GUARDIAN` | `suspend_curator_registrar`, immediate |
| Resume a registrar | Owner | `SuspendCuratorRegistrar(curator, id, false)`, sensitive |
| Move a hub or spoke | Owner | `ReassignCuratorHub`, `ReassignCuratorSpoke`, sensitive |
| Replace a registrar admin | Owner | `SetCuratorRegistrarAdmin`, sensitive |
| Upgrade this contract | Owner | `UpgradeCurator`, sensitive |
| Stop immediate governance forwarding | Owner | `revoke_role_immediate(curator, CURATOR)`, immediate. Queued operations remain |

A suspended registrar's calls revert `RegistrarSuspended`, so it cannot
re-halt a listing governance relaxed. Revoking `CURATOR` alone keeps
`PROPOSER` and queued operations. To contain an incident, suspend the
registrars and cancel their pending operations.

## Entrypoints

The code is in [`src/api.rs`](src/api.rs). The
[endpoint reference](../../docs/reference/endpoints.md#curator) lists
signatures, authority, and views. Registrar calls take `caller` and `registrar`
first. `caller` must authorize.

## Cap grants

With `set_cap_grant`, a hub owner lets another registrar list its hub asset in
one spoke the grantee owns. A grant sets caps, collateral and borrow flags, and
the maximum LTV, liquidation threshold, bonus, and liquidation fee.

A foreign hub asset's listing, edit, or flag relaxation needs a live grant.
A grant is live when its grantor still owns the hub, is not suspended, and
keeps the granting admin. The hub owner limits a grantee immediately with these
calls:

- `revoke_cap_grant` deletes the grant. No new listing or edit can use it.
- `tighten_flags` with `frozen` on the grantee's listing stops new supply and
  borrowing. Withdrawals, repayments, and liquidations stay open. `paused` and
  `no_seize` keep the spoke owner's values.
- `cancel_proposal` cancels the grantee's queued listing, edit, or relaxation
  for the asset. Revoke first, then cancel.

The grant's `borrow_cap` is the hub owner's whole loss budget in that spoke.
The LTV, threshold, bonus, and fee maxima bound only the granted asset as
collateral. The grantee picks the collateral that backs borrows of the asset
and can halt its seizure immediately.

To lower a grantee's caps, first lower the grant. Then the grantee cannot queue
the caps back up. Next, queue the cap-lowering edit. The edit carries the
listing as read at queue time, so it reverts any later risk-parameter change by
the grantee.

Grants, ownership, and suspension are checked at proposal, not at execution.
Anyone can execute a queued operation between its delay and its expiry. A
later change reaches it only by `cancel_proposal` from the registrar that now
owns the named hub or spoke. The curator cancels only operations it queued. A
governance `CANCELLER` can cancel any operation. It must cancel a suspended
registrar's operation that names only its own hub and spoke.

## Storage and TTL

| Key | Tier | Holds |
| --- | --- | --- |
| `DataKey::Governance` | instance | Governance contract |
| `DataKey::NextRegistrar` | instance | Last issued registrar id. The first is 1 |
| `DataKey::RegistrarAdmin(registrar)` | persistent | `(admin, epoch)`. An admin change advances the epoch |
| `DataKey::Roles(registrar, epoch, account)` | persistent | Role bits, valid only under current admin epoch |
| `DataKey::HubRegistrar(hub_id)`, `DataKey::SpokeRegistrar(spoke_id)` | persistent | Owning registrar |
| `DataKey::CapGrant(grantee, hub_id, spoke_id, asset)` | persistent | `(grantor, epoch, CapGrant)`. Absent once the grantor's admin changes |
| `DataKey::Suspended(registrar)` | persistent | Present only while suspended |
| `DataKey::Proposal(op_id)` | persistent | Hub and spoke a queued op names. `cancel_proposal` removes it |

Authenticated calls renew the instance TTL (`renew_instance`). Persistent reads
and writes extend with `TTL_THRESHOLD_SHARED` / `TTL_BUMP_SHARED`.

## Events

Every registry write publishes one `["curator", ...]` event from its storage
setter. See [curator events](../../docs/reference/events.md#curator-events).

## Operations

```bash
make testnet deployCurator
```

```bash
make testnet grantGovRole <curator> CURATOR
```

Grant `PROPOSER` and `CANCELLER` the same way. After the timelocks, execute the
grants. Then open the first registrar:

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
  guards.rs    # integration tests: signer, cancel-role, cap-edit, and grant-boundary checks
```

The governance side is in [`timelock/immediate.rs`](../governance/src/timelock/immediate.rs).
It holds the immediate `CURATOR` and `GUARDIAN` calls and `revoke_role_immediate`.

## References

- Governance roles and timelock: [`contracts/governance/README.md`](../governance/README.md)
- Emergency power only tightens (ADR-0007): [`docs/explanation/decisions.md`](../../docs/explanation/decisions.md#adr-0007)
- Threat model: [`docs/explanation/threat-model.md`](../../docs/explanation/threat-model.md)
