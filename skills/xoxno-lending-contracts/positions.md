# Accounts and positions from a contract

The position NFT owner owns the account; `account_id` is the NFT token ID
widened from `u32` to `u64`. `PositionMode` and spoke are fixed at creation.
Check owner, mode, and spoke before reusing an ID.

## Lifecycle

- ID `0` creates an account through `supply`, `multiply`, `flash_position`,
  `migrate_from_blend`, or liquidation `Credit(0)`. Store the returned ID.
- `account_exists(id)` checks and renews only the controller's
  `AccountMeta(id)`, not position maps, delegates, NFT entries, your local
  pointer, or your instance.
- `withdraw` and strategy verbs remove the account when both position maps
  end empty; liquidation and `clean_bad_debt` use their own paths; `repay`
  writes only debt and never removes an empty account
  ([cleanup](composing.md#operation-specific-cleanup)).
- Deletion removes controller entries and burns the NFT atomically. IDs are
  never reused.

Sources: [`contracts/controller/src/account.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/account.rs),
[`positions/supply.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/positions/supply.rs),
and
[`positions/debt.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/positions/debt.rs).

## Canonical local account pointer

These helpers store and renew your account ID, as
[`resolve_vault_account`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/defindex-strategy/src/lib.rs):

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

TTL constants: [SKILL.md](SKILL.md#storage-ttl). With the wrapper, pass
`&lending.controller()` as the client.

Resolve before token authorization, call the controller, store any returned
ID. After a possibly deleting operation, check `account_exists`; remove the
stored ID only on `Ok(Ok(false))` and abort on lookup failure.

`XoxnoLending::resolve_account(stored)` returns the same branch (stored ID
while `account_exists` is true, else `NEW_ACCOUNT` (`0`); lookup failure
aborts) but neither extends nor clears your key: extend it on every
successful use and overwrite it with the returned ID.

Per-owner adapters use `DataKey::VaultAccount(Address)` like the DeFindex
strategy, extending that owner's key on every successful lookup and write.

## Full exit with the wrapper

The wrapper's `withdraw` and `withdraw_all` return
`Withdrawal { amount, account_closed }`. `account_closed` is true only when
`owner_of` fails with `NonExistentToken` (account deleted, NFT burned). Any
other lookup failure gives `false`; the key stays.

For a full exit, call `withdraw` with `i128::MAX`. In xoxno-contract-sdk 0.3
and earlier, `withdraw_all` and `WITHDRAW_ALL` send zero, which is
deprecated: a later controller release rejects it.

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
    if account_id == 0 {
        return 0;
    }

    let withdrawal = lending.withdraw(account_id, &market, i128::MAX);
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

Config keys: [SKILL.md](SKILL.md#minimal-call-shape). The account closes only
when no other supply or debt remains. Tokens go to the stored admin, not a
caller-passed address.

Return early when the resolve helper gives `0`: withdrawing from account `0`
fails and rolls back the helper's stale-pointer removal, so the pointer would
never clear.

## Renewal responsibilities

`renew_account(owner, id)` is NFT-owner-only. It extends the controller
instance, account metadata, existing supply/debt/delegate maps, and NFT
owner/balance state, never your contract's storage.

`PositionNft::renew(token_id)` is permissionless and extends NFT state only.
The crate's `position_nft` client has only ownership views, so a keeper
contract calls `renew` by name:

```rust
env.invoke_contract::<()>(
    &position_nft,
    &Symbol::new(&env, "renew"),
    vec![&env, token_id.into_val(&env)],
);
```

To renew controller account state, the NFT holder contract exposes an
authenticated forwarding entrypoint (config keys as in
[SKILL.md](SKILL.md#minimal-call-shape)):

```rust
use soroban_sdk::{Address, Env};
use xoxno_contract_sdk::lending::ControllerClient;
use xoxno_contract_sdk::LendingAddresses;

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

Read admin and controller from storage, not arguments: a caller-supplied
controller can return `false` from `account_exists` and clear your pointer.
Three lifetimes renew here: `extend_ttl` the instance, the resolve helper the
pointer, `renew_account` controller and NFT state.

Renew before expiry. A simulated, assembled transaction restores archived
entries inline at the submitter's rent; only a hand-built footprint needs a
separate `RestoreFootprint`. See
[archived entries](../xoxno-lending-troubleshooting/SKILL.md#archived-entries).

## Ownership and delegation

Before owner/delegate or owner-only work on a stored ID:

1. require `account_exists(id)`
2. read NFT `owner_of(u32::try_from(id))`
3. read `get_account_attributes(id)`
4. require expected owner, mode, and spoke

These preconditions and completion checks do not replace controller
authorization.

An NFT transfer moves account, collateral, and debt but not delegates. A
previous owner's grant is suspended and reactivates if ownership returns to
that owner while the manager registration is active and no later holder
changed the delegate list. To revoke, `remove_delegate` each delegate before
transferring. A new holder can delete a dormant grant with `remove_delegate`
(no event). No view lists delegates; track `AccountDelegateEvent` and the
current owner.

`add_delegate` accepts only a governance-activated position manager.
Delegates can borrow or withdraw to any recipient, so grant only to controlled
code. A keeper renewing only NFT TTL needs no grant.

The only owner-to-account index is NFT enumeration: `balance(owner)`, then
`get_owner_token_id(owner, index)`. It is a snapshot; re-check `owner_of`
before acting.

## Reading positions

View units: [abi.md](abi.md#views-that-need-interpretation).
`get_account_positions` supply entries also carry BPS risk parameters. Read
token amounts from `get_collateral_amount` / `get_borrow_amount` or the
wrapper's `collateral`, `debt`, and `position`; never mix RAY shares, indexes,
and decimals by hand.

A full withdrawal pays the claim's floor, up to one base unit below the
half-up view; a full repayment needs the debt's ceiling, up to one above. For
floor/ceil math and sizing, follow
[`../xoxno-lending/math.md`](../xoxno-lending/math.md#shares-and-token-amounts).

## DeFindex adapter PPS convention

Not a generic vault formula: one DeFindex adapter share is one scaled supply
share, so PPS is the supply index floor-rescaled from RAY to 12 decimals:

```rust
use xoxno_contract_sdk::lending::constants::RAY;

const PPS_DECIMALS: u32 = 12;

let index = lending.supply_index(&market);
let defindex_pps = index / (RAY / 10_i128.pow(PPS_DECIMALS));
```

`supply_index` (from `get_market_index`) is positive RAY; `RAY / 10^12` is
exactly `10^15`; integer division floors a positive index and cannot
overflow. A vault with fees, multiple assets, idle balances, or another share
model computes its own total-assets/share ratio.
