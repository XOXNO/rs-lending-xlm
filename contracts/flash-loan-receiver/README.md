# Flash Loan Receiver (test)

Test-only receiver for pool/controller flash-loan smoke tests. **Not for
production.** Any address can invoke it; a real receiver must gate the caller
to the trusted pool.

## Callback

```text
execute_flash_loan(initiator, asset, amount, fee, pool, data)
```

`data` is XDR `FlashLoanRequest { mode }`. Repayment is by **approve** (pool
pulls after return), not transfer.

| Mode | Behavior |
| --- | --- |
| `Success` | Approve `amount + fee` to pool |
| `NoRepay` | No approval |
| `UnderRepay` | Approve less than owed |
| `ReenterPoolFlashLoan` | Nested `pool.flash_loan` |
| `ReenterControllerSupply` | Nested controller `supply` |
| `Panic` | Deliberate trap |

## Layout

```text
src/lib.rs              Receiver + modes
examples/encode_request.rs   XDR helper for tests
```
