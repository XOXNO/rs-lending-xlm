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

Collateral positions also store the risk parameters used by health-factor,
liquidation, and LTV math. Those values are refreshed by account threshold update
flows when spoke risk parameters change.

## Alternatives Considered

- **Store token-native balances.** Rejected because interest accrual would require
  sweeping every account or accepting stale balances.
- **Single combined `Positions(id)` map.** Rejected because side-specific flows
  would read and write unrelated side state.
- **One key per account, asset, and side.** Rejected because it multiplies entry
  count and TTL renewal surface.

## Consequences

Positive:

- Interest accrual is `O(1)` per market row.
- Supply-only and repay-only flows touch only the relevant side.
- Account TTL renewal remains bounded to three account keys.
- Position maps are keyed by `HubAssetKey`, matching controller and pool market
  isolation.

Accepted costs:

- Large accounts can still pay map costs for the side they touch.
- Risk parameter changes need explicit threshold update handling for existing
  collateral positions.

## References

- `common/src/types/controller.rs`
- `common/src/types/pool.rs`
- `contracts/controller/src/account`
- `contracts/controller/src/positions`
- `contracts/pool/src/interest.rs`
