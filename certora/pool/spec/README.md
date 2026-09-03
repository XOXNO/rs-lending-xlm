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
| Rate and index accounting | Utilization and rate bounds, compounding, index limits, and interest allocation |
| Position accounting | Directed share rounding for supply, borrow, withdraw, repay, and full close |
| Seize and settlement accounting | Bad-debt removal, supply-index write-down, revenue absorption, and net settlement |
| Fee and strategy accounting | Revenue allocation, withdrawal fees, strategy debt and payout, claims, and recapitalization |
| Flash-loan accounting | Fee, balance targets, principal recovery, and fee booking |
| Pool-core sanity | Reachable witnesses for every fixture family and floor exception |

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

The pool accounting confs are the ones that keep `multi_assert_check: true`:
their rules carry eight to twelve asserts each, so per-assert splitting buys a
usable failure location. Every other conf in the suite has it off.

## Properties under review

### Share accounting

Supply and debt are stored as scaled shares. Each conversion has a fixed,
conservative rounding direction. Positive token movement that would change zero
shares must fail, so no operation moves value without changing the accounting
record.

### Interest and indexes

Rate rules cover utilization, the kinked rate curve, rate caps, compounding,
and the split of accrued interest into supplier reward and protocol revenue.
Borrow index growth is monotone and bounded. Supply index growth is bounded but
may fall during eligible bad-debt socialization.

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

The floor keeps later share conversion defined. It can leave a small residual
claim after a total loss, so backing and recapitalization rules remain part of
the same safety story.

### Cash and revenue

Tracked cash, rather than an incidental token balance, is the reserve book.
Claims, recapitalization, strategy fees, and liquidation fees must preserve
the relationship among cash, supplied shares, borrowed shares, and revenue.

### Flash loans

The suite checks the successful accounting chain: fee calculation, required
balance targets, principal recovery, and fee booking. Principal does not enter
the cash book merely because it was lent during a transaction.

## Fixture domain

`spec/fixture.rs` seeds one market per rule. The pool stores exactly two keys
per market, `PoolKey::Params` and `PoolKey::State` (`contracts/pool/src/storage.rs`),
and `seed` writes both plus the Ownable owner through the constructor. Nothing
else in the pool is read from storage, so a seeded rule has no arbitrary book
left over -- unlike the controller, whose position maps stay havoced.

What the fixture still pins, and why:

| Field | Fixture | Production range | Where it is drawn |
|---|---|---|---|
| `asset_decimals` | 7 in `params` | `MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS` (3..=18) | symbolic in the state-invariant, seize/settle and position-accounting families |
| `reserve_factor` | 1_000 in `params` | `< BPS` | symbolic in the state-invariant and seize/settle families |
| rate curve (`base_borrow_rate`, `slope1..3`, `mid_utilization`, `optimal_utilization`, `max_borrow_rate`) | one mainnet-shaped curve | any curve `InterestRateModel::verify` accepts | **fixed**; quantified over in `certora/common/spec/rate_index_accounting_rules.rs` instead, where the curve is the subject and no market state is loaded |
| `max_utilization` | `RAY` (uncapped) | `optimal_utilization..=RAY` | `params_with_max_util` pins `0.9 RAY` in the two utilization-cap rules |
| `flashloan_fee` / `is_flashloanable` | per rule | `<= MAX_FLASHLOAN_FEE_BPS` | symbolic in the flash and strategy rules |

Keeping the curve fixed is deliberate: the pool families are about share and
cash accounting across a market transition, and a symbolic curve adds four
nonlinear terms to every rule that accrues without changing the accounting
claim. The curve's own properties are proved one layer down, on the common
rate model, with no host state in the way.

`fixture::state` stamps `last_timestamp = e.ledger().timestamp() * 1_000` and
`time::now_ms` recomputes the same product, both checked. Every rule that seeds
a market therefore assumes `e.ledger().timestamp() <= u64::MAX / 1_000`, so the
overflow path is excluded explicitly rather than by a trap the reader cannot
see.

## What these proofs do not establish

- Arbitrary token-contract behavior, allowance behavior, or callback behavior.
- Reentrancy and rollback across external calls.
- Persistence of returned account positions by the controller.
- Unbounded multi-year accrual or arbitrary-length batch-loop induction.

Those boundaries are deliberate. They must be covered by controller proofs,
integration tests, targeted adversarial tests, or a future sound external-call
model.

## How to run and extend

Run static proof checks before submitting:

    ./certora/compile_all.sh
    make certora-wasm

Run the pool sanity profile first, then the targeted job or the core profile.

    ./certora/scripts/run_profile.py sanity
    ./certora/scripts/run_profile.py core

When changing pool accounting:

1. Identify the invariant and affected market transition.
2. Add a focused fixture and reachability witness.
3. Add or revise the smallest rule that proves the intended property.
4. Run the relevant pool configuration and inspect the exact artifact report.
5. Update this guide if the proof boundary or residual risk changes.
