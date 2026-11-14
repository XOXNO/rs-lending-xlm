# Curator

Registry through which third-party registrars run hubs, spokes and markets via
governance. Governance owns it and stays the controller's owner. This contract
holds the governance `CURATOR` role, plus `PROPOSER` and `CANCELLER` for
registrar proposals.

| | |
| --- | --- |
| Owner | Governance contract, set once by the constructor |
| Governance | Authorizes `upgrade`, `register_registrar` and every governance-only call |
| Roles | Per registrar: an admin, plus `ROLE_MARKET`, `ROLE_SPOKE`, `ROLE_FLAGS` bits |
| Client | None; governance calls by function name |
| Errors | `CuratorError` 801–819, in the [error reference](../../docs/reference/errors.md#curator) |

## Authorization

### Registrars

- Governance opens a registrar with owner-proposed sensitive
  `RegisterCuratorRegistrar`, and binds an existing hub or spoke with
  `ReassignCuratorHub` / `ReassignCuratorSpoke`.
- The admin holds every role bit, replaces itself with `set_admin` and sets
  others' bits with `set_roles`; zero bits remove an account.
- Any admin change (`set_admin` or governance `SetCuratorRegistrarAdmin`)
  voids the registrar's role bits and cap grants; the new admin resets them.
- A created hub or spoke is bound to its registrar once; only governance moves
  it (`reassign_hub`, `reassign_spoke`).

### Registrar actions

Authority, [flag rules](../../docs/reference/endpoints.md#curator-flag-rules)
and [proposal rules](../../docs/reference/endpoints.md#curator-proposal-rules)
are in the endpoint reference. Immediate: hub, spoke and market creation,
`tighten_flags`, `revoke_cap_grant`, `cancel_proposal`. Listings, edits, rate
changes and flag relaxation go through `propose` and the timelock; no
immediate editor exists for listing fields. `create_market` needs a configured
token oracle; governance validates the parameters. To close a listing, freeze
it with `tighten_flags`, then queue a cap-lowering edit.

`ForeignPause` stops a spoke owner pausing another registrar's hub asset, in
`tighten_flags`, a new listing, an edit or a relaxation: a pause blocks
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
| Stop immediate governance forwarding | Owner | `revoke_role_immediate(curator, CURATOR)`, immediate; queued operations remain |

A suspended registrar's calls revert `RegistrarSuspended`, so it cannot
re-halt a listing governance relaxed. Revoking `CURATOR` alone keeps
`PROPOSER` and queued operations. To contain an incident, suspend the
registrars and cancel their pending operations.

## Entrypoints

Code in [`src/api.rs`](src/api.rs); signatures, authority and views in the
[endpoint reference](../../docs/reference/endpoints.md#curator). Registrar
calls take `caller` and `registrar` first; `caller` must authorize.

## Cap grants

With `set_cap_grant`, a hub owner lets another registrar list its hub asset in
one spoke the grantee owns. A grant sets caps, collateral and borrow flags, and
maximum LTV, liquidation threshold, bonus and liquidation fee.

A foreign hub asset's listing, edit or flag relaxation needs a live grant:
its grantor still owns the hub, is not suspended, and keeps the granting
admin. The hub owner limits a grantee immediately:

- `revoke_cap_grant` deletes the grant; no new listing or edit can use it.
- `tighten_flags` with `frozen` on the grantee's listing stops new supply and
  borrowing; withdrawals, repayments and liquidations stay open. `paused` and
  `no_seize` keep the spoke owner's values.
- `cancel_proposal` cancels the grantee's queued listing, edit or relaxation
  for the asset. Revoke first, then cancel.

The grant's `borrow_cap` is the hub owner's whole loss budget in that spoke.
The LTV, threshold, bonus and fee maxima bound only the granted asset as
collateral. The grantee picks the collateral backing borrows of the asset and
can halt its seizure at once.

To lower a grantee's caps, lower the grant first so the grantee cannot queue
them back up, then queue the cap-lowering edit. The edit carries the listing
as read at queue time, so it reverts any later risk-parameter change by the
grantee.

Grants, ownership and suspension are checked at proposal, not at execution.
Anyone can execute a queued operation between its delay and its expiry; a
later change reaches it only by `cancel_proposal` from the registrar now owning
the named hub or spoke. The curator cancels only operations it queued; a
governance `CANCELLER` can cancel any, and must cancel a suspended registrar's
operation that names only its own hub and spoke.

## Storage and TTL

| Key | Tier | Holds |
| --- | --- | --- |
| `DataKey::Governance` | instance | Governance contract |
| `DataKey::NextRegistrar` | instance | Last issued registrar id; first is 1 |
| `DataKey::RegistrarAdmin(registrar)` | persistent | `(admin, epoch)`; admin change advances epoch |
| `DataKey::Roles(registrar, epoch, account)` | persistent | Role bits, valid only under current admin epoch |
| `DataKey::HubRegistrar(hub_id)`, `DataKey::SpokeRegistrar(spoke_id)` | persistent | Owning registrar |
| `DataKey::CapGrant(grantee, hub_id, spoke_id, asset)` | persistent | `(grantor, epoch, CapGrant)`; absent once grantor's admin changes |
| `DataKey::Suspended(registrar)` | persistent | Present only while suspended |
| `DataKey::Proposal(op_id)` | persistent | Hub and spoke a queued op names; removed by `cancel_proposal` |

Authenticated calls renew the instance TTL (`renew_instance`); persistent reads
and writes extend with `TTL_THRESHOLD_SHARED` / `TTL_BUMP_SHARED`.

## Events

Every registry write publishes one `["curator", ...]` event from its storage
setter; see [curator events](../../docs/reference/events.md#curator-events).

## Operations

```bash
make testnet deployCurator
```

```bash
make testnet grantGovRole <curator> CURATOR
```

Grant `PROPOSER` and `CANCELLER` likewise. After the timelocks, execute the
grants, then open the first registrar:

```bash
make testnet registerCuratorRegistrar <admin_address>
```

`make help-deploy` lists the curator verbs. `curator.wasm` is not in the
release set: deploy and upgrade from a local build, without `RELEASE_TAG`.

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
(immediate `CURATOR` / `GUARDIAN` calls, `revoke_role_immediate`).

## References

- Governance roles and timelock: [`contracts/governance/README.md`](../governance/README.md)
- Flag ratchet (ADR-0007): [`docs/explanation/decisions.md`](../../docs/explanation/decisions.md#adr-0007)
- Threat model: [`docs/explanation/threat-model.md`](../../docs/explanation/threat-model.md)
