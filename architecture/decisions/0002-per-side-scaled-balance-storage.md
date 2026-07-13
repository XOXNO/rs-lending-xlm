# ADR 0002: Per-Side Scaled-Balance Storage

- Status: Accepted
- Date: 2026-05-05
- Revised: 2026-06-30
- Deciders: XOXNO Lending contract team

## Context

Accounts can hold supply and debt positions across multiple `HubAssetKey`
markets. The protocol needs interest accrual without rewriting every account
and needs account storage that avoids reading unrelated state on common flows.

Soroban charges per persistent entry read/write, and persistent entries need TTL
renewal. A single large account record would make supply, repay, and withdraw
touch unrelated positions.

## Decision

Use scaled-balance accounting and split account state by side:

- `ControllerKey::AccountMeta(u64)`: owner, active spoke id, and position mode.
- `ControllerKey::SupplyPositions(u64)`: `Map<HubAssetKey, AccountPositionRaw>`.
- `ControllerKey::BorrowPositions(u64)`: `Map<HubAssetKey, DebtPositionRaw>`.

Supply and debt balances are stored as RAY-scaled shares. Actual token amounts
are reconstructed with the pool supply or borrow index. Indexes advance through
pool market sync, not account sweeps.

Collateral (supply) positions also store the risk parameters (LTV, liquidation
threshold/bonus/fees) used by health-factor, liquidation, and LTV math at open
time. Debt positions carry only the scaled share. Risk parameters on collateral
are refreshed explicitly via `update_account_threshold` (or on supply) when
spoke risk parameters change. Empty side maps are pruned on write to bound
storage/TTL cost.

## Alternatives Considered

- **Store token-native balances.** Rejected because interest accrual would require
  sweeping every account or accepting stale balances.
- **Single combined `Positions(id)` map.** Rejected because side-specific flows
  would read and write unrelated side state.
- **One key per account, asset, and side.** Rejected because it multiplies entry
  count and TTL renewal surface.

## Consequences

Positive:

- Interest accrual is `O(1)` per market row (pool sync, no account sweeps).
- Supply-only and repay-only flows touch only the relevant side map.
- Account TTL renewal remains bounded (AccountMeta + the two side maps; empty sides are removed on write).
- Position maps are keyed by `HubAssetKey`, matching controller and pool market isolation.
- Risk snapshots live only on collateral; debt is pure scaled (minimizes storage for borrow-heavy accounts).

Accepted costs:

- Large accounts can still pay map costs for the side they touch.
- Risk parameter changes need explicit threshold update handling for existing
  collateral positions.

## References

- `common/src/types/controller.rs` (Account, AccountMeta, AccountPositionRaw, DebtPositionRaw, ControllerKey)
- `common/src/types/pool.rs`
- `contracts/controller/src/storage/account.rs` (AccountMeta, SupplyPositions/BorrowPositions maps; empty map pruning on write)
- `contracts/controller/src/positions` (risk snapshot on supply positions, scaled-only debt)
- `contracts/controller/src/risk/params.rs` and `pool_ops/mod.rs` (refresh_supply_risk_params, update_account_threshold)
- `contracts/pool/src/interest.rs`
- `architecture/INVARIANTS.md` §5.2 (Account Storage) and §5.4 (Halt Controls interaction)

This storage design remains the implemented ground truth (per-side scaled maps, collateral carries risk snapshot at open/refresh time, debt is scaled-only, O(1) per-market interest).
