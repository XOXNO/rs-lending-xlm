# Liquidity Pool

Market engine for one central pool: **interest, scaled shares, tracked cash**
per `(hub_id, asset)`. Controller owns risk; this contract only moves liquidity
under its own math.

| | |
| --- | --- |
| Mutators | `#[only_owner]` → controller only |
| Views | public |
| Interface | `interfaces/pool` |
| Math / keys | `common` (`Ray`, rates, `PoolKey`) |

## Role

```text
Controller (owner) ──only_owner──► Pool
  pre-transfers supply/repay           interest sync → mutate scaled + cash
  HF / spokes / caps                   CEI before token out
```

No collateral, no HF. Cash (not SAC balance) gates outflows; direct donations
raise SAC balance only and are not withdrawable
([ADR 0001](../../architecture/decisions/0001-controller-pool-ownership-boundary.md)).

## Surface

| Call | Effect |
| --- | --- |
| `create_market` | Params + state; indexes = `RAY` |
| `supply` / `borrow` / `withdraw` / `repay` | Cash ±; mint/burn scaled shares |
| `net_settle` | Same-market supply vs debt, no transfer |
| `seize_positions` | Bad-debt index write-down or deposit → revenue |
| `add_rewards` / `claim_revenue` | Supply-index growth; burn revenue shares |
| `flash_loan` / `create_strategy` | Lend → callback → pull; fee → revenue |
| `update_indexes` / `update_params` | Accrue; optional IRM replace |
| Views | Reserves, revenue, amounts, util, rates, bulk indexes (simulate) |

Checkpoint views do not write accrual; use `get_bulk_indexes` for live indexes.

## Layout

```text
src/
  lib.rs        ABI, batch orchestration, CEI, flash loan
  cache.rs      Load/save, cash, scale/unscale, resolve withdraw/repay
  interest.rs   Chunked accrual, protocol revenue, bad-debt write-down
  utils.rs      TTL, IRM, util/insolvency, liquidation fee
  views.rs      Checkpoint reads
  events.rs     Market-state / params snapshots
```

## Invariants (short)

| # | Rule |
| --- | --- |
| 1 | Cash is truth for outflows |
| 2 | Cash conservation on every mutator |
| 3 | CEI: save before token out |
| 4 | Full withdraw floors payout; full repay ceils debt burn |
| 5 | Bad-debt socializes via supply-index floor ([ADR 0007](../../architecture/decisions/0007-bad-debt-socialization-with-index-floor.md)) |

## Related

| Doc | Topic |
| --- | --- |
| [`architecture/INVARIANTS.md`](../../architecture/INVARIANTS.md) | Protocol-wide properties |
| [ADR 0002](../../architecture/decisions/0002-per-side-scaled-balance-storage.md) | Scaled balances |
| [ADR 0006](../../architecture/decisions/0006-flash-loan-balance-snapshot.md) | Flash loan brackets |
| `common/src/rates.rs` | Compound / index math |
