# DeFindex Strategy

This contract is a DeFindex vault adapter over the controller, with **one
vault, one controller account**. Deposits and withdrawals move supply
collateral. `harvest` claims no external yield. It only emits the supply index
as price per share.

| | |
| --- | --- |
| Config | `hub_id`, `spoke_id`, `asset`, `controller`, `pool` |
| Mapping | `VaultAccount(vault)` maps to the controller `account_id` |
| Client | Uses [`interfaces/controller`](../../interfaces/controller). It has no client of its own |

## Authorization

`deposit`, `withdraw`, and `harvest` require auth from `from`, the vault. The
strategy owns each vault's controller account and acts as its owner. Views
need no auth.

The strategy does not check that `from` is a DeFindex vault. Any address that
authorizes a deposit gets its own supply-only account, separate from any
account that address holds on the controller.

## Entrypoints

The [endpoint reference](../../docs/reference/endpoints.md#defindex-strategy)
lists signatures and returns. The constructor checks the spoke and its listing
of the asset. `deposit` pulls tokens and calls controller `supply`. `withdraw`
calls controller `withdraw` and pays `to`. `harvest` emits price per share (12 decimals, rounded
down) with amount 0.

## Storage and TTL

| Key | Tier | Holds |
| --- | --- | --- |
| `DataKey::Config` | instance | `hub_id`, `spoke_id`, `asset`, `controller`, `pool` |
| `DataKey::VaultAccount(vault)` | persistent | The vault's controller `account_id` |

- A full withdrawal clears `VaultAccount`. The next deposit opens a new
  account. Vaults never share an account.
- The mapping TTL extends to 120 days (`TTL_BUMP_USER`) below 30 days
  (`TTL_THRESHOLD_USER`), the same tier as the controller account.

## Events

`harvest` publishes `HarvestEvent`. See
[events](../../docs/reference/events.md#defindex-strategy-events).

## Layout

```text
src/
  lib.rs   Strategy trait, vault ↔ account mapping, TTL extension on read
```

## References

- Controller `supply` and `withdraw`: [`contracts/controller`](../controller/README.md)
- Errors: [DeFindex strategy errors](../../docs/reference/errors.md#defindex-strategy)
