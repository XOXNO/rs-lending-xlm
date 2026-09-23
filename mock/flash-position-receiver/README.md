# Flash Position Receiver (test)

Test-only receiver under `mock/` for harness and live testnet
`flash_position` tests.
**Not for production.** Any address can `set_plan`; a real receiver must gate
the caller to the trusted controller.

The receiver does not mint. The live scenario funds it with native XLM, and
the callback transfers that collateral to the controller. The controller does
not pull a repayment, so leftover debt tokens stay on this contract.

## Callback

```text
execute_flash_position(initiator, account_id, asset, amount, fee, amount_received, controller, data)
```

The callback ignores `fee`, `amount_received` and `data`. Behavior comes from
`set_plan(mode, collateral, amount, extra, extra_amount, spoke_id)`. Without a
plan the callback traps with `MissingPlan`; an unknown `mode` traps with
`InvalidMode`. Transfers skip a non-positive amount.

| Mode | Behavior |
| --- | --- |
| `0` Success | Transfer `amount` of `collateral`, then `extra_amount` of `extra` |
| `1` KeepFunds | No transfer |
| `2` BelowMin | Transfer `amount - 1` of `collateral` |
| `3` Panic | Deliberate trap (`CallbackPanic`) |
| `4`–`9` | Nested `supply` / `borrow` / `withdraw` / `repay` / `flash_loan` / `flash_position` on hub `1` |
