# Pool verification

The pool rules prove one isolated market's accounting. The pool stores
totals, not accounts, so the rules prove that a call's returned position change
matches the change in market totals. The controller stores that position.
These rules do not prove that part.

## Rule modules

| Module | Conf | Proves |
|---|---|---|
| `state_invariant_rules.rs` | `pool-state-invariant` | Market invariant after create and every transition (supply, borrow, withdraw, repay, settle, seize, claim, recapitalize, strategy) |
| `position_accounting_rules.rs` | `position-accounting` | Directed share rounding and split additivity for supply, borrow, withdraw, repay, settlement, and full close |
| `seize_settle_accounting_rules.rs` | `seize-settle-accounting` | Bad-debt removal, supply-index write-down, revenue absorption, net settlement |
| `fee_strategy_accounting_rules.rs` | `fee-strategy-accounting` | Revenue split, liquidation fees, strategy debt and payout, claims, recapitalization |
| `flash_loan_accounting_rules.rs` | `flash-loan-accounting` | Fee, balance targets, principal recovery, fee booking |
| `isomorphism_rules.rs` | `pool-isomorphism` | Accrue fixed point. A second same-time accrual moves nothing |
| `guard_rules.rs` | `pool-guards` | Utilization caps, no cash overdraw, backed revenue, no orphan debt, payouts from pool cash |
| `lifecycle_rules.rs` | `pool-lifecycle`, `-reverts`, `-reverts-sanity` | Create writes zeroed state, duplicate create reverts, zero-time accrue is a no-op (hosted prover only) |
| `core_sanity_rules.rs` | `pool-core-sanity` | Reachability witnesses for each family and the supply-index floor |

Rules for the rate curve, compounding, index caps, and interest split are in
`certora/common/spec/rate_index_accounting_rules.rs`.

The first six confs keep `multi_assert_check: true`. Their rules carry many
asserts, and a split per assert shows which one fails.

## Properties

**Shares.** Each scaled-share conversion rounds in the direction that favors
the protocol. A non-zero token amount that changes zero shares must fail, so
no call moves value without a record.

**Indexes.** The borrow index never decreases and is capped at
`MAX_BORROW_INDEX_RAY`. The supply index is capped at `MAX_SUPPLY_INDEX_RAY`
and falls only on bad-debt socialization.

**Loss socialization.** Bad debt writes down the supply index of its own
market only. The write-down stops at a non-zero floor so later share
conversions stay defined:

```rust
let new_supply_index = max(proportional_write_down, SUPPLY_INDEX_FLOOR_RAW);
```

After a total loss the floor can leave a small unbacked claim
(`seize_floor_residual_reachable`). Review backing and recapitalization rules
with the loss rules.

**Cash and revenue.** Tracked cash, not the token balance, is the reserve
book. Claims, recapitalization, and fees keep cash, shares, and revenue
consistent.

**Flash loans.** Only the fee, not the principal, enters the cash book.

## Fixture domain

`fixture.rs` seeds one market per rule. `seed` writes both per-market keys,
`PoolKey::Params` and `PoolKey::State` (`contracts/pool/src/storage.rs`), plus
the owner through the constructor. Unlike the controller, a seeded pool rule
has no havoced storage left.

| Field | Fixture | Production range | Symbolic in |
|---|---|---|---|
| `asset_decimals` | 7 | `MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS` (3..=18) | state-invariant, seize/settle, position-accounting, lifecycle |
| `reserve_factor` | 1_000 | `< BPS` | state-invariant, seize/settle |
| rate curve | one mainnet-shaped curve | any curve `InterestRateModel::verify` accepts | fixed here; symbolic in the common rate rules |
| `max_utilization` | `RAY` | `optimal_utilization..=RAY` | `params_with_max_util` sets `0.9 RAY` in the two utilization-cap rules |
| `flashloan_fee`, `is_flashloanable` | per rule | `<= MAX_FLASHLOAN_FEE_BPS` | flash and strategy rules |

The curve is fixed on purpose. A symbolic curve adds nonlinear terms to every
accruing rule without changing the accounting claim.

`fixture::state` sets `last_timestamp = e.ledger().timestamp() * 1_000`, the
same checked product `time::now_ms` computes, so every seeded rule assumes
`e.ledger().timestamp() <= u64::MAX / 1_000`. Without that assumption, Sunbeam silently
drops the overflow panic path.

## Not proved here

See [What a verdict does not prove](../../README.md#what-a-verdict-does-not-prove).
Controller rules, integration tests, and adversarial tests cover those gaps.

## Run

Run the profiles `sanity` and `core`. See [Run](../../README.md#run).
