# ADR 0002: Per-Side Scaled-Balance Storage

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team
- Supersedes: none

## Context

Each lending account holds positions in multiple assets, on each side
(supply and borrow). Two questions need a decision:

1. Amount representation over time as interest accrues.
2. Account-state layout that keeps high-frequency flows (supply, repay,
   withdraw) from paying for unrelated state on each call.

Soroban storage charges for read and write of each persistent entry, and
storage entries have TTLs that must be bumped to remain alive. Reading or
writing a single large account record on each operation pays for state the
operation does not touch.

## Decision

**Scaled-balance accounting.** Positions store balances scaled against the
market's supply or borrow index (RAY precision). Actual amounts are
reconstructed at read time:

- `supply_actual = scaled_supply * supply_index / RAY`
- `borrow_actual = scaled_debt * borrow_index / RAY`

Indexes are advanced by `interest::global_sync` before each asset-scoped pool
mutation (`contracts/pool/src/interest.rs`). Interest accrues to all holders of
that market by index motion, not by per-account writes.

**Per-side storage split.** Account state is partitioned into three keys:

- `ControllerKey::AccountMeta(u64)`: owner, e-mode category id, and position
  mode.
- `ControllerKey::SupplyPositions(u64)`: `Map<Address, AccountPositionRaw>`.
- `ControllerKey::BorrowPositions(u64)`: `Map<Address, DebtPositionRaw>`.

The two sides store distinct persistent (`#[contracttype]`) shapes. The
collateral side stores `AccountPositionRaw`: `scaled_amount_ray`,
`liquidation_threshold_bps`, `liquidation_bonus_bps`, `loan_to_value_bps`, an
open-time snapshot of the collateral's risk parameters. The debt side stores
`DebtPositionRaw`, which carries only `scaled_amount_ray`; debt-side risk
parameters are read from live market config rather than snapshotted. The typed
in-memory `AccountPosition` / `DebtPosition` (Ray/Bps) are derived from these
raw forms. The asset is the enclosing map key; the side is the enclosing
storage key; the account id is the key discriminant. Liquidation-threshold
updates to a live position are applied by the keeper-gated
`update_account_threshold` (`contracts/controller/src/router.rs`), which
requires a health-factor buffer for risk-increasing changes.

## Alternatives Considered

- **Per-account amount + last-update timestamp**, accruing interest at read
  time per position. Rejected: each read is a multiplication that depends
  on a per-account timestamp, and a global rate change forces a sweep over
  all accounts to keep them consistent.
- **Single combined `Positions(id)` map covering both sides.** Rejected:
  each supply or repay would read and write a record containing the
  unrelated side's positions. The split keeps `process_supply` to the
  supply side and `process_repay` to the borrow side
  (`contracts/controller/src/positions/supply.rs`, `contracts/controller/src/positions/repay.rs`).
- **Per-asset per-account entries (one storage key per (account, asset, side)).**
  Rejected: explodes Soroban entry count for accounts with multiple positions
  and multiplies TTL bumps. The map-inside-key form lets a flow read or
  write one map per touched side.

## Consequences

Positive:

- Interest accrual is `O(1)` per market row in the central pool: index motion
  replaces account sweeps.
- Supply-only and repay-only flows mutate only the relevant side; a debt-free
  withdraw still loads the full account record but takes the permissive
  `RiskDecreasing` oracle path instead of the risk-increasing health-factor
  path (`process_withdraw` selects the policy by whether `borrow_positions` is
  empty, `contracts/controller/src/positions/withdraw.rs`).
- Health-factor checks load both sides where required.
- Storage TTL is partitioned: account-side entries are renewed by the account
  owner via the on-chain `renew_account` entrypoint (owner-authenticated,
  `contracts/controller/src/router.rs`), and any party can extend footprint TTL
  through permissionless off-chain `ExtendFootprintTtl` ops run by the keeper
  (`services/keeper`). The controller instance is bumped by
  `renew_controller_instance`; the central pool instance and asset-keyed
  `Params` / `State` rows are renewed by pool load/mutation paths.

Negative / accepted costs:

- Reading an actual amount requires multiplying by an index that may have
  drifted since last sync; the controller cache memoizes per-asset
  `MarketIndex` to avoid recomputation
  (`contracts/controller/src/cache/mod.rs::cached_market_index`).
- Index updates must be guarded against degenerate states; see ADR 0007
  (bad-debt socialization floor).
- Bulk position limits (`PositionLimits`, validated cap `POSITION_LIMIT_MAX`
  = 10 per side at `set_position_limits`) are required to bound the size of
  each map.

## References

- `SCF_BUILD_ARCHITECTURE.md` §5 (Account and Storage Model), §8
  (Fixed-Point Domains).
- `contracts/controller/src/positions/supply.rs::process_supply`
- `contracts/controller/src/positions/repay.rs::process_repay`
- `contracts/controller/src/positions/withdraw.rs`
- `contracts/pool/src/interest.rs::global_sync`
- `common/src/constants/` (`RAY`, `WAD`, `BPS`)
