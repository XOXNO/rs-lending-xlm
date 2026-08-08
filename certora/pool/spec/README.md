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
