# Accounts and positions from a contract

Account ownership is the position NFT. `account_id` is the NFT token ID widened
from `u32` to `u64`; ownership, fixed `PositionMode`, and fixed spoke must be
checked before reusing an ID.

## Lifecycle

- ID `0` creates an account through `supply`, `multiply`, `flash_position`,
  `migrate_from_blend`, or liquidation `Credit(0)`. Store the returned ID.
- `account_exists(id)` checks and renews only the controller's
  `AccountMeta(id)`. It does not renew or prove the existence of position maps,
  delegates, NFT entries, your local pointer, or your contract instance.
- Cleanup is operation-specific. A full withdrawal can remove an empty
  account; liquidation and bad-debt cleanup have their own removal paths;
  strategy close paths may remove after their own checks. `repay` persists the
  debt side and does not automatically remove an otherwise empty account.
- Deletion removes controller entries and burns the NFT atomically. IDs are
  not reused.

See
[`contracts/controller/src/account.rs`](../../contracts/controller/src/account.rs),
[`positions/supply.rs`](../../contracts/controller/src/positions/supply.rs),
and
[`positions/debt.rs`](../../contracts/controller/src/positions/debt.rs).

## Canonical local account pointer

This is the caller-owned half of account lifetime. It is adapted from
[`resolve_vault_account`](../../contracts/defindex-strategy/src/lib.rs):

```rust
use common::constants::{TTL_BUMP_USER, TTL_THRESHOLD_USER};
use controller_interface::ControllerClient;
use soroban_sdk::{contracterror, contracttype, panic_with_error, Env};

#[contracttype]
pub enum DataKey {
    AccountId,
}

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalError {
    AccountLookupFailed = 1,
}

fn store_account(env: &Env, account_id: u64) {
    let storage = env.storage().persistent();
    storage.set(&DataKey::AccountId, &account_id);
    storage.extend_ttl(
        &DataKey::AccountId,
        TTL_THRESHOLD_USER,
        TTL_BUMP_USER,
    );
}

fn resolve_account(env: &Env, controller: &ControllerClient) -> u64 {
    let storage = env.storage().persistent();
    let account_id = storage.get(&DataKey::AccountId).unwrap_or(0);
    if account_id == 0 {
        return 0;
    }

    match controller.try_account_exists(&account_id) {
        Ok(Ok(true)) => {
            storage.extend_ttl(
                &DataKey::AccountId,
                TTL_THRESHOLD_USER,
                TTL_BUMP_USER,
            );
            account_id
        }
        Ok(Ok(false)) => {
            storage.remove(&DataKey::AccountId);
            0
        }
        _ => panic_with_error!(env, LocalError::AccountLookupFailed),
    }
}
```

Call the resolve helper before any nested token authorization, call the
controller verb, then call the store helper with a returned ID. Reconcile after
an operation that can delete the account. Clear only on `Ok(Ok(false))`; a
host or decode failure does not prove the account is gone.

For per-owner adapters use
`DataKey::VaultAccount(Address)` exactly as the DeFindex strategy does, and
extend the specific owner key on every successful lookup and write.

## Renewal responsibilities

`renew_account(owner, id)` is NFT-owner-only. It extends the controller
instance, account metadata, existing supply/debt/delegate maps, and NFT
owner/balance state. It cannot touch the integrating contract's storage.

`PositionNft::renew(token_id)` is permissionless and extends NFT state only.
A keeper can call it directly. To renew controller account state, the NFT
holder contract must expose an authenticated forwarding entrypoint:

```rust
use common::ttl::renew_instance;
use controller_interface::ControllerClient;
use soroban_sdk::{Address, Env};

pub fn renew_owned_account(
    env: Env,
    admin: Address,
    controller: Address,
) {
    renew_instance(&env);
    admin.require_auth();
    let client = ControllerClient::new(&env, &controller);
    let account_id = resolve_account(&env, &client);
    if account_id != 0 {
        client.renew_account(&env.current_contract_address(), &account_id);
    }
}
```

This entrypoint renews three distinct lifetimes: its own instance, the resolve
helper renews the local persistent pointer, and `renew_account`
renews controller/NFT state.

Archived entries require a Soroban restore-footprint operation before they can
be extended. Schedule renewal before expiry.

## Ownership and delegation

Before owner/delegate or owner-only work on a stored ID:

1. require `account_exists(id)`
2. read NFT `owner_of(u32::try_from(id))`
3. read `get_account_attributes(id)`
4. require expected owner, mode, and spoke

These are branch-specific completion/precondition checks, not substitutes for
controller authorization. NFT transfer moves the account, collateral, and
debt. A grant from the previous owner is inactive after transfer.

An owner may call `add_delegate` only for a governance-activated position
manager. Delegates may borrow or withdraw to arbitrary recipients, so grant
only to controlled code. A keeper renewing only NFT TTL does not need a
delegate grant.

The NFT enumerable interface is the only owner-to-account reverse index:
`balance(owner)` followed by `get_owner_token_id(owner, index)`. Treat the
result as a snapshot and re-check `owner_of` before acting.

## Reading positions

- `get_collateral_amount` / `get_borrow_amount`: accrued token base units
- `get_account_positions`: RAY-scaled raw shares
- `get_health_factor`: WAD; `i128::MAX` means no debt or no account
- `get_market_index`: accrued RAY indexes, no oracle lookup

Use the helpers in `common::rates`; do not manually mix RAY shares, indexes,
and token decimals. Sizing guidance is in
[`../xoxno-lending/math.md`](../xoxno-lending/math.md).

## DeFindex adapter PPS convention

This is not a generic vault formula. The production DeFindex adapter defines
its own shares as one-to-one with the lending account's scaled supply shares,
so its reported PPS is the supply index floor-rescaled from RAY to the
adapter's 12-decimal convention:

```rust
use common::math::fp::Ray;

let index = controller.get_market_index(&market).supply_index;
let defindex_pps = Ray::from(index).to_asset_floor(&env, 12);
```

Use this only when implementing the same adapter accounting convention. A
vault with fees, multiple assets, idle balances, or a different share model
must compute its own total-assets/share ratio.
