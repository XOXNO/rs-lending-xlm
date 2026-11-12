# Curator roles: Morpho comparison and local trust constraints

Local behavior checked against the working tree on 2026-11-06. Morpho sources
below retain the pins from the 2026-11-05 research; external code, deployed role
holders, and network state were not refreshed for this cleanup. This is a design
comparison, not evidence of current deployment configuration.

## Comparison that informs the design

| System | Authority boundary |
| --- | --- |
| Morpho Blue | Market parameters are immutable; the core has no market pause. Vault roles control allocations rather than rewriting those parameters. |
| MetaMorpho V1 / V1.1 | Cap increases wait; cap decreases are immediate. A guardian can revoke pending changes. V1 enforces a one-day to two-week timelock range. |
| Morpho Vaults V2 | Curator actions have selector-specific timelocks; sentinels revoke pending actions and reduce exposure. Timelock reductions wait under the existing rule; selectors can be abdicated. |
| Public Allocator | Flow caps constrain each allocation. This is an execution-time bound, unlike the local registry's proposal-time cap-grant check. |

The useful comparison is the separation between adding risk, reducing exposure,
and cancelling pending authority. Historical vault sizes and signer counts do
not establish the safety of the local registry or determine its required delay.

## Current local decision

Governance remains the controller owner. The curator registry has an immutable
governance address and needs explicit governance `CURATOR` and `PROPOSER`
grants. `CURATOR` is not a default constructor role. A registrar
admin holds every registrar role and can replace its admin in one step; member
accounts receive the `ROLE_MARKET`, `ROLE_SPOKE`, or `ROLE_FLAGS` bits.

| Registrar action | Boundary |
| --- | --- |
| Create hub or spoke | Immediate, `ROLE_SPOKE`; returned IDs bind to the registrar. |
| Create market | Immediate, `ROLE_MARKET`, owned hub; governance validates market parameters and requires a configured token oracle. Configuration does not prove a live price is available. |
| Lower caps | Immediate, `ROLE_MARKET`, owned spoke; governance rejects any cap increase or unchanged pair and preserves every other listing field. |
| Tighten flags | Immediate, `ROLE_FLAGS`, owned spoke; no-op and flag clearing are rejected. |
| Close listing | Immediate, `ROLE_MARKET`, hub or spoke owner; zero caps and freeze. Closing a foreign registrar's spoke also removes its grant. Other flags remain unchanged. |
| Add or edit listing, change rates, relax flags | Governance proposal and timelock; registry checks the applicable role, ownership, and foreign-hub grant before queueing. |

The governance immediate API itself enforces the cap-decrease and flag-tightening
restrictions. Arbitrary listing edits, including risk parameters and permissions,
remain timelocked. The registry does not expose an arbitrary immediate editor.

## Trust and operational constraints

- **Flags can stop recovery.** `paused` can block repayment and user exit;
  `no_seize` can block liquidation. Registrar `ROLE_FLAGS` authority therefore carries
  more power than a Morpho vault allocation role, including on foreign-hub
  listings. Freezing a listing alone leaves repayment, withdrawal, and liquidation
  available, subject to existing flags and the controller's normal checks.
- **A grant is checked when proposing.** Its grantor must still own the hub and
  be active; the grantee must own the spoke and stay within cap, permission, and
  risk ceilings. Revoking a grant, reassigning ownership, or suspending a
  registrar does not automatically cancel operations already queued in
  governance. Incident response must explicitly cancel those operations and,
  when needed, close the listing.
- **Cancellation is governance-only.** The curator keeps no record of queued
  proposals and exposes no cancel path; a registrar's queued operation is
  stopped by a governance `CANCELLER` or expires. A hub owner limits a grantee
  by revoking the grant and freezing the listing.
- **Guardian containment and owner recovery differ.** A guardian can suspend a
  registrar immediately; resumption, registrar registration, reassignment,
  admin replacement, and curator upgrade require owner-proposed sensitive
  operations. Removing the registry's `CURATOR` grant is timelocked;
  `revoke_role_immediate` only accepts `GUARDIAN` and `ORACLE`. Revoking only
  `CURATOR` stops immediate forwarding, not `PROPOSER` authority or queued ops.
- **Delays must be read from the intended deployment.** The constructor accepts
  any nonzero minimum. A decrease cannot go below 34,560 ledgers; that rule
  does not raise an existing smaller delay or change already queued readiness.
  Sensitive operations use the configured minimum or the sensitive floor,
  whichever is larger. These source rules alone do not establish an exit window.
- **Manager activation is owner-proposed.** Any proposer may queue deactivation,
  and a guardian may deactivate immediately. New proposal policy does not
  invalidate pre-existing queued operations; inspect and cancel them explicitly
  during an upgrade if their authority no longer matches policy.

Setup and incident actions are in the [curator README](../../contracts/curator/README.md).
The [governance README](../../contracts/governance/README.md) defines operation
lifecycle and role checks; the [threat model](../explanation/threat-model.md)
places these powers in the wider protocol trust boundary.

## Pinned comparison sources

These are the historical research references, not a fresh external verification.

| Repository | Pinned source |
| --- | --- |
| Morpho Blue | [Morpho.sol](https://github.com/morpho-org/morpho-blue/blob/b4af81dcd9e4e2f2d937cc57ab7f654c18f7c73c/src/Morpho.sol) |
| MetaMorpho V1 | [MetaMorpho.sol](https://github.com/morpho-org/metamorpho/blob/a99b95b2f1c7bd2bbac6757ba61e7083e3797659/src/MetaMorpho.sol) |
| MetaMorpho V1.1 | [MetaMorphoV1_1.sol](https://github.com/morpho-org/metamorpho-v1.1/blob/b6bb715896da481f6c49f2d276a0ed7dce5794a0/src/MetaMorphoV1_1.sol) |
| Morpho Vaults V2 | [VaultV2.sol](https://github.com/morpho-org/vault-v2/blob/48da66bb64af98320c34483fcc42177645e44338/src/VaultV2.sol) |
| Public Allocator | [PublicAllocator.sol](https://github.com/morpho-org/public-allocator/blob/aa324d81c2be9b25260ad865c18248e4099d274e/src/PublicAllocator.sol) |

Related documentation: [roles](https://docs.morpho.org/curate/concepts/roles),
[timelocks](https://docs.morpho.org/curate/concepts/timelock), and
[V2 emergency controls](https://docs.morpho.org/curate/tutorials-v2/emergency).
