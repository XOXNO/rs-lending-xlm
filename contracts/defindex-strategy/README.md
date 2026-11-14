# DeFindex Strategy

DeFindex vault adapter over the controller: **one vault, one controller
account**. Deposits and withdrawals move supply collateral; `harvest` claims no
external yield, only emits the supply index as price per share.

| | |
| --- | --- |
| Config | `hub_id`, `spoke_id`, `asset`, `controller`, `pool` |
| Mapping | `VaultAccount(vault)` → controller `account_id` |
| Client | Uses [`interfaces/controller`](../../interfaces/controller); has no client of its own |

## Authorization

`deposit`, `withdraw` and `harvest` require auth from `from`, the vault. The
strategy owns each vault's controller account and acts as its owner. Views
need no auth.

## Entrypoints

Signatures and returns: [endpoint reference](../../docs/reference/endpoints.md#defindex-strategy).
The constructor also checks the spoke and its listing of the asset. `deposit`
pulls tokens and calls controller `supply`; `withdraw` calls controller
`withdraw` and pays `to`. `harvest` emits price per share (12 decimals, rounded
down) with amount 0.

## Storage and TTL

| Key | Tier | Holds |
| --- | --- | --- |
| `DataKey::Config` | instance | `hub_id`, `spoke_id`, `asset`, `controller`, `pool` |
| `DataKey::VaultAccount(vault)` | persistent | The vault's controller `account_id` |

- A full withdrawal clears `VaultAccount`; the next deposit opens a new
  account. Vaults never share an account.
- The mapping TTL extends to 120 days (`TTL_BUMP_USER`) below 30 days
  (`TTL_THRESHOLD_USER`), the same tier as the controller account.

## Events

`harvest` publishes `HarvestEvent`; see
[events](../../docs/reference/events.md#defindex-strategy-events).

## Layout

```text
src/
  lib.rs   Strategy trait, vault ↔ account mapping, TTL extension on read
```

## References

- Controller `supply` and `withdraw`: [`contracts/controller`](../controller/README.md)
- Errors: [DeFindex strategy errors](../../docs/reference/errors.md#defindex-strategy)
