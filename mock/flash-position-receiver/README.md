# Flash Position Receiver (test)

Test-only receiver under `mock/` for live testnet `flash_position` coverage.
**Not for production.** Any address can `set_plan`; a real receiver must gate
the caller to the trusted controller.

Does **not** mint. The integration scenario pre-funds it with native XLM and
the callback **pushes** that collateral to the controller. Leftover debt tokens
stay on this contract — there is no repayment pull.

## Callback

```text
execute_flash_position(initiator, account_id, asset, amount, fee, amount_received, controller, data)
```

`data` is ignored. Behavior comes from
`set_plan(mode, collateral, amount, extra, extra_amount)`:

| Mode | Behavior |
| --- | --- |
| `0` Success | Transfer `amount` of `collateral`, then `extra_amount` of `extra` |
| `1` KeepFunds | No transfer |
| `2` BelowMin | Transfer `amount - 1` of `collateral` (if positive) |
| `3` Panic | Deliberate trap (`CallbackPanic`) |
| `4`–`9` | Nested `supply` / `borrow` / `withdraw` / `repay` / `flash_loan` / `flash_position` |
