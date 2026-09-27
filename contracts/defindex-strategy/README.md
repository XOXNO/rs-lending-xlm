# DeFindex Strategy

DeFindex vault adapter over the lending controller: **one vault, one
controller account**. Deposits and withdrawals move supply collateral.
`harvest` claims no external yield; it emits the supply index as a price per
share.

| | |
| --- | --- |
| Config | `hub_id`, `spoke_id`, `asset`, `controller`, `pool` |
| Mapping | `VaultAccount(vault)` → controller `account_id` |
| Client | Uses [`interfaces/controller`](../../interfaces/controller); has no client of its own |

Full signatures are in `contracts/defindex-strategy/src/lib.rs`; the generated
client drops the `Env` argument.

## Entrypoints

| Entrypoint | Caller | Does |
| --- | --- | --- |
| `__constructor(asset, init_args)` | deployer, once | `init_args` = `(controller, hub_id, spoke_id)`; reads `pool` from the controller |
| `deposit(amount, from) -> i128` | `from` | Pulls tokens and calls controller `supply` into the vault's account |
| `withdraw(amount, from, to) -> i128` | `from` | Calls controller `withdraw`, pays `to`; a full exit clears the mapping |
| `harvest(from, data)` | `from` | Emits the price per share (12 decimals, rounded down) with amount 0 |
| `balance(from) -> i128` | anyone | Live collateral of the vault's account |
| `asset() -> Address` | anyone | The underlying asset |

## Notes

- A full withdrawal clears `VaultAccount`, so the next deposit opens a new
  account. Two vaults never share an account.
- The mapping TTL extends to 120 days (`TTL_BUMP_USER`) when it drops below
  30 days (`TTL_THRESHOLD_USER`).

## Layout

```text
src/
  lib.rs   Strategy trait, vault ↔ account mapping, TTL extension on read
```

## References

- Controller `supply` and `withdraw`: [`contracts/controller`](../controller/README.md)
