# Flash Loan Receiver (test)

Test-only receiver under `mock/` for pool/controller flash-loan smoke tests.
**Not for production.** Any address can invoke it; a real receiver must gate
the caller to the trusted pool.

## Callback

```text
execute_flash_loan(initiator, asset, amount, fee, pool, data)
```

`data` is XDR `FlashLoanRequest { mode }`. Repayment is by **approve** (pool
pulls after return), not transfer.

Nested reentry is aimed at the controller stored by `set_plan(controller, hub_id, spoke_id, account_id)`. If `set_plan` was never called, the receiver falls back to a hardcoded testnet controller id.

| Mode | Behavior |
| --- | --- |
| `Success` | Approve `amount + fee` to pool |
| `NoRepay` | No approval |
| `UnderRepay` | Approve less than owed |
| `ReenterPoolFlashLoan` | Nested `pool.flash_loan`, then approve |
| `Panic` | Deliberate trap |
| `ReenterControllerSupply` | Nested controller `supply`, then approve |
| `OverRepay` | Approve more than `amount + fee` |
| `PushToPool` | Direct-transfer dust into the pool (must fail the balance bracket) |
| `ReenterControllerBorrow` | Nested `borrow`, then approve |
| `ReenterControllerWithdraw` | Nested `withdraw`, then approve |
| `ReenterControllerRepay` | Nested `repay`, then approve |
| `ReenterControllerFlashLoan` | Nested controller `flash_loan`, then approve |
| `ReenterControllerFlashPosition` | Nested `flash_position`, then approve |
| `ReenterControllerMultiply` | Nested `multiply`, then approve |
| `ReenterControllerSwapDebt` | Nested `swap_debt`, then approve |
| `ReenterControllerSwapCollateral` | Nested `swap_collateral`, then approve |
| `ReenterControllerRdwc` | Nested `repay_debt_with_collateral`, then approve |
| `ReenterControllerLiquidate` | Nested `liquidate`, then approve |
| `ReenterMigrateBlend` | Nested `migrate_from_blend`, then approve |

Reentry modes approve repayment *after* the nested call so a broken flash-loan guard cannot hide behind a later repay failure.

## Layout

```text
src/lib.rs              Receiver + modes
examples/encode_request.rs   XDR helper for tests
```
