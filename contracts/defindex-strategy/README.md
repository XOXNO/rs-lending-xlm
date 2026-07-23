# DeFindex Strategy

DeFindex vault adapter over the lending controller: **one vault ↔ one
controller account**. Deposit/withdraw supply collateral; `harvest` emits a
D12 price-per-share from the pool supply index (no external yield claim).

| | |
| --- | --- |
| Config | `hub_id`, `spoke_id`, `asset`, `controller`, `pool` |
| Mapping | `VaultAccount(vault)` → controller `account_id` |
| Client | `interfaces/controller` |

## Surface

| Call | Behavior |
| --- | --- |
| `asset` | Configured underlying |
| `deposit(amount, from)` | Pull tokens → controller `supply` into vault’s account |
| `withdraw(amount, from, to)` | Controller `withdraw`; pay `to`; clear mapping on full exit |
| `balance(from)` | Live collateral for vault’s account |
| `harvest(from, data)` | Auth `from`; emit PPS from supply index (amount = 0) |

Constructor takes `asset` + init args that unpack the rest of `Config`.

## Layout

```text
src/
  lib.rs   Strategy trait, vault↔account mapping, TTL extend on read
```

## Notes

- Full withdraw clears `VaultAccount` immediately so a later deposit opens a
  fresh account (no stale mapping).
- Two vaults never share a lending account.
- TTL: extend vault mapping when below ~30d, up to ~180d.
