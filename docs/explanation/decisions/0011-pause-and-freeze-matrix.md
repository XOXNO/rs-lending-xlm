# ADR 0011: Pause And Freeze Matrix

- Status: Accepted
- Date: 2026-07-02
- Deciders: XOXNO Lending contract team

## Context

Three halt controls with different scopes:

1. Global controller pause (OpenZeppelin pausable + `#[when_not_paused]`)  
2. Per-spoke-asset `paused`  
3. Per-spoke-asset `frozen`  

A protocol-wide scare, an oracle incident, and a single-market incident need
different switches. Operators need an explicit matrix of what each layer blocks.

## Decision

Keep three layers with different coverage.

### Layer 1: Global pause

`#[when_not_paused]` gates risk-increasing and index-mutating entrypoints,
including: `supply`, `borrow`, `multiply`, `swap_debt`, `swap_collateral`,
`repay_debt_with_collateral`, `migrate_from_blend`, `flash_loan`,
`update_indexes`, `claim_revenue`, `add_rewards`, and
`update_account_threshold`.

It does not gate `withdraw`, `repay`, `liquidate`, or `clean_bad_debt` — exits
and de-risking stay live.

- The constructor deploys the controller paused.  
- `upgrade` re-pauses before swapping code.  
- **Pause:** governance `pause(caller)` — GUARDIAN, immediate.  
- **Unpause:** timelocked `AdminOperation::Unpause` only. Controller
  `pause` / `unpause` are owner-only (owner = governance after execute).  

### Layer 2: Per-spoke-asset `paused`

Blocks `supply`, `borrow`, `withdraw`, and `repay` for that
`(spoke, hub-asset)`, including exits. Strategy flows share those processors and
inherit the flag.

### Layer 3: Per-spoke-asset `frozen`

Blocks only new `supply` and `borrow`. `withdraw` and `repay` stay live (orderly
wind-down).

### Setting per-asset flags

Flags live on `SpokeAssetArgs` and are written by `add_asset_to_spoke` /
`edit_asset_in_spoke`. Every edit states them explicitly so a parameter edit
cannot silently clear a flag.

| Path | Behavior |
|------|----------|
| Timelocked `edit_asset_in_spoke` | Planned wind-down and parameter changes; may clear flags |
| GUARDIAN `set_spoke_asset_flags` | Immediate, tighten-only (`false → true` or stay). Clearing reverts `SpokeAssetFlagRelaxation`; reopening uses the timelock |

Global pause is the protocol-wide brake. Per-listing oracle or token incidents
use layer 2.

### Liquidations

`liquidate` and `clean_bad_debt` are never blocked by global pause or `frozen`.

If the **debt** listing is spoke-`paused`, the liquidation repay leg reverts
(`SpokeAssetPaused`): a paused listing does not accept inbound tokens. The check
runs on post-normalization legs in `apply_liquidation_repayments` (the plan
normalizer can drop raw request legs). Seizure of paused **collateral** stays
open. `clean_bad_debt` takes no tokens in and is never blocked by that gate.

### Listing lifecycle

- `remove_asset_from_spoke` requires zero usage (`SpokeAssetInUse` otherwise).
  A live position’s listing always exists so flags stay enforceable. Removal is
  registry cleanup; `frozen` is the wind-down tool.  
- `edit_asset_in_spoke` works on deprecated spokes so a listing paused at
  deprecation is not permanently locked.  
- Caps are not validated against usage: a cap below live usage is the ratchet;
  enforcement is entry-time only. Interest may push usage over a cap until exits
  catch up.  
- Lifecycle: active → frozen/paused → removed when empty. Unlisted on exit paths
  fails safe; entry fails loud (`AssetNotInSpoke`).  

## Alternatives considered

- Pause everything including exits — traps users; creates a bank-run incentive.  
- Gate liquidations behind pause — delayed liquidation mints bad debt.  
- Single per-asset flag — “stop everything” and “wind down” need different exit
  semantics.  
- Immediate unpause — risk-loosening; see ADR 0010.  

## Consequences

**Positive:** exits and liquidations survive global pause; per-asset flags
isolate one market; halt is fast; resume is deliberate.

**Limitation:** global pause is not an oracle killswitch. Debt-bearing
`withdraw` and `liquidate` still read oracles while globally paused. For an
oracle incident, use per-asset `paused`. There is no admin operation that deletes
an oracle entry for a live market; flags contain exposure.

**Costs:** operators must pick the right layer; per-asset `paused` is stronger
than global pause for that market (blocks exits).

## References

- `contracts/controller/src/governance/access.rs`  
- `contracts/controller/src/positions/mod.rs`  
- `contracts/controller/src/positions/liquidation/apply.rs`  
- `contracts/controller/src/config/asset.rs`  
- `contracts/governance/src/timelock.rs`  
- `contracts/governance/src/op.rs`  
- [ADR 0010](./0010-governance-timelock-for-controller-admin.md)  
- [ADR 0009](./0009-mainnet-launch-hardening-and-operational-control.md)  
