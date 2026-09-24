---
name: xoxno-lending-contracts
description: Use when a Soroban contract must call XOXNO Lending on-chain, own or renew an account, compose controller verbs, implement a flash callback, or test the integration with the xoxno-contract-sdk fixture.
user-invocable: true
argument-hint: "[what the contract must do on XOXNO Lending]"
---

# XOXNO Lending from Soroban contracts

Use this skill when the controller caller is another contract. Start with the
protocol model, units, and deployed addresses in
[`../xoxno-lending/SKILL.md`](../xoxno-lending/SKILL.md).

## Route by task

- Account creation, local pointer TTL, reconciliation, ownership, delegation,
  discovery, and DeFindex adapter PPS: [positions.md](positions.md)
- Flash-loan and flash-position callbacks: [flash-loans.md](flash-loans.md)
- Atomic multi-verb flows, swaps, liquidation, and keeper batching:
  [composing.md](composing.md)
- Tests on the embedded protocol WASM with `LendingFixture`:
  [testing.md](testing.md)
- Interface locations and integration-specific ABI interpretation: [abi.md](abi.md)
- Canonical error names, codes, and remedies:
  [`../../docs/reference/errors.md`](../../docs/reference/errors.md)
- Route payload construction:
  [`../xoxno-swap-aggregator/composition.md`](../xoxno-swap-aggregator/composition.md)

## Dependencies

Depend on the `xoxno-contract-sdk` crate from crates.io. It is MIT licensed,
uses `soroban-sdk` 28, and embeds the WASM of an attested rs-lending-xlm
release:

```toml
[dependencies]
soroban-sdk = "28"
xoxno-contract-sdk = "0.2"

[dev-dependencies]
soroban-sdk = { version = "28", features = ["testutils"] }
xoxno-contract-sdk = { version = "0.2", features = ["testutils"] }
```

Use the crate version whose embedded release is the deployment you call. The
compatibility table is in the crate README. Build the contract with
`stellar contract build`; `soroban-sdk` 28 needs stellar-cli 25.2 or newer.

The crate contains:

- `XoxnoLending`: a wrapper in which the current contract is the caller, the
  payer, and the receiver. Its `open_account`, `deposit`, `supply`, `repay`,
  and `liquidate` create the nested token-transfer authorization.
- Generated clients and types in `xoxno_contract_sdk::lending::{controller,
  pool, position_nft, price_aggregator}`. `lending::ControllerClient` is the
  controller client. Arguments pass by reference.
- `lending::helpers::authorize_transfer_as_current(env, token, from, to,
  amount)` for a generated-client call that pulls tokens.
- The flash callback traits `FlashLoanReceiver` and `FlashPositionReceiver`,
  and `lending::helpers::approve_flash_repayment`.
- `lending::constants`, and the test fixture `testutils::LendingFixture`.

Each generated module has its own copy of the shared types, and the copies do
not convert into each other. When you call the controller, use the
`lending::controller` types, for example `controller::HubAssetKey` and
`controller::PositionMode`.

The generated clients hold only the functions an integrator calls: the
controller user, keeper, and view functions, and read-only views of the pool,
the NFT, and the price aggregator. The crate has no swap router client; see
[`../xoxno-swap-aggregator/payload.md`](../xoxno-swap-aggregator/payload.md).
The source signatures are the traits under
[`../../interfaces/`](../../interfaces/), especially
[`ControllerInterface`](../../interfaces/controller/src/lib.rs).

Resolve the controller and position NFT addresses from
[`../xoxno-lending/addresses.md`](../xoxno-lending/addresses.md) and pass them,
or a `LendingAddresses` value, to the constructor. Resolve the pool with
`get_pool_address()`, or check a passed pool against it, and store the three
as `LendingAddresses`. Do not embed a deployment address in Wasm. Build
the wrapper with `XoxnoLending::new` from the stored addresses, not with
`XoxnoLending::mainnet` or `XoxnoLending::testnet`: these compile the crate's
address constants into your contract.

## Storage TTL

Your contract owns its instance and its persistent keys, and it renews them
with its own constants. A ledger closes about every 5 seconds, so one day is
86,400 / 5 = 17,280 ledgers. The values below are the ones the protocol uses
for its own instance and per-user entries: renew below 30 days
(518,400 ledgers), and extend the instance to 180 days (3,110,400 ledgers) and
an account pointer to 120 days (2,073,600 ledgers).

```rust
const DAY_IN_LEDGERS: u32 = 17_280;
const INSTANCE_TTL_THRESHOLD: u32 = 30 * DAY_IN_LEDGERS;
const INSTANCE_TTL_EXTEND_TO: u32 = 180 * DAY_IN_LEDGERS;
const POINTER_TTL_THRESHOLD: u32 = 30 * DAY_IN_LEDGERS;
const POINTER_TTL_EXTEND_TO: u32 = 120 * DAY_IN_LEDGERS;
```

`extend_ttl` does nothing while the remaining TTL is above the threshold. The
threshold must not be larger than the extend-to value. The host clamps an
instance or persistent extension to the network maximum entry TTL.

## Contract caller rules

1. Pass `env.current_contract_address()` as `caller`. Direct invoker auth
   satisfies `caller.require_auth()`.
2. Before `supply`, `repay`, `liquidate`, or `recapitalize`, authorize the
   exact nested token transfer from your contract to the pool. The wrapper's
   `open_account`, `deposit`, `supply`, `repay`, and `liquidate` do this. For
   a generated-client call, including `recapitalize`, call
   `authorize_transfer_as_current`.
3. Run every controller read before that authorization. The controller verb
   must be the next cross-contract call.
4. `multiply` initial payment is the exception: authorize a transfer to the
   controller, not the pool.
5. Renew your contract instance in every public entrypoint and callback with
   `env.storage().instance().extend_ttl(INSTANCE_TTL_THRESHOLD,
   INSTANCE_TTL_EXTEND_TO)`.
6. Persist account IDs in local persistent storage and renew those keys on
   every successful use. `renew_account` renews controller/NFT state; it does
   not renew storage owned by the caller contract.
7. A public entrypoint that spends your contract's own balance or borrowing
   power (supply, repay, borrow, withdraw, or a token transfer out) must
   authorize against an address your contract stored, never one the caller
   passes. Take your own account ID from your storage, and send tokens only
   to a stored address or back to the address whose tokens you pulled.

The complete local pointer renew/reconcile pattern is in
[positions.md](positions.md#canonical-local-account-pointer). Production code
using the same pattern is
[`contracts/defindex-strategy/src/lib.rs`](../../contracts/defindex-strategy/src/lib.rs).

## Minimal call shape

With the generated client:

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
[positions.md](positions.md#canonical-local-account-pointer). The resolve
read comes before the authorization, and the controller verb is the next
cross-contract call.

`deposit` with account ID `0` opens a new account in `spoke_id`. With an
existing ID, `spoke_id` must be the account's spoke. `supply(account_id,
market, amount)` reads the spoke first, and `open_account(spoke_id, market,
amount)` always opens a new account.

## Completion checks

Before submission, require successful simulation of the exact transaction,
including its auth tree and footprint. Simulation is feasibility evidence, not
completion.

Completion requires confirmed transaction `SUCCESS` plus the applicable
operation-specific return and state checks:

- account-creating call: local pointer was stored and its TTL extended;
  `account_exists(id)` is true
- account reuse: NFT `owner_of(u32::try_from(id))` is the expected owner and
  `get_account_attributes(id)` has the expected mode and spoke
- token-pulling or strategy call: returned amounts/IDs and resulting balances
  or positions match the requested branch
- full exit: reconcile `account_exists`; clear the local pointer only on an
  explicit `false`, or, on the wrapper path, on `Withdrawal::account_closed`
- flash callback: pool/controller settlement succeeds after the callback
  (allowance was pulled for a cash flash loan, or declared collateral was
  measured and deposited for a flash position)

Do not infer successful completion from a callback return, a submitted
transaction hash, or a zero balance alone.
