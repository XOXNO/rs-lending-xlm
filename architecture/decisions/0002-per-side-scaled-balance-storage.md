# ADR 0002: Per-Side Scaled-Balance Storage

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team
- Supersedes: none

## Context

Each lending account holds positions in multiple assets, on each side
(supply and borrow). Two questions need a decision:

1. How are amounts represented over time as interest accrues?
2. How is account state laid out in storage so that high-frequency flows
   (supply, repay, withdraw) do not pay for unrelated state on every call?

Soroban storage charges for read and write of each persistent entry, and
storage entries have TTLs that must be bumped to remain alive. Reading or
writing a single large account record on every operation pays for state the
operation does not touch.

## Decision

**Scaled-balance accounting.** Positions store balances scaled against the
pool's supply or borrow index (RAY precision). Actual amounts are
reconstructed at read time:

- `supply_actual = scaled_supply * supply_index / RAY`
- `borrow_actual = scaled_debt * borrow_index / RAY`

Indexes are advanced by `interest::global_sync` before each pool mutation
(`pool/src/interest.rs`). Interest accrues to all holders by index motion,
not by per-account writes.

**Per-side storage split.** Account state is partitioned into three keys:

- `ControllerKey::AccountMeta(u64)` — owner, isolation/e-mode flags.
- `ControllerKey::SupplyPositions(u64)` — `Map<Address, AccountPosition>`.
- `ControllerKey::BorrowPositions(u64)` — `Map<Address, AccountPosition>`.

`AccountPosition` stores
`scaled_amount_ray`, `liquidation_threshold_bps`, `liquidation_bonus_bps`,
`liquidation_fees_bps`, `loan_to_value_bps`. The asset is the enclosing
map key; the side is the enclosing storage key; the account id is the
key discriminant. Risk parameters are an open-time snapshot (see ADR
0008 for liquidation-threshold updates).

## Alternatives Considered

- **Per-account amount + last-update timestamp**, accruing interest at read
  time per position. Rejected: every read is a multiplication that depends
  on a per-account timestamp, and a global rate change forces a sweep over
  all accounts to keep them consistent.
- **Single combined `Positions(id)` map covering both sides.** Rejected:
  every supply or repay would read and write a record containing the
  unrelated side's positions. The split keeps `process_supply` to the
  supply side and `process_repay` to the borrow side
  (`controller/src/positions/supply.rs`, `controller/src/positions/repay.rs`).
- **Per-asset per-account entries (one storage key per (account, asset, side)).**
  Rejected: explodes Soroban entry count for accounts with multiple positions
  and multiplies TTL bumps. The map-inside-key form lets a flow read or
  write one map per touched side.

## Consequences

Positive:

- Interest accrual is `O(1)` per pool: index motion replaces account sweeps.
- Supply-only and repay-only flows touch only the relevant side; debt-free
  withdraws skip loading borrow state entirely
  (`controller/src/positions/withdraw.rs`).
- Health-factor checks load both sides only where actually required.
- Storage TTL is partitioned: `keepalive_accounts` bumps account-side
  entries; pool TTL is bumped on every pool mutation
  (`controller/src/storage/ttl.rs`, `controller/src/router.rs`).

Negative / accepted costs:

- Reading an actual amount requires multiplying by an index that may have
  drifted since last sync; the controller cache memoizes per-asset
  `MarketIndex` to avoid recomputation
  (`controller/src/cache/mod.rs::cached_market_index`).
- Index updates must be guarded against degenerate states; see ADR 0007
  (bad-debt socialization floor).
- Bulk position limits (`PositionLimits`, validated cap 32 per side at
  `set_position_limits`) are required to bound the size of each map.

## References

- `SCF_BUILD_ARCHITECTURE.md` §5 (Account and Storage Model), §8
  (Fixed-Point Domains).
- `controller/src/positions/supply.rs::process_supply`
- `controller/src/positions/repay.rs::process_repay`
- `controller/src/positions/withdraw.rs`
- `pool/src/interest.rs::global_sync`
- `common/src/constants.rs::{RAY, WAD, BPS}`
