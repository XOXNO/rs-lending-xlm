---
name: xoxno-lending-contracts
description: Use when a Soroban contract must call XOXNO Lending on-chain, own or renew an account, compose controller verbs, or implement a flash callback.
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
- Interface locations and integration-specific ABI interpretation: [abi.md](abi.md)
- Canonical error names, codes, and remedies:
  [`../../docs/reference/errors.md`](../../docs/reference/errors.md)
- Route payload construction:
  [`../xoxno-swap-aggregator/composition.md`](../xoxno-swap-aggregator/composition.md)

## Dependencies

The ABI crates are git dependencies. Pin all of them and `soroban-sdk` to the
same release used by the deployment:

```toml
[dependencies]
soroban-sdk = "=27.0.6"
controller-interface = { git = "https://github.com/XOXNO/rs-lending-xlm", tag = "v1.0.0" }
common = { git = "https://github.com/XOXNO/rs-lending-xlm", tag = "v1.0.0" }
```

Add `pool-interface`, `position-nft-interface`, or
`swap-aggregator-interface` only when calling those contracts directly. The
authoritative signatures are the traits under
[`../../interfaces/`](../../interfaces/), especially
[`ControllerInterface`](../../interfaces/controller/src/lib.rs).

Resolve the controller and other deployed contracts from
[`../xoxno-lending/addresses.md`](../xoxno-lending/addresses.md), pass them to
the constructor, and resolve the pool with `get_pool_address()`. Do not embed a
deployment address in Wasm.

## Contract caller rules

1. Pass `env.current_contract_address()` as `caller`. Direct invoker auth
   satisfies `caller.require_auth()`.
2. Before `supply`, `repay`, `liquidate`, or `recapitalize`, authorize the
   exact nested token transfer from your contract to the pool with
   `common::token::authorize_transfer_as_current`.
3. Run every controller read before that authorization. The controller verb
   must be the next cross-contract call.
4. `multiply` initial payment is the exception: authorize a transfer to the
   controller, not the pool.
5. Renew your contract instance in every public entrypoint and callback with
   `common::ttl::renew_instance(&env)`.
6. Persist account IDs in local persistent storage and renew those keys on
   every successful use. `renew_account` renews controller/NFT state; it does
   **not** renew storage owned by the caller contract.

The complete local pointer renew/reconcile pattern is in
[positions.md](positions.md#canonical-local-account-pointer). Production code
using the same pattern is
[`contracts/defindex-strategy/src/lib.rs`](../../contracts/defindex-strategy/src/lib.rs).

## Minimal call shape

```rust
use common::token::authorize_transfer_as_current;
use common::ttl::renew_instance;
use common::types::HubAssetKey;
use controller_interface::ControllerClient;
use soroban_sdk::{vec, Address, Env};

pub fn supply_from_contract(
    env: Env,
    controller: Address,
    pool: Address,
    account_id: u64,
    spoke_id: u32,
    market: HubAssetKey,
    amount: i128,
) -> u64 {
    renew_instance(&env);
    let me = env.current_contract_address();
    let client = ControllerClient::new(&env, &controller);

    // Reconcile and renew the local pointer before this authorization.
    authorize_transfer_as_current(&env, &market.asset, &me, &pool, amount);
    client.supply(
        &me,
        &account_id,
        &spoke_id,
        &vec![&env, (market, amount)],
    )
}
```

The fragment intentionally leaves pointer lookup/storage to the canonical
helper. A production entrypoint must call that helper before authorization and
store the returned ID afterward.

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
  explicit `false`
- flash callback: pool/controller settlement succeeds after the callback
  (allowance was pulled for a cash flash loan, or declared collateral was
  measured and deposited for a flash position)

Do not infer successful completion from a callback return, a submitted
transaction hash, or a zero balance alone.
