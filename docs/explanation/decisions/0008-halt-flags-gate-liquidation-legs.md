# 0008. Halt flags gate liquidation legs: paused stalls liquidation, frozen does not

Status: Accepted

## Context

A listing sometimes needs quarantine: a token contract behaves maliciously, an oracle
feed is under manipulation, or an exploit touches one asset. Quarantine only works if it
contains value movement in both directions — a "paused" flag that still lets liquidators
pull the tainted asset out is not a quarantine. But liquidation is also the protocol's
solvency mechanism, and multi-asset accounts intertwine legs: one account's liquidation
repays several debt assets and seizes several collateral assets in a single plan. The
design must decide what a halted listing means for a liquidation that touches it, and it
must do so without ever letting a softer flag (meant only to stop growth) trap users'
exits.

## Decision

One check carries both meanings.
`contracts/controller/src/positions/mod.rs::enforce_spoke_asset_flags` asserts
`!sa.paused` unconditionally, for every verb, and asserts `!sa.frozen` only when the
caller passes `FreezePolicy::BlockOnEntry`. The two flags are therefore precisely
distinct:

- `paused` is full quarantine: every verb touching the listing reverts
  (`SpokeError::SpokeAssetPaused`) — entries, exits, and liquidation legs alike.
- `frozen` is entry-only: it blocks new deposits and borrows
  (`FreezePolicy::BlockOnEntry`, reverting `SpokeError::SpokeAssetFrozen`) and never
  blocks withdraw, repay, or liquidation, which pass `FreezePolicy::AllowOnExit`.

Liquidation enforces the flags with `FreezePolicy::AllowOnExit` on every leg, in both
phases: the debt legs and the computed seizure legs during planning
(`contracts/controller/src/positions/liquidation/plan.rs::build_liquidation_plan`), and
again when applying repayments and collateral seizures
(`contracts/controller/src/positions/liquidation/apply.rs::apply_liquidation_repayments`,
`apply.rs::apply_liquidation_seizures`). A listing absent from the spoke makes the check
a no-op, so delisted assets stay exitable and liquidatable.

Both paused-side consequences are pinned adversarially:
`tests/test-harness/tests/controller/security_audit.rs::poc_paused_debt_blocks_liquidation_repay`
(debt side) and
`security_audit.rs::poc_paused_collateral_blocks_liquidation_seizure`
(collateral side); the delisted-listing no-op is pinned by
`contracts/controller/tests/positions/flags.rs::missing_spoke_asset_is_noop`.

## Alternatives

- **Exempting liquidation from halt flags entirely.** Liquidation would stay live through
  any quarantine, preserving solvency enforcement. But it hands liquidators exactly the
  extraction path the quarantine exists to close: during a token or oracle incident, the
  tainted asset would keep flowing out of the pool through liquidations priced by the
  suspect feed. Containment is worthless with a built-in bypass.
- **Per-leg skipping.** Liquidation could route around the paused listing and settle only
  the healthy legs. This keeps partial liveness but breaks the plan's accounting: the
  seizure proportions, bonus curve, and health-factor outcome are computed over the whole
  account, and settling a subset can leave the account less healthy or over-seize the
  liquid legs. Aborting the whole liquidation keeps every executed plan internally
  consistent.
- **Making `paused` entry-only with a separate harder quarantine flag.** A three-flag
  scheme separates "stop growth" from "stop everything" more granularly, but `frozen`
  already is the entry-only tier; a third flag would duplicate it and widen the guardian
  surface (ADR 0007) for no new capability.

## Consequences

- The accepted trade-off, stated plainly: a `paused` listing on **either** the debt or
  the collateral side stalls the **entire** liquidation of a multi-asset account — one
  tainted leg makes the whole account temporarily unliquidatable. `frozen` does not do
  this: it only blocks entry, and liquidation proceeds through frozen listings
  unimpeded. This is containment-over-liveness, chosen deliberately: while a listing is
  paused, unhealthy accounts touching it can accrue further bad debt unliquidated, and
  that solvency risk is accepted for the duration of the quarantine.
- The risk window is bounded by governance latency: clearing `paused` requires the
  timelocked `EditAssetInSpoke` (ADR 0007), so a quarantine costs at least one timelock
  delay of suspended liquidations on affected accounts. Guardians should therefore reach
  for `frozen` when the concern is only new exposure, and reserve `paused` for genuine
  quarantine.
- Liquidation bots must treat `paused` as account-wide: filtering out only the paused
  leg produces plans that revert; the whole account is skipped until the flag clears.
- Delisting is not quarantine: a removed listing is a no-op in
  `enforce_spoke_asset_flags`, keeping wind-downs exitable and liquidatable — freezing or
  delisting can never trap funds, only `paused` can, and only deliberately.
- The gating matrix (global pause, `paused`, `frozen`, per verb) is pinned by the HALT
  domain, its interaction with liquidation by the LIQ and RISK domains (see
  ../../reference/invariants.md §HALT, §LIQ, §RISK), and the tainted-asset scenario is
  analyzed in ../threat-model.md.
