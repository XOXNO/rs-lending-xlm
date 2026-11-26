---
name: xoxno-lending-contracts
description: "Use when a Soroban contract must call XOXNO Lending on-chain, own or renew an account, compose controller verbs, implement a flash callback, or test the integration with the xoxno-contract-sdk fixture."
user-invocable: true
argument-hint: "[what the contract must do on XOXNO Lending]"
---

# XOXNO Lending from Soroban contracts

This skill is for a contract that calls the controller. For the protocol model,
units, and addresses, see
[`../xoxno-lending/SKILL.md`](../xoxno-lending/SKILL.md).

## Choose a task

Pick the file that matches your task:

- Account creation, local pointer TTL, reconciliation, ownership, delegation,
  discovery, and DeFindex adapter PPS: [positions.md](positions.md)
- Flash-loan and flash-position callbacks: [flash-loans.md](flash-loans.md)
- Atomic controller operations, swaps, liquidation, and keeper batches:
  [composing.md](composing.md)
- Tests on the embedded protocol WASM with `LendingFixture`:
  [testing.md](testing.md)
- Interface locations and ABI interpretation: [abi.md](abi.md)
- Error names, codes, and remedies:
  [`docs/reference/errors.md`](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/errors.md)
- Route payload construction:
  [`../xoxno-swap-aggregator/composition.md`](../xoxno-swap-aggregator/composition.md)

## Dependencies

`xoxno-contract-sdk` (crates.io, MIT, `soroban-sdk` 28) embeds protocol
release WASM:

```toml
[dependencies]
soroban-sdk = "28"
xoxno-contract-sdk = "0.2"

[dev-dependencies]
soroban-sdk = { version = "28", features = ["testutils"] }
xoxno-contract-sdk = { version = "0.2", features = ["testutils"] }
```

Verified crate `0.2.0` embeds protocol `v1.1.0`, commit `1053ae033`. Match
the embedded release to the target deployment (the crate README has a
compatibility table). Build with `stellar contract build`. `soroban-sdk` 28
needs stellar-cli 25.2 or newer.

The crate contains:

- `XoxnoLending` is the wrapper. The current contract is caller, payer, and
  receiver. Its `open_account`, `deposit`, `supply`, `repay`, and `liquidate`
  create the nested token-transfer authorization.
- Generated clients and types in `xoxno_contract_sdk::lending::{controller,
  pool, position_nft, price_aggregator}`. `lending::ControllerClient` is the
  controller client. Arguments pass by reference.
- `lending::helpers::authorize_transfer_as_current(env, token, from, to,
  amount)`, for token-pulling generated-client calls.
- Flash callback traits `FlashLoanReceiver`, `FlashPositionReceiver`, and
  `lending::helpers::approve_flash_repayment`.
- `lending::constants` and the test fixture `testutils::LendingFixture`.

Each generated module has its own non-convertible copy of the shared types.
Controller calls use `lending::controller` types such as
`controller::HubAssetKey` and `controller::PositionMode`. Never duplicate a
`#[contracttype]` locally, because the copy can drift.

Clients cover only integrator functions. These are controller user, keeper, and
view functions, plus read-only pool, NFT, and price-aggregator views. The
crate has no swap router client. See
[`../xoxno-swap-aggregator/payload.md`](../xoxno-swap-aggregator/payload.md).
The source signatures are in the traits under
[`interfaces/`](https://github.com/XOXNO/rs-lending-xlm/tree/main/interfaces/), especially
[`ControllerInterface`](https://github.com/XOXNO/rs-lending-xlm/blob/main/interfaces/controller/src/lib.rs).

Pass controller and position NFT addresses from
[`../xoxno-lending/addresses.md`](../xoxno-lending/addresses.md), or a
`LendingAddresses`, to your constructor. Resolve the pool with
`get_pool_address()`, or check a passed pool address against it, and store all
three as `LendingAddresses`. Never embed a deployment address in Wasm. Build
the wrapper with `XoxnoLending::new` from stored addresses. Do not use
`XoxnoLending::mainnet` or `XoxnoLending::testnet`, which compile in the
crate's address constants.

## Storage TTL

Your contract renews its own instance and persistent keys. At about 5 s per
ledger, one day is 17,280 ledgers. These are the protocol values for its
instance and account entries:

| Setting | Days | Ledgers |
| --- | --- | --- |
| Renewal threshold | 30 | 518,400 |
| Instance extension | 180 | 3,110,400 |
| Account pointer extension | 120 | 2,073,600 |

```rust
const DAY_IN_LEDGERS: u32 = 17_280;
const INSTANCE_TTL_THRESHOLD: u32 = 30 * DAY_IN_LEDGERS;
const INSTANCE_TTL_EXTEND_TO: u32 = 180 * DAY_IN_LEDGERS;
const POINTER_TTL_THRESHOLD: u32 = 30 * DAY_IN_LEDGERS;
const POINTER_TTL_EXTEND_TO: u32 = 120 * DAY_IN_LEDGERS;
```

`extend_ttl` does nothing while the remaining TTL exceeds the threshold. The
threshold must not exceed the extend-to value. The host clamps the value to the
network maximum entry TTL.

## Contract caller rules

1. Pass `env.current_contract_address()` as `caller`. Direct invoker auth
   satisfies `caller.require_auth()`.
2. Before a token pull, authorize the exact nested transfer from
   [composing.md](composing.md#token-pull-ordering). The transfer goes to the
   pool for `supply`, `repay`, `liquidate`, and `recapitalize`, and to the
   controller for a `multiply` initial payment. Wrapper verbs authorize the
   transfer themselves. Generated-client calls, including `recapitalize`, use
   `authorize_transfer_as_current`.
3. Finish all contract reads before you create the authorization. The
   controller operation is your next contract call.
4. Renew your instance in every public entrypoint and callback:
   `env.storage().instance().extend_ttl(INSTANCE_TTL_THRESHOLD,
   INSTANCE_TTL_EXTEND_TO)`.
5. Persist account IDs locally and renew those keys on every successful use.
   `renew_account` renews controller and NFT state, not your storage.
6. An entrypoint that spends your contract's balance or borrowing power must
   follow three rules. The spending actions are supply, repay, borrow,
   withdraw, and token transfer out. The entrypoint authorizes a stored
   address, never a caller-passed one. It takes the account ID from storage. It
   sends tokens only to a stored address or back to whoever supplied them.

The pointer pattern is in
[positions.md](positions.md#canonical-local-account-pointer). For production
use, see
[`contracts/defindex-strategy/src/lib.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/defindex-strategy/src/lib.rs).

## Minimal call shape

Use this code with the generated client:

```rust
use soroban_sdk::{contracttype, vec, Address, Env};
use xoxno_contract_sdk::lending::controller::HubAssetKey;
use xoxno_contract_sdk::lending::helpers::authorize_transfer_as_current;
use xoxno_contract_sdk::lending::ControllerClient;
use xoxno_contract_sdk::LendingAddresses;

#[contracttype]
pub enum ConfigKey {
    Admin,
    Lending,
}

pub fn supply_from_contract(env: Env, spoke_id: u32, market: HubAssetKey, amount: i128) -> u64 {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_THRESHOLD, INSTANCE_TTL_EXTEND_TO);
    let config = env.storage().instance();
    let admin: Address = config.get(&ConfigKey::Admin).expect("set in constructor");
    admin.require_auth();
    let lending: LendingAddresses = config.get(&ConfigKey::Lending).expect("set in constructor");
    let me = env.current_contract_address();
    let client = ControllerClient::new(&env, &lending.controller);
    let account_id = resolve_account(&env, &client);

    authorize_transfer_as_current(&env, &market.asset, &me, &lending.pool, amount);
    let account_id = client.supply(&me, &account_id, &spoke_id, &vec![&env, (market, amount)]);
    store_account(&env, account_id);
    account_id
}
```

With the wrapper, the part after `admin.require_auth()` becomes:

```rust
    let addresses: LendingAddresses = config.get(&ConfigKey::Lending).expect("set in constructor");
    let lending = XoxnoLending::new(&env, &addresses);
    let account_id = resolve_account(&env, &lending.controller());

    let account_id = lending.deposit(account_id, spoke_id, &market, amount);
    store_account(&env, account_id);
    account_id
```

The resolve and store helpers are in
[positions.md](positions.md#canonical-local-account-pointer). The resolve read
precedes the authorization. The controller verb is the next cross-contract
call.

`deposit` with ID `0` opens an account in `spoke_id`. With an existing ID,
`spoke_id` must match the account's spoke. `supply(account_id, market, amount)`
reads the spoke first. `open_account(spoke_id, market, amount)` always opens
a new account.

## Completion checks

Before submission, the exact transaction, including its auth tree and
footprint, must simulate successfully. Simulation shows feasibility only.
Completion is a confirmed `SUCCESS` plus these checks:

- account creation: the pointer is stored and its TTL is extended, and
  `account_exists(id)` is true
- account reuse: `owner_of(u32::try_from(id))` is the expected owner, and
  `get_account_attributes(id)` has the expected mode and spoke
- token pull or strategy: the returned amounts, the returned IDs, and the
  resulting balances or positions match the requested branch
- full exit: reconcile `account_exists`. Clear the pointer only on an explicit
  `false` or, through the wrapper, `Withdrawal::account_closed`
- flash callback: [settlement](flash-loans.md#settlement-completion) succeeds

A callback return, submitted hash, or zero balance alone proves nothing.
