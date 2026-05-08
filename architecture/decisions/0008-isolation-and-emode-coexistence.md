# ADR 0008: Isolation and E-Mode Coexistence Model

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team
- Supersedes: none

## Context

Two risk-shaping features serve different purposes:

- **Isolation mode** restricts an account to a single, typically newer
  or higher-risk, collateral asset and bounds aggregate exposure
  through a global per-asset USD debt ceiling.
- **E-mode (efficiency mode)** lets correlated assets (e.g.
  stablecoins, ETH-like assets) be borrowed against each other at
  tighter LTV / liquidation-threshold parameters than the default.

Both touch account-level state and both interact with risk
parameters. The protocol has to decide how they coexist with each
other and with the per-account-position storage model
(ADR 0002).

A second concern is operability: e-mode categories age, and the
protocol must be able to deprecate one without rewriting member
markets or invalidating live accounts that referenced it.

## Decision

Isolation is account-level state; e-mode is category state plus a
per-market reverse membership list.

**Isolation** (`AccountMeta.is_isolated`, `isolated_asset`):

- An isolated account uses exactly one isolated collateral asset.
- Borrows are limited to assets with `isolation_borrow_enabled = true`.
- Total isolated debt is tracked in `ControllerKey::IsolatedDebt(asset)`
  in USD WAD. Borrowing increments; repay and liquidation decrement.
- Isolated accounts opt into `OraclePolicy::IsolatedRepay` on `repay`
  (ADR 0004), because the global counter would otherwise drift under
  permissive pricing.
- Isolated debt updates are batched in the cache and flushed once per
  controller op via `cache.flush_isolated_debts()`.

**E-mode** (`ControllerKey::EModeCategory(u32)`):

- Each category stores its own `loan_to_value_bps`,
  `liquidation_threshold_bps`, `liquidation_bonus_bps`,
  `is_deprecated`, and `assets: Map<Address, EModeAssetConfig>`.
- Each member market stores a reverse list:
  `AssetConfig.e_mode_categories: Vec<u32>`. The reverse list lets the
  controller quickly check whether an asset participates in a category
  the account selected.
- Category id `0` is the sentinel for "no e-mode" (cache short-circuits
  on `category_id == 0`, see `cached_emode_asset`).
- A category is selected at account creation. Switching categories
  requires the account to be in a state compatible with the new
  parameters; otherwise creating a new account is the supported path.
- `remove_e_mode_category` flags the category deprecated, clears its
  asset map, and removes its id from each member market's reverse
  list. Deprecated categories remain readable; new activity is blocked.

**Siloed borrowing** (`AssetConfig.is_siloed_borrowing`) is a separate,
asset-level flag that prevents an account from holding multiple debt
assets when any final debt asset is siloed. Siloed borrowing composes
with both isolation and e-mode and is checked alongside them in the
borrow validation path.

## Alternatives Considered

- **Account-level e-mode flag plus per-account override.** Rejected:
  multiplies storage per account and makes parameter changes O(N)
  across accounts. Category-level state plus a snapshot of risk
  parameters at account creation (ADR 0002) keeps storage cheap.
- **No isolated-debt counter, rely on collateral cap.** Rejected: a
  collateral cap caps deposits but not borrowed exposure across
  accounts. The USD WAD counter bounds aggregate isolated debt
  globally.
- **Reuse e-mode categories as isolation surfaces.** Rejected: e-mode
  is a relaxation (tighter LTV for correlated assets); isolation is a
  restriction (single collateral with a global ceiling). Conflating
  them collapses the threat model.
- **Allow a deprecated category to be deleted.** Rejected: live
  accounts may still reference it; deprecation preserves read state
  while blocking new activity.

## Consequences

Positive:

- Isolation and e-mode each have a clean, separately-auditable surface.
- Reverse membership lists keep e-mode category checks O(1) per asset
  per cache hit (`cached_emode_asset`).
- The isolated-debt counter gives operators a single number per
  asset for monitoring and ceiling enforcement.
- Deprecation does not require migrating live accounts.

Negative / accepted costs:

- Two flags in `AccountMeta` (`is_isolated`, `isolated_asset`) plus a
  category id and a mode field; their interaction increases verification
  and monitoring complexity.
- Strict oracle pricing on isolated `repay` reduces the surface where
  permissive pricing would otherwise let users repay; this is
  intentional and belongs in protocol risk disclosures.
- Category-asset membership has two storage faces (category-side map
  and per-market reverse list); both must stay consistent. The
  controller updates both in `add_asset_to_e_mode_category` /
  `edit_asset_in_e_mode_category` / `remove_asset_from_e_mode`.

## References

- `SCF_BUILD_ARCHITECTURE.md` §12 (E-Mode, Isolation, and Siloed
  Borrowing).
- `controller/src/positions/emode.rs`
- `controller/src/positions/borrow.rs` (siloed/isolated/e-mode checks)
- `controller/src/cache/mod.rs::cached_emode_asset`
- `controller/src/utils.rs` (isolated-debt accounting helpers)
- `common/src/types.rs` (`AccountMeta`, `EModeCategory`,
  `EModeAssetConfig`)
