# Flash Position Receiver (test)

Test-only receiver for harness and live testnet `flash_position` tests.
**Not for production.** Any address can `set_plan`; a real receiver must gate
the caller to the trusted controller.

It does not mint: the live scenario funds it with native XLM, which the
callback transfers to the controller as collateral. The controller pulls no
repayment, so leftover debt tokens stay here.

## Callback

```text
execute_flash_position(initiator, account_id, asset, amount, fee, amount_received, controller, data)
```

The callback ignores `fee`, `amount_received` and `data`; behavior comes from
`set_plan(mode, collateral, amount, extra, extra_amount, spoke_id)`. No plan
traps `MissingPlan`; an unknown `mode` traps `InvalidMode`. Transfers skip
non-positive amounts.

| Mode | Behavior |
| --- | --- |
| `0` Success | Transfer `amount` of `collateral`, then `extra_amount` of `extra` |
| `1` KeepFunds | No transfer |
| `2` BelowMin | Transfer `amount - 1` of `collateral` |
| `3` Panic | Deliberate trap (`CallbackPanic`) |
| `4`–`9` | Nested `supply` / `borrow` / `withdraw` / `repay` / `flash_loan` / `flash_position` on hub `1` |
