# Pool-core verification

## Purpose

The pool suite verifies the accounting core of each isolated market: scaled
supply and debt shares, indexes, cash, protocol revenue, bad-debt treatment,
fees, settlement, and flash-loan accounting.

The pool holds market aggregates, not account maps. These rules establish that
a returned position change agrees with the corresponding market-total change.
Persisting that result in an account is a separate controller responsibility.

## Review map

| Job | Primary evidence |
|---|---|
| Rate and index accounting | Utilization caps, one-chunk accrual, and view/accrue isomorphism; the rate curve, compounding, index limits, and interest split are proved in `certora/common/spec/rate_index_accounting_rules.rs` |
| Position accounting | Directed share rounding and split additivity for supply, borrow, withdraw, repay, net settlement, and full close |
| Seize and settlement accounting | Bad-debt removal, supply-index write-down, revenue absorption, and net settlement |
| Fee and strategy accounting | Revenue allocation, liquidation withdrawal fees, strategy debt and payout, claims, and recapitalization |
| Flash-loan accounting | Fee, balance targets, principal recovery, and fee booking |
| Pool-core sanity | Reachability witnesses for each accounting family above and for the supply-index floor |

## Conf to spec map

| Rule module | Confs |
|---|---|
| `state_invariant_rules.rs` | `pool-state-invariant.conf` |
| `position_accounting_rules.rs` | `position-accounting.conf` |
| `seize_settle_accounting_rules.rs` | `seize-settle-accounting.conf` |
| `fee_strategy_accounting_rules.rs` | `fee-strategy-accounting.conf`, `fee-strategy-accounting-reverts.conf`, `fee-strategy-accounting-reverts-sanity.conf` |
| `flash_loan_accounting_rules.rs` | `flash-loan-accounting.conf` |
| `isomorphism_rules.rs` | `pool-isomorphism.conf` |
| `guard_rules.rs` | `pool-guards.conf` |
| `lifecycle_rules.rs` | `pool-lifecycle.conf`, `pool-lifecycle-reverts.conf`, `pool-lifecycle-reverts-sanity.conf` |
| `core_sanity_rules.rs` | `pool-core-sanity.conf` |

The `-reverts` confs hold the `call(...); cvlr_assert!(false);` rules at
`rule_sanity: none`, each paired with a `_fixture_completes` witness in the
sibling `-reverts-sanity` conf. `certora/README.md` explains why.

Six pool confs keep `multi_assert_check: true`: `pool-state-invariant`,
`position-accounting`, `seize-settle-accounting`, `fee-strategy-accounting`,
`flash-loan-accounting`, and `pool-guards`. Most of their rules carry several
asserts, so per-assert splitting gives a usable failure location. Every other
conf in the suite has it off.

## Properties under review

### Share accounting

Supply and debt are stored as scaled shares. Each conversion has a fixed,
conservative rounding direction. Positive token movement that would change zero
shares must fail, so no operation moves value without changing the accounting
record.

### Interest and indexes

The rate rules in `certora/common/spec/rate_index_accounting_rules.rs` cover
the kinked rate curve, rate caps, compounding, and the split of accrued
interest into supplier reward and protocol revenue. The pool rules cover
utilization caps and one accrual chunk on a seeded market. The borrow index
never decreases and is capped at `MAX_BORROW_INDEX_RAY`. The supply index is
capped at `MAX_SUPPLY_INDEX_RAY` and can fall when eligible bad debt is
socialized.

### Loss socialization

Residual eligible debt reduces the supply index of the affected market only.
The non-zero index floor is an intentional exception to exact proportional
write-down:

```rust
let new_supply_index = max(
    proportional_write_down,
    SUPPLY_INDEX_FLOOR_RAW,
);
```

The floor keeps later share conversion defined. After a total loss, it can
leave a small unbacked claim (`seize_floor_residual_reachable`). For this
reason, review the backing and recapitalization rules with the loss rules.

### Cash and revenue

Tracked cash, rather than an incidental token balance, is the reserve book.
Claims, recapitalization, strategy fees, and liquidation fees must preserve
the relationship among cash, supplied shares, borrowed shares, and revenue.

### Flash loans

The suite checks the successful accounting chain: fee calculation, required
balance targets, principal recovery, and fee booking. Only the fee enters the
cash book; the lent principal does not.

## Fixture domain

`fixture.rs` seeds one market per rule. The pool stores exactly two keys
per market, `PoolKey::Params` and `PoolKey::State` (`contracts/pool/src/storage.rs`),
and `seed` writes both plus the Ownable owner through the constructor. Nothing
else in the pool is read from storage, so a seeded rule has no arbitrary book
left over -- unlike the controller, whose position maps stay havoced.

Fields the fixture pins, and where rules draw them instead:

| Field | Fixture | Production range | Where it is drawn |
|---|---|---|---|
| `asset_decimals` | 7 in `params` | `MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS` (3..=18) | symbolic in the state-invariant, seize/settle, position-accounting and lifecycle families |
| `reserve_factor` | 1_000 in `params` | `< BPS` | symbolic in the state-invariant and seize/settle families |
| rate curve (`base_borrow_rate`, `slope1..3`, `mid_utilization`, `optimal_utilization`, `max_borrow_rate`) | one mainnet-shaped curve | any curve `InterestRateModel::verify` accepts | **fixed**; quantified over in `certora/common/spec/rate_index_accounting_rules.rs` instead, where the curve is the subject and no market state is loaded |
| `max_utilization` | `RAY` (uncapped) | `optimal_utilization..=RAY` | `params_with_max_util` pins `0.9 RAY` in the two utilization-cap rules |
| `flashloan_fee` / `is_flashloanable` | per rule | `<= MAX_FLASHLOAN_FEE_BPS` | symbolic in the flash and strategy rules |

The curve stays fixed on purpose. The pool families check share and cash
accounting across a market transition. A symbolic curve adds nonlinear terms to
every rule that accrues and does not change the accounting claim. The common
rate rules prove the curve's own properties, with no host state.

`fixture::state` stamps `last_timestamp = e.ledger().timestamp() * 1_000`, and
`time::now_ms` recomputes the same product. Both multiplications are checked.
Every rule that seeds a market therefore assumes
`e.ledger().timestamp() <= u64::MAX / 1_000`. The assumption makes the excluded
overflow path visible; without it, Sunbeam drops the panic path silently.

## What these proofs do not establish

- Arbitrary token-contract behavior, allowance behavior, or callback behavior.
- Reentrancy and rollback across external calls.
- Persistence of returned account positions by the controller.
- Unbounded multi-year accrual or arbitrary-length batch-loop induction.

These boundaries are deliberate. Controller proofs, integration tests, and
adversarial tests must cover them.

## How to run and extend

Before you submit, run the static checks and build the focused artifacts:

    ./certora/compile_all.sh
    make certora-wasm

Run the `sanity` profile first; it holds the pool witness confs. Then run the
changed conf or the `core` profile.

    ./certora/scripts/run_profile.py sanity
    ./certora/scripts/run_profile.py core

When changing pool accounting:

1. Identify the invariant and affected market transition.
2. Add a focused fixture and reachability witness.
3. Add or revise the smallest rule that proves the intended property.
4. Run the relevant pool conf and read the report for the exact artifact you
   built.
5. Update this guide if the proof boundary or residual risk changes.
