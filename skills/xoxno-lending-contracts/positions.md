# Accounts and positions from a contract

The position NFT owner is the account owner. `account_id` is the NFT token ID
widened from `u32` to `u64`. The `PositionMode` and spoke of an account are
fixed at creation. Check the owner, mode, and spoke before you reuse an ID.

## Lifecycle

- ID `0` creates an account through `supply`, `multiply`, `flash_position`,
  `migrate_from_blend`, or liquidation `Credit(0)`. Store the returned ID.
- `account_exists(id)` checks and renews only the controller's
  `AccountMeta(id)`. It does not renew or prove the existence of position maps,
  delegates, NFT entries, your local pointer, or your contract instance.
- Cleanup depends on the operation. `withdraw` and the strategy verbs remove
  the account when both position maps end empty. Liquidation and
  `clean_bad_debt` remove it through their own paths. `repay` writes only the
  debt side and does not remove an empty account.
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
use soroban_sdk::{contracterror, contracttype, panic_with_error, Env};
use xoxno_contract_sdk::lending::ControllerClient;

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
        POINTER_TTL_THRESHOLD,
        POINTER_TTL_EXTEND_TO,
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
                POINTER_TTL_THRESHOLD,
                POINTER_TTL_EXTEND_TO,
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

The TTL constants are in [SKILL.md](SKILL.md#storage-ttl). With the wrapper,
pass `&lending.controller()` as the client.

Call the resolve helper before any nested token authorization, call the
controller verb, then call the store helper with a returned ID. Reconcile after
an operation that can delete the account. Clear only on `Ok(Ok(false))`; a
host or decode failure does not prove the account is gone.

`XoxnoLending::resolve_account(stored)` returns the same branch: the stored ID
while `account_exists` is true, else `NEW_ACCOUNT` (`0`), and a failed lookup
aborts the call. It does not extend or clear your key. If you use it, extend
the key on every successful use and overwrite it with the returned ID.

For per-owner adapters use
`DataKey::VaultAccount(Address)` exactly as the DeFindex strategy does, and
extend the specific owner key on every successful lookup and write.

## Full exit with the wrapper

The wrapper's `withdraw` and `withdraw_all` return
`Withdrawal { amount, account_closed }`. `account_closed` is true only when
the position NFT's `owner_of` fails with `NonExistentToken`: the protocol
deleted the account and burned its NFT. That is the explicit non-existence
signal on this path. Any other lookup failure gives `false`, and the key
stays.

```rust
use soroban_sdk::{token, Address, Env};
use xoxno_contract_sdk::lending::controller::HubAssetKey;
use xoxno_contract_sdk::{LendingAddresses, XoxnoLending};

pub fn withdraw_all_to_admin(env: Env, market: HubAssetKey) -> i128 {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
    let config = env.storage().instance();
    let admin: Address = config.get(&ConfigKey::Admin).expect("set in constructor");
    admin.require_auth();
    let addresses: LendingAddresses = config.get(&ConfigKey::Lending).expect("set in constructor");
    let lending = XoxnoLending::new(&env, &addresses);
    let account_id = resolve_account(&env, &lending.controller());

    let withdrawal = lending.withdraw_all(account_id, &market);
    if withdrawal.account_closed {
        env.storage().persistent().remove(&DataKey::AccountId);
    }
    token::Client::new(&env, &market.asset).transfer(
        &env.current_contract_address(),
        &admin,
        &withdrawal.amount,
    );
    withdrawal.amount
}
```

The config keys are the ones in [SKILL.md](SKILL.md#minimal-call-shape). The
withdrawal closes the account only when no other supply or debt remains. The
tokens go to the stored admin, not to an address the caller passes.

## Renewal responsibilities

`renew_account(owner, id)` is NFT-owner-only. It extends the controller
instance, account metadata, existing supply/debt/delegate maps, and NFT
owner/balance state. It cannot touch the integrating contract's storage.

`PositionNft::renew(token_id)` is permissionless and extends NFT state only.
A keeper can call it directly. The crate's `position_nft` client has
ownership views only, so a keeper contract calls `renew` by name:

```rust
env.invoke_contract::<()>(
    &position_nft,
    &Symbol::new(&env, "renew"),
    vec![&env, token_id.into_val(&env)],
);
```

To renew controller account state, the NFT holder contract must expose an
authenticated forwarding entrypoint:

```rust
use soroban_sdk::{contracttype, Address, Env};
use xoxno_contract_sdk::lending::ControllerClient;
use xoxno_contract_sdk::LendingAddresses;

#[contracttype]
pub enum ConfigKey {
    Admin,
    Lending,
}

pub fn renew_owned_account(env: Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
    let config = env.storage().instance();
    let admin: Address = config.get(&ConfigKey::Admin).expect("set in constructor");
    admin.require_auth();
    let lending: LendingAddresses = config.get(&ConfigKey::Lending).expect("set in constructor");
    let client = ControllerClient::new(&env, &lending.controller);
    let account_id = resolve_account(&env, &client);
    if account_id != 0 {
        client.renew_account(&env.current_contract_address(), &account_id);
    }
}
```

Read the admin and controller addresses from your own storage, not from
arguments. A caller-supplied controller can return `false` from
`account_exists` and make the resolve helper clear your pointer.

This entrypoint renews three lifetimes: the instance `extend_ttl` renews its
own instance, the resolve helper renews the local pointer, and
`renew_account` renews controller and NFT state.

Renew before expiry. A simulated and assembled transaction restores archived
entries inline, and the submitter pays restore rent. Only a hand-built
footprint needs a separate `RestoreFootprint` operation. See
[archived entries](../xoxno-lending-troubleshooting/SKILL.md#archived-entries).

## Ownership and delegation

Before owner/delegate or owner-only work on a stored ID:

1. require `account_exists(id)`
2. read NFT `owner_of(u32::try_from(id))`
3. read `get_account_attributes(id)`
4. require expected owner, mode, and spoke

These checks are preconditions and completion checks for your contract. They
do not replace controller authorization. An NFT transfer moves the account,
collateral, and debt. A grant from the previous owner is inactive after the
transfer.

An owner may call `add_delegate` only for a governance-activated position
manager. Delegates may borrow or withdraw to arbitrary recipients, so grant
only to controlled code. A keeper renewing only NFT TTL does not need a
delegate grant.

The NFT enumerable interface is the only owner-to-account reverse index:
`balance(owner)` followed by `get_owner_token_id(owner, index)`. Treat the
result as a snapshot and re-check `owner_of` before acting.

## Reading positions

- `get_collateral_amount` / `get_borrow_amount`: accrued token base units,
  rounded half up
- `get_account_positions`: raw maps; `scaled_amount` is RAY-scaled shares, and
  supply entries also carry their BPS risk parameters
- `get_health_factor`: WAD; `i128::MAX` means no debt or no account
- `get_market_index`: accrued RAY indexes, no oracle lookup

Read token amounts from these views, or from the wrapper's `collateral`,
`debt`, and `position`, which call them. Do not mix RAY shares, indexes, and
token decimals by hand. A full withdrawal pays the floor of the claim, which
can be one base unit below the half-up view. A full repayment needs the
ceiling of the debt, which can be one base unit above it. When your contract
must floor what a user can claim, or ceil what a user owes, compute it as in
[`../xoxno-lending/math.md`](../xoxno-lending/math.md#shares-and-token-amounts).
Sizing guidance is in the same file.

## DeFindex adapter PPS convention

This is not a generic vault formula. The DeFindex adapter treats one of its
shares as one scaled supply share of the lending account. Its reported PPS is
therefore the supply index, floor-rescaled from RAY to 12 decimals:

```rust
use xoxno_contract_sdk::lending::constants::RAY;

const PPS_DECIMALS: u32 = 12;

let index = lending.supply_index(&market);
let defindex_pps = index / (RAY / 10_i128.pow(PPS_DECIMALS));
```

`supply_index` is the RAY supply index from `get_market_index`, and it is
positive. `RAY / 10^12` is exactly `10^15`. Integer division truncates toward
zero, which is the floor for a positive index, and it cannot overflow.

Use this only when implementing the same adapter accounting convention. A
vault with fees, multiple assets, idle balances, or a different share model
must compute its own total-assets/share ratio.
