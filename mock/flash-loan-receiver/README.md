# Flash Loan Receiver (test)

Test-only receiver for harness and live testnet flash-loan tests.
**Not for production.** Anyone can invoke it; a real receiver must gate the
caller to the trusted pool.

## Callback

```text
execute_flash_loan(initiator, asset, amount, fee, pool, data)
```

`data` is XDR `FlashLoanRequest { mode }`; other bytes trap `InvalidData`. The
receiver repays by `approve`; the pool pulls `amount + fee` with
`transfer_from` after the callback.

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
| `ReenterControllerBorrow`, `ReenterControllerWithdraw`, `ReenterControllerRepay`, `ReenterControllerFlashLoan`, `ReenterControllerFlashPosition`, `ReenterControllerMultiply`, `ReenterControllerSwapDebt`, `ReenterControllerSwapCollateral`, `ReenterControllerRdwc`, `ReenterControllerLiquidate` | Nested `borrow`, `withdraw`, `repay`, `flash_loan`, `flash_position`, `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral` or `liquidate` respectively, then approve |
| `ReenterMigrateBlend` | Nested `migrate_from_blend`, then approve |
| `ReenterControllerUpgradePoolParams` | Nested owner-only `upgrade_liquidity_pool_params`, then approve |

Here the flash loan starts at the controller, so the Soroban host rejects each
controller re-entry with `Error(Context, InvalidAction)` before the protocol flash guard
runs. The harness pins host rejection; these callbacks do not test the
controller's flash guard.

## Layout

```text
src/lib.rs                   Receiver + modes
examples/encode_request.rs   Prints the hex XDR data for a mode name
                             (used by make test-flash-loan-receiver)
```
