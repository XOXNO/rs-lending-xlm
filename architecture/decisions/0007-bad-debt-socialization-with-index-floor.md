# ADR 0007: Bad-Debt Socialization With Supply-Index Floor

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team

## Context

Liquidation caps repay at actual debt and pays a liquidator bonus. Residual
collateral can be too small for a rational liquidator to finish. Stranded debt
diverges from recoverable liquidity if left forever.

Options: leave it, pause the market, or socialize across suppliers via the
supply index. Socialization needs a floor so the index never hits zero and
breaks scaled-balance math.

## Decision

Socialize uncollectable debt by reducing pool `supply_index_ray`, floored at
`SUPPLY_INDEX_FLOOR_RAW` (= `WAD`).

### Trigger

After liquidation (or via `clean_bad_debt`), socializable when total debt USD
**and** total collateral USD satisfy `debt > collateral` **and**
`collateral <= BAD_DEBT_USD_THRESHOLD` (5 WAD). Underwater positions with
collateral **above** that threshold are not socializable here; further
liquidation must wind them down.

Then `execute_bad_debt_cleanup` seizes **both** sides in one
`pool.seize_positions` batch, emits `CleanBadDebtEvent`, and removes the
account.

### Index motion

Only **Borrow**-side seizure drives the index. Deposit seizure adds scaled
amount to pool revenue. Borrow seizure unscales debt and
`apply_bad_debt_to_supply_index`:

```text
total_supplied_value = supplied * supply_index
capped = min(bad_debt, total_supplied_value)
reduction_factor = (total_supplied_value - capped) / total_supplied_value
new_supply_index = supply_index * reduction_factor  // floored
```

Revenue accrual short-circuits when `index <= floor`.

### Standalone path

`clean_bad_debt(caller, account_id)` is **permissionless** (caller auth + not
flash-loaning). Not `#[when_not_paused]`. Reverts unless the socializable
predicate still holds — callers apply realized loss only.

## Alternatives considered

- Auto-pause market — the loss has already crystallized; obscures accounting.  
- Insurance fund — separate capital and accounting surface.  
- No index floor — numerical collapse of scaled balances.  
- Per-account pro-rata writes — conflicts with scaled-balance design (ADR 0002).  


## Consequences

**Positive:** loss shows as index step-down; event for ops; floor keeps math
safe; no account sweep.

**Costs:** suppliers bear socialized loss (disclose); 5 WAD threshold is a
tunable; no dedicated “severe drop” event (use market-state snapshot).

## References

- `contracts/pool/src/interest.rs::apply_bad_debt_to_supply_index`  
- `contracts/controller/src/positions/liquidation/{apply,bad_debt,math}.rs`  
- `common/src/constants/pool.rs`, `contracts/controller/src/constants.rs`  
- [INVARIANTS.md](../INVARIANTS.md) §1.5, §3.3, §4.4  
- [ADR 0011](./0011-pause-and-freeze-matrix.md) (tainted-debt vs clean_bad_debt)  
