# DeFindex Strategy

DeFindex vault adapter over the lending controller: **one vault ↔ one
controller account**. Deposits and withdrawals move supply collateral. `harvest`
emits the supply index as a 12-decimal price per share, rounded down, and claims
no external yield.

| | |
| --- | --- |
| Config | `hub_id`, `spoke_id`, `asset`, `controller`, `pool` |
| Mapping | `VaultAccount(vault)` → controller `account_id` |
| Client | `interfaces/controller` |

## Surface

| Call | Behavior |
| --- | --- |
| `asset` | Configured underlying |
| `deposit(amount, from)` | Auth `from`; pull tokens → controller `supply` into vault’s account |
| `withdraw(amount, from, to)` | Auth `from`; controller `withdraw`; pay `to`; clear mapping on full exit |
| `balance(from)` | Live collateral for vault’s account |
| `harvest(from, data)` | Auth `from`; emit PPS from supply index (amount = 0) |

Constructor takes `asset` and `init_args` = `(controller, hub_id, spoke_id)`;
it reads `pool` from the controller.

## Layout

```text
src/
  lib.rs   Strategy trait, vault↔account mapping, TTL extend on read
```

## Notes

- A full withdrawal clears `VaultAccount`, so the next deposit opens a new
  account.
- Two vaults never share a lending account.
- The vault mapping TTL extends to 120 days (`TTL_BUMP_USER`) when it falls
  below 30 days (`TTL_THRESHOLD_USER`).
