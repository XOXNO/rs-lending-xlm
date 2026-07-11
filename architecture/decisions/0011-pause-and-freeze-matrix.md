# ADR 0011: Pause And Freeze Matrix

- Status: Accepted (amended 2026-07-11, see Addendum)
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

`liquidate` and `clean_bad_debt` are never blocked by the global pause or
by the `frozen` flag, preserving the solvency defense in every incident
mode. One narrow exception exists for `paused` (see Addendum): a liquidation
**repay leg** whose debt listing is paused reverts, because a paused listing
accepts no inbound tokens from anyone. Seizure of paused **collateral**
remains fully allowed, and `clean_bad_debt` (which takes no tokens in) is
never blocked.

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

## Addendum (2026-07-11): Tainted-Debt Gate And Listing Lifecycle

Three amendments after an adversarial review of the pause/liquidation
interaction. None reverses the core decision that liquidation outlives the
halt controls; the first closes a token-integrity hole the original
analysis (which reasoned only about price/HF integrity) did not cover.

### Tainted-debt liquidation gate

The original blanket exemption let liquidators pay a **paused debt asset**
into the pool while user `repay` of the same asset was blocked. Pausing a
listing usually means its token or oracle is untrusted; if the token is
compromised (e.g. infinite mint), the liquidation repay leg was the one
remaining path accepting it — fake tokens would extinguish real debt,
become withdrawable pool cash, and seize real collateral at bonus.

Amendment: a liquidation repay leg whose debt listing is `paused` reverts
`SpokeAssetPaused`. The authoritative check sits in
`apply_liquidation_repayments`, on the post-normalization legs that
actually transfer (the plan normalizer can drop request legs, so checking
the raw request alone is insufficient); a fast-fail twin runs in
`validate_liquidation_inputs`. Seizure of paused **collateral** stays open:
the liquidator pays real value and bears the seized asset's risk, so the
original bad-debt argument still holds on the collateral side. This gate
does not reintroduce a liquidation-DoS: it binds only accounts whose chosen
repay asset is paused — exactly the population where accepting payment is
the exploit.

### Listing lifecycle

- `remove_asset_from_spoke` requires **zero usage** (`SpokeAssetInUse`
  otherwise). Invariant: a live position's listing always exists — flags
  stay enforceable for the lifetime of every position, the spoke oracle
  override cannot vanish under a live position (which would instantly
  reprice it to the base oracle), and no usage row survives into a
  re-listing. Removal is registry cleanup; `frozen` is the wind-down tool.
- `edit_asset_in_spoke` now works on **deprecated** spokes. Deprecation
  blocks new entries at the account gates independently; refusing edits
  only stranded live listings (a listing paused at deprecation time was
  permanently locked). Risk-param stewardship therefore follows one rule:
  **params are live while the listing exists; frozen only when unlisted** —
  applied uniformly by withdraw refresh, net-settle refresh, and
  `update_account_threshold` (which skips delisted assets instead of
  reverting).

## References

- `contracts/controller/src/governance/access.rs`
- `contracts/controller/src/positions/mod.rs` (`enforce_spoke_asset_flags`)
- `contracts/controller/src/positions/liquidation/apply.rs` (tainted-debt gate)
- `contracts/controller/src/config/asset.rs` (listing lifecycle)
- [ADR 0009](./0009-mainnet-launch-hardening-and-operational-control.md)
- [ADR 0010](./0010-governance-timelock-for-controller-admin.md)
