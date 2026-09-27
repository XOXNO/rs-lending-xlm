# Pool verification

The pool rules prove the accounting of one isolated market: scaled supply and
debt shares, indexes, cash, revenue, bad debt, fees, settlement and flash
loans.

The pool stores market totals, not accounts. The rules prove that the position
change a call returns agrees with the change in market totals. Storing that
position is the controller's job and is not proved here.

## Rule modules

| Module | Conf | Proves |
|---|---|---|
| `state_invariant_rules.rs` | `pool-state-invariant` | One market invariant holds after create and every transition (supply, borrow, withdraw, repay, settle, seize, claim, recapitalize, strategy) |
| `position_accounting_rules.rs` | `position-accounting` | Directed share rounding and split additivity for supply, borrow, withdraw, repay, settlement, full close |
| `seize_settle_accounting_rules.rs` | `seize-settle-accounting` | Bad-debt removal, supply-index write-down, revenue absorption, net settlement |
| `fee_strategy_accounting_rules.rs` | `fee-strategy-accounting` | Revenue split, liquidation fees, strategy debt and payout, claims, recapitalization |
| `flash_loan_accounting_rules.rs` | `flash-loan-accounting` | Fee, balance targets, principal recovery, fee booking |
| `isomorphism_rules.rs` | `pool-isomorphism` | Accrue reaches a fixed point: a second accrual at the same time moves nothing |
| `guard_rules.rs` | `pool-guards` | Utilization caps, no cash overdraw, revenue stays backed, no orphan debt, payouts come from pool cash |
| `lifecycle_rules.rs` | `pool-lifecycle`, `-reverts`, `-reverts-sanity` | Create writes zeroed state, duplicate create reverts, zero-time accrue is a no-op (hosted prover only) |
| `core_sanity_rules.rs` | `pool-core-sanity` | Reachability witness for each family above and the supply-index floor |

The rate curve, compounding, index caps and interest split are proved in
`certora/common/spec/rate_index_accounting_rules.rs`.

The first six confs keep `multi_assert_check: true`. Their rules carry many
asserts, and a split per assert shows which one fails.

## Properties

**Shares.** Supply and debt are scaled shares. Each conversion rounds in a
fixed direction that favors the protocol. A non-zero token amount that changes
zero shares must fail, so no call moves value without a record.

**Indexes.** The borrow index never decreases and is capped at
`MAX_BORROW_INDEX_RAY`. The supply index is capped at `MAX_SUPPLY_INDEX_RAY`
and falls only when bad debt is socialized.

**Loss socialization.** Bad debt writes down the supply index of its own
market only. The write-down stops at a non-zero floor so that later share
conversions stay defined:

```rust
let new_supply_index = max(proportional_write_down, SUPPLY_INDEX_FLOOR_RAW);
```

After a total loss the floor can leave a small unbacked claim
(`seize_floor_residual_reachable`). Review the backing and recapitalization
rules together with the loss rules.

**Cash and revenue.** Tracked cash, not the token balance, is the reserve
book. Claims, recapitalization and fees keep cash, supplied shares, borrowed
shares and revenue consistent.

**Flash loans.** Only the fee enters the cash book, not the principal.

## Fixture domain

`fixture.rs` seeds one market per rule. The pool stores two keys per market,
`PoolKey::Params` and `PoolKey::State` (`contracts/pool/src/storage.rs`), and
`seed` writes both plus the owner through the constructor. So, unlike the
controller, a seeded pool rule has no havoced storage left.

| Field | Fixture | Production range | Symbolic in |
|---|---|---|---|
| `asset_decimals` | 7 | `MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS` (3..=18) | state-invariant, seize/settle, position-accounting, lifecycle |
| `reserve_factor` | 1_000 | `< BPS` | state-invariant, seize/settle |
| rate curve | one mainnet-shaped curve | any curve `InterestRateModel::verify` accepts | fixed here; symbolic in the common rate rules |
| `max_utilization` | `RAY` | `optimal_utilization..=RAY` | `params_with_max_util` sets `0.9 RAY` in the two utilization-cap rules |
| `flashloan_fee`, `is_flashloanable` | per rule | `<= MAX_FLASHLOAN_FEE_BPS` | flash and strategy rules |

The curve is fixed on purpose. A symbolic curve adds nonlinear terms to every
rule that accrues and does not change the accounting claim.

`fixture::state` sets `last_timestamp = e.ledger().timestamp() * 1_000`, and
`time::now_ms` computes the same checked product. Every seeded rule therefore
assumes `e.ledger().timestamp() <= u64::MAX / 1_000`. Without this assumption
Sunbeam drops the overflow panic path without a warning.

## Not proved here

- Token, allowance and callback behavior.
- Reentrancy and rollback across external calls.
- The controller storing the returned positions.
- Multi-year accrual and long batch loops.

Controller rules, integration tests and adversarial tests cover these.

## Run

    ./certora/compile_all.sh
    make certora-wasm
    ./certora/scripts/run_profile.py sanity
    ./certora/scripts/run_profile.py core
