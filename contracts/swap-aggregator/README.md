# Swap Aggregator

DEX swap router for Soroban. Executes multi-hop, multi-venue routes built
off-chain and passed as XDR. Used by controller strategies
(`multiply`, `swap_collateral`, `swap_debt`, `repay_debt_with_collateral`).

| | |
| --- | --- |
| Owner | OZ `Ownable` (two-step) |
| Trust | Untrusted by controller — balance-delta checked |
| Venues | Aquarius · Comet · Phoenix · Soroswap · Sushi |

## Entrypoint

```text
execute_strategy(sender, total_in, swap_xdr) -> i128
```

`swap_xdr` → `StrategyPayload`: `token_in` / `token_out`, split `paths`,
`total_min_out`, optional `referral_id`.

1. Auth `sender`, pull `total_in`.
2. Optional fees (input or output side).
3. Walk hops; credit only **measured** balance deltas.
4. Revert if out `< total_min_out`.
5. Send `token_out` to `sender`.

Nothing from venues or payload amount fields is truth — only real balance
changes count ([ADR 0005](../../docs/explanation/decisions/0005-strategy-aggregator-output-validated-by-balance-delta.md)).

## Admin

| Area | Entrypoints |
| --- | --- |
| Fees | `set_static_fee`, `claim_admin_fees`, `sweep_balance` |
| Whitelist | `add_to_whitelist`, `remove_from_whitelist` |
| Referrals | `add_referral`, `set_referral_*`, `claim_referral_fees` |
| Upgrade | `upgrade` |

Fee cap: 1000 bps (static and referral).

## Layout

```text
src/
  lib.rs     Entrypoints, fees, path execution
  types.rs   StrategyPayload, venues, storage keys
  vault.rs   Invocation-local balance ledger
  venues/    Per-DEX hop adapters
  errors.rs  Error codes
```
