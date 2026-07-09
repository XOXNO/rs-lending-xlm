# ADR 0011: Pause And Freeze Matrix

- Status: Accepted
- Date: 2026-07-02
- Deciders: XOXNO Lending contract team

## Context

The protocol has three independent halt controls with different scopes:

- a global controller pause (OpenZeppelin `pausable`, enforced by
  `#[when_not_paused]` on controller entrypoints);
- a per-spoke-asset `paused` flag on the spoke listing;
- a per-spoke-asset `frozen` flag on the spoke listing.

The correct incident response differs by failure mode: a protocol-wide
solvency scare, an oracle incident, and a single-market incident each need a
different switch. Operators need an explicit matrix of what each layer blocks
so the wrong switch is not pulled under pressure.

## Decision

Keep three layers with deliberately different coverage.

## Layer 1: Global Pause

`#[when_not_paused]` gates the risk-increasing and index-mutating
entrypoints: `supply`, `borrow`, `multiply`, `swap_debt`, `swap_collateral`,
`repay_debt_with_collateral`, `migrate_from_blend`, `flash_loan`,
`update_indexes`, `claim_revenue`, `add_rewards`, and
`update_account_threshold`.

It does NOT gate `withdraw`, `repay`, `liquidate`, or `clean_bad_debt`:
exits and de-risking stay live during a global pause.

- The constructor deploys the controller paused.
- `upgrade` re-pauses before swapping code.
- `pause`/`unpause` are immediate governance-owner actions, never timelocked
  (ADR 0010).

## Layer 2: Per-Spoke-Asset Paused

The `paused` flag blocks `supply`, `borrow`, `withdraw`, and `repay` for
that (spoke, hub-asset) — including exits. Strategy flows route through the
same supply/borrow/withdraw/repay processing, so the flag covers them too.

## Layer 3: Per-Spoke-Asset Frozen

The `frozen` flag blocks only new `supply` and `borrow`; `withdraw` and
`repay` stay live. It winds a listing down without trapping users.

## Setting The Per-Asset Flags

Both flags travel on `SpokeAssetArgs` and are written by
`add_asset_to_spoke` / `edit_asset_in_spoke`; every edit states them
explicitly, so a routine parameter edit cannot silently clear an active
flag. Flag changes therefore ride the same governance timelock as the rest
of the listing edit — the per-asset flags are planned wind-down and
containment tools, while the instant incident brake remains the layer-1
global pause; an oracle incident on one asset is contained per spoke via
the layer-2 `paused` flag.

## Liquidations

`liquidate` and `clean_bad_debt` are never blocked — not by the global
pause and not by the per-asset flags — preserving the solvency defense in
every incident mode.

## Alternatives Considered

- **Pause everything globally, including exits.** Rejected because trapping
  users inside a paused protocol converts an incident into a bank-run
  incentive and blocks de-risking exactly when it matters.
- **Gate liquidations behind the pause.** Rejected because a delayed
  liquidation window mints bad debt; liquidation must outlive every halt
  control.
- **A single per-asset flag.** Rejected because "stop everything on this
  asset" (paused) and "wind this listing down" (frozen) are different
  operations with different exit semantics.

## Consequences

Positive:

- Exits and liquidations survive a global pause, so a pause cannot trap
  users or suspend the solvency defense.
- Per-asset flags isolate a single-market incident without halting the
  protocol.

Explicit limitation: the global pause is NOT an oracle killswitch.
`withdraw` with outstanding debt and `liquidate` still read oracles while
globally paused. In an oracle incident the correct tool is the per-asset
`paused` flag, which stops the flows that would consume the bad price.
There is deliberately no admin op that removes an asset's oracle entry
outright: that would brick every live position referencing the asset
(repay only) instead of containing exposure via the flag above.

Accepted costs:

- Operators must pick the right layer; the matrix above is the runbook
  reference.
- A per-asset `paused` flag blocks exits for that asset, so it is a stronger
  intervention than the global pause for the affected market.

## References

- `contracts/controller/src/governance/access.rs`
- `contracts/controller/src/positions/mod.rs` (`enforce_spoke_asset_flags`)
- [ADR 0009](./0009-mainnet-launch-hardening-and-operational-control.md)
- [ADR 0010](./0010-governance-timelock-for-controller-admin.md)
