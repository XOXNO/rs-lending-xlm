# ADR 0006: Flash-Loan Settlement by Pool Balance Snapshot

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team

## Context

A flash loan must leave the pool with at least principal + fee by end of the
transaction. Soroban is one host invocation (panic reverts all); auth is
per-invocation; SACs still need custody proof, not a receiver return value.

## Decision

Settle by snapshotting the pool’s own token balance and checking post-repay
balance. Implementation: `pool::flash_loan`, orchestrated by
`controller::strategies::flash_loan::process_flash_loan`.

### Pool (`flash_loan`)

1. `#[only_owner]` (controller only).  
2. Positive amount; non-negative fee.  
3. Synced market cache; `require_reserves(amount)`.  
4. Receiver must be deployed Wasm.  
5. Snapshot SAC `pre_balance`; expect `pre - amount` after payout and
   `pre + fee` after repay (exact equality, not “≥”).  
6. Liquidity gate is tracked market `cash >= amount`, not raw SAC balance.  
7. Transfer `amount` to receiver; assert balance.  
8. Callback `execute_flash_loan(initiator, asset, amount, fee, pool, data)`.  
9. Assert pool balance still equals post-payout expectation (callback must not
   move pool loaned-token balance).  
10. Require allowance ≥ `amount + fee`, then `transfer_from` that total.  
11. Assert final balance == `pre_balance + fee`.  
12. Fee → protocol revenue (when eligible) and `cash += fee`; persist state.  


### Controller guards

- `#[when_not_paused]`, `caller.require_auth()`  
- `require_not_flash_loaning` (single-flight with strategies)  
- Market flash-loan enabled; fee from market config (`MAX_FLASHLOAN_FEE_BPS`
  enforced at listing, not in the hot path)  
- Set `FlashLoanOngoing` for the call window  

## Alternatives considered

- EVM-style internal balance bookkeeping — weaker than observing pool custody.  
- Controller-owned begin/end — extra custody and storage.  
- Trust callback return value.  
- Persistent allowance for the pool.  
- No reentrancy flag — the flash guard also covers strategy router calls.  


## Consequences

**Positive:** repayment proven by pool balance; no temporary pool storage key;
one flash guard for flash loans and strategies.

**Costs:** extra balance reads; receivers must implement the ABI and approve the
pool pull. `contracts/flash-loan-receiver` is test harness only.

## References

- `contracts/pool/src/lib.rs::flash_loan`  
- `contracts/controller/src/strategies/flash_loan.rs`  
- `common/src/constants/shared.rs` (`MAX_FLASHLOAN_FEE_BPS`)  
- [INVARIANTS.md](../INVARIANTS.md) §2.5  
- [ADR 0005](./0005-strategy-aggregator-output-validated-by-balance-delta.md)  
