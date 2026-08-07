# 0007. Guardian ratchet: halting is immediate, resuming is timelocked

Status: Accepted

## Context

Incident response and deliberate governance pull in opposite directions. When an oracle
misbehaves, a token is compromised, or an exploit is in flight, minutes matter — the
protocol must be haltable without waiting out a timelock. But the key that can halt
instantly is also the key most exposed to compromise or coercion, and a key that can both
halt and resume can whipsaw the protocol: pause to trap users, unpause into a hostile
state, repeat. Deployment transitions carry the same risk in miniature — a freshly
deployed or freshly upgraded controller that woke up live would let a single operation
sequence skip community review.

## Decision

The `GUARDIAN` role acts immediately, but only in the tightening direction.
`contracts/governance/src/timelock/immediate.rs::pause` pauses the controller with no
delay, and `immediate.rs::set_spoke_asset_flags` sets per-listing halt flags with no
delay — both gated only by `begin_immediate` role checks. On the controller side,
`contracts/controller/src/config/asset.rs::set_spoke_asset_flags` enforces a one-way
ratchet via `contracts/controller/src/config/asset.rs::require_flag_ratchet`: `paused` and
`frozen` may only go false→true; any clearing attempt reverts
`SpokeError::SpokeAssetFlagRelaxation`.

The relaxing direction always transits the timelock. There is no
`AdminOperation::Pause` variant in `interfaces/governance/src/lib.rs::AdminOperation` at
all — pausing exists only on the immediate guardian path — while resuming requires the
timelocked `AdminOperation::Unpause`, which `contracts/governance/src/op.rs::resolve_op`
maps to the controller's `unpause`. Clearing spoke-asset flags requires the timelocked
`AdminOperation::EditAssetInSpoke`, handled by
`contracts/controller/src/config/asset.rs::edit_asset_in_spoke` as a full listing rewrite
with no ratchet: the operator states the complete desired listing, flags included.

Deployment transitions fail safe. `contracts/controller/src/governance/access.rs::init`
pauses the controller as its final act, and `governance/access.rs::upgrade` re-pauses
before swapping WASM if the controller is running — so a fresh deployment and every
upgrade land paused, and going live always transits the timelocked `Unpause`.

## Alternatives

- **Symmetric guardian powers.** Letting the guardian both halt and resume gives the
  fastest recovery from false alarms. But it converts one compromised key into full
  control of protocol liveness in both directions — the whipsaw attack — and makes the
  guardian a target worth capturing. Under the ratchet, a hostile guardian degenerates
  into denial of service, which the timelock already recovers from.
- **Timelocked pausing.** Routing halts through the timelock closes the hostile-halt
  vector entirely but forfeits rapid incident response: every live exploit would enjoy the
  full delay window. A halt is reversible and information-preserving; the asymmetry
  (instant halt, delayed resume) prices each direction by its worst case.
- **A dedicated flag-clearing entrypoint.** A narrow timelocked `clear_spoke_asset_flags`
  would avoid the hazard that `edit_asset_in_spoke` silently re-opens a halted listing
  when the operator omits the current flags. The full-rewrite design wins on surface
  area: one timelocked verb owns the entire listing state, every edit is reviewed as the
  complete post-state, and the omission hazard is answered by rustdoc on
  `edit_asset_in_spoke` plus runbook discipline rather than a second privileged path.

## Consequences

- Incident response is a single immediate call, globally (`pause`) or per listing
  (`set_spoke_asset_flags`); the threat model can scope a compromised guardian key to
  denial of service and exclude state-changing abuse (see ../threat-model.md).
- Every resume — global unpause, flag clear, first go-live after deploy or upgrade —
  costs at least one timelock delay. Outages are extended by design; the community review
  window is the point.
- Any timelocked `EditAssetInSpoke` must restate the intended `paused`/`frozen` values:
  an edit prepared before an incident and executed after it will clear flags the guardian
  set in between. Operational tooling must re-derive listing edits against current state.
- The pause taxonomy this creates (global pause, per-listing `paused`, per-listing
  `frozen`) is what liquidation gating in ADR 0008 builds on; the ratchet and
  start-paused properties are pinned by the HALT domain and the role boundaries by the
  AUTH domain (see ../../reference/invariants.md §INV-HALT, §INV-AUTH).
