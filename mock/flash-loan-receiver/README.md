# Flash Loan Receiver (test)

Test-only receiver under `mock/` for harness and live testnet flash-loan tests.
**Not for production.** Any address can invoke it; a real receiver must gate
the caller to the trusted pool.

## Callback

```text
execute_flash_loan(initiator, asset, amount, fee, pool, data)
```

`data` is XDR `FlashLoanRequest { mode }`; other bytes trap with `InvalidData`.
The receiver repays by `approve`. The pool pulls `amount + fee` with
`transfer_from` after the callback returns.

Reentry modes read the `Plan` stored by `set_plan(controller, hub_id, spoke_id, account_id)`. Without a plan they trap with `MissingPlan`. `ReenterPoolFlashLoan` calls the callback's `pool` and takes only `hub_id` from the plan; the other reentry modes call `controller`.

| Mode | Behavior |
| --- | --- |
| `Success` | Approve `amount + fee` to pool |
| `NoRepay` | No approval |
| `UnderRepay` | Approve `amount + fee - 1` |
| `ReenterPoolFlashLoan` | Nested `pool.flash_loan`, then approve |
| `Panic` | Deliberate trap (`CallbackPanic`) |
| `ReenterControllerSupply` | Nested controller `supply`, then approve |
| `OverRepay` | Approve `amount + fee + 1` |
| `PushToPool` | Transfer 1 unit to the pool, no approval (fails the pool balance check) |
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
| `ReenterControllerUpgradePoolParams` | Nested owner-only `upgrade_liquidity_pool_params`, then approve |

Reentry modes approve repayment after the nested call. In these tests the flash
loan starts at the controller, so the Soroban host rejects each controller
re-entry with `Error(Context, InvalidAction)` before the protocol flash guard
runs. The harness pins host rejection; these callbacks do not test the
controller's flash guard.

## Layout

```text
src/lib.rs                   Receiver + modes
examples/encode_request.rs   Prints the hex XDR data for a mode name
                             (used by make test-flash-loan-receiver)
```
