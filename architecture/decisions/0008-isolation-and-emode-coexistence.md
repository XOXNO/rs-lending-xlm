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
per-market reverse membership list. The two are **mutually exclusive per
account**: an account is either isolated or in an e-mode category, never both.
This is enforced at account creation by
`emode::validate_e_mode_isolation_exclusion` (panics
`EModeError::EModeWithIsolated` when `e_mode_category_id > 0 && is_isolated`,
called from `create_account`) and re-checked at runtime by
`emode::ensure_e_mode_compatible_with_asset` on deposit, borrow, and collateral
swap, which rejects using an isolated-asset collateral while an e-mode category
is active. Only siloed borrowing composes with each of them.

**Isolation** (`AccountMeta.is_isolated`, `isolated_asset`):

- An isolated account uses exactly one isolated collateral asset.
- Borrows are limited to assets with `isolation_borrow_enabled = true`.
- Aggregate exposure against an isolated collateral is tracked in two
  persistent layers:
  - `ControllerKey::IsolatedDebt(collateral)` — protocol-wide USD WAD
    counter (cached per tx, flushed via `cache.flush_isolated_debts()`).
  - `ControllerKey::IsolatedBasis(account_id, debt_asset)` — per-position
    USD WAD principal recorded at borrow time for symmetric decrement on
    repay and liquidation.
- **Borrow** (`add_isolated_debt`, pre-pool): prices the debt asset under
  `OraclePolicy::RiskIncreasing`, converts the borrow amount to USD WAD,
  checks `current + amount <= isolation_debt_ceiling_usd` on the
  **collateral** config, increments the global counter, and adds the same
  WAD to the position basis.
- **Repay / liquidation** (`adjust_isolated_debt_for_repay`): no oracle
  reads (`OraclePolicy::Repay` on repay). Decrement is proportional to the
  repaid share of scaled debt against the stored basis (floored on partial
  repays; full close removes the remaining basis). Bad-debt cleanup uses
  `clear_position_isolated_debt` to drop the full basis for a seized leg.
- `OraclePolicy::IsolatedRepay` remains in the policy enum for Certora/spec
  compatibility but is **not** used on the repay hot path; basis accounting
  removed the oracle-drift problem that policy was meant to solve.

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
- A category is selected at account creation and is **immutable** thereafter —
  `e_mode_category_id` has no setter anywhere in the controller. The deposit
  entrypoint rejects (`EModeError::EModeMismatch`) a non-zero category that
  differs from the account's stored category. Changing category means creating
  a new account.
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
- The `IsolatedDebt(collateral)` counter tracks **borrow-time principal
  basis in USD WAD**, not live marked-to-market debt:
  - Interest accrual never increments the counter, so nominal outstanding
    debt can exceed the configured ceiling while new borrows remain blocked.
  - The borrow price is frozen into basis; debt-asset price moves after
    borrow are not reflected until positions turn over.
  - Partial-repay decrements floor the proportional basis share, so the
    counter can stay slightly above the ideal principal share until a full
    close (conservative for ceiling enforcement).
- The ceiling is therefore a **monitored soft bound**, not a hard cap on
  aggregate exposure. Accepted because every isolated position stays
  LTV-collateralized and independently liquidatable, and operators monitor
  `get_isolated_debt` and can pause or re-cap a market. Hard-cap options
  (deferred to keep the hot path cheap): enforce borrow against a
  keeper-recomputed sum of live scaled debt valued at current prices (see
  INVARIANTS §3.4), or add an on-chain index of isolated accounts. Per-account
  basis keys already exist and keep increment/decrement symmetric; they do not
  by themselves cap interest-inflated nominal debt.
- Category-asset membership has two storage faces (category-side map
  and per-market reverse list); both must stay consistent. The controller
  updates both faces in `config::add_asset_to_e_mode_category` and
  `config::remove_asset_from_e_mode`; `config::edit_asset_in_e_mode_category`
  rewrites only the category-side `EModeAssetConfig` flags, because membership
  (and thus the reverse list) is unchanged.

## References

- `SCF_BUILD_ARCHITECTURE.md` §12 (E-Mode, Isolation, and Siloed
  Borrowing), `architecture/INVARIANTS.md` §3.4 (Isolation Debt).
- `contracts/controller/src/emode.rs` (runtime helpers:
  `validate_e_mode_isolation_exclusion`, `ensure_e_mode_compatible_with_asset`,
  `apply_e_mode_to_asset_config`, `validate_isolated_collateral`)
- `contracts/controller/src/config.rs` (e-mode admin:
  `edit_e_mode_category`, `remove_e_mode_category`,
  `add_asset_to_e_mode_category`, `edit_asset_in_e_mode_category`,
  `remove_asset_from_e_mode`)
- `contracts/controller/src/positions/borrow.rs` (siloed/isolated/e-mode checks)
- `contracts/controller/src/positions/isolated_debt.rs` (isolated-debt helpers:
  `add_isolated_debt`, `adjust_isolated_debt_for_repay`,
  `clear_position_isolated_debt`)
- `contracts/controller/src/storage/debt.rs` (`IsolatedDebt`, `IsolatedBasis`)
- `contracts/controller/src/cache/mod.rs::{cached_emode_asset, flush_isolated_debts}`
- `common/src/types/` (`AccountMeta`, `EModeCategory`,
  `EModeAssetConfig`)
