---
name: integrating-lending-from-soroban-contracts
description: Use when a Soroban smart contract (vault, strategy, protocol) must itself supply, borrow, withdraw, repay, or read positions on XOXNO Lending — cross-contract integration in Rust, including authorization for nested token transfers and account TTL management.
---

# Integrating XOXNO Lending from Soroban Contracts

**REQUIRED BACKGROUND:** the `lending-protocol-fundamentals` skill (accounts,
spokes, HubAssetKey, units).

## Overview

Your contract calls the **controller** cross-contract and owns a lending
account like any user: it passes its own address as `caller`, stores the
returned `u64` account id, and manages TTL. The reference implementation is
`contracts/defindex-strategy/src/lib.rs` in the protocol repo — a vault
adapter doing exactly this.

## Dependencies

Add the client-only ABI crates from the protocol repo (git dependency or
vendored path); they compile to `rlib`, not WASM:

```toml
[dependencies]
controller-interface = { git = "<protocol-repo>", tag = "<release>", package = "controller-interface" }
common = { git = "<protocol-repo>", tag = "<release>", package = "common" }  # HubAssetKey, types
```

The crates are not published to crates.io — pin the git dependency to a
release tag/rev and match your `soroban-sdk` version to that revision's
workspace (see its root `Cargo.toml`).

```rust
use controller_interface::ControllerClient;
use common::types::HubAssetKey;
```

Take the controller `Address` as a constructor/config parameter (never
hardcode) and resolve the pool once via `client.get_pool_address()`.

## Authorization model

- `require_auth()` on your contract's address passes **automatically** when
  your contract is the direct invoker — no auth entries needed for the
  controller call itself.
- What is NOT automatic: nested token movements the controller performs on
  your behalf. `supply` and `repay` execute
  `token.transfer(caller → pool, amount)` — your contract must pre-authorize
  that exact sub-invocation with `authorize_as_current_contract`.
- The authorization covers only the **next** outbound contract call. No
  cross-contract call may sit between `authorize_as_current_contract` and the
  controller call, or the transfer auth is consumed/absent and the call
  reverts.
- `withdraw` and `borrow` move tokens pool → recipient; they need no token
  pre-authorization.

## Supply from a contract

```rust
use soroban_sdk::auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation};
use soroban_sdk::{vec, Address, Env, IntoVal, Symbol, Vec};

pub fn supply_to_lending(env: &Env, controller: &Address, pool: &Address,
                         hub_asset: HubAssetKey, amount: i128, spoke_id: u32,
                         stored_account_id: u64) -> u64 {
    let me = env.current_contract_address();
    let client = ControllerClient::new(env, controller);

    // Any controller read (e.g. account_exists) must happen BEFORE this:
    // the authorization applies to the next sub-invocation only.
    env.authorize_as_current_contract(vec![env,
        InvokerContractAuthEntry::Contract(SubContractInvocation {
            context: ContractContext {
                contract: hub_asset.asset.clone(),
                fn_name: Symbol::new(env, "transfer"),
                args: (me.clone(), pool.clone(), amount).into_val(env),
            },
            sub_invocations: Vec::new(env),
        }),
    ]);

    // stored_account_id == 0 creates the account bound to spoke_id (>= 1);
    // for an existing account the passed spoke_id must equal the account's
    // stored spoke or the call reverts SpokeMismatch
    client.supply(&me, &stored_account_id, &spoke_id,
                  &vec![env, (hub_asset, amount)])
}
```

Persist the returned account id in your own **persistent** storage and extend
its TTL on use — losing it orphans the position (there is no on-chain
owner→accounts lookup).

## Withdraw, borrow, repay

```rust
// amount 0 = close position; returns actual amounts paid
let paid = client.withdraw(&me, &account_id,
                           &vec![env, (hub_asset.clone(), amount)],
                           &Some(me.clone())); // None also pays the caller

// tokens arrive at `to` (or caller when None); debt recorded on the account
client.borrow(&me, &account_id, &vec![env, (hub_asset.clone(), amount)], &None);

// repay pulls tokens from caller -> pool: same authorize_as_current_contract
// pattern as supply, immediately before the call
client.repay(&me, &account_id, &vec![env, (hub_asset, amount)]);
```

## Reading state cross-contract

- `get_collateral_amount` / `get_borrow_amount` — underlying units,
  index-applied for you.
- `get_market_index(hub_asset)` — supply/borrow indexes accrued to now,
  **reads no oracle** — safe for share pricing (defindex derives its
  price-per-share from `supply_index`).
- `max_withdraw` / `max_borrow` fold in liquidity, caps, and LTV/HF gates;
  `max_supply` is supply-cap headroom only. All return `0` while paused.
- `account_exists(account_id)` — reconcile your stored id; clear it if the
  account is gone.
- Full view surface: `reading-lending-protocol-state` skill.

## TTL management

- `renew_account(caller, account_id)` extends the account's controller
  storage — it is **owner-only** (owner = your contract), so expose an
  entrypoint that forwards to it. Account loads/mutations (supply, withdraw,
  …) renew the same storage, so actively used accounts mostly self-maintain;
  renew explicitly during idle stretches.
- Renewal extends live entries only; an archived entry needs a Soroban
  `RestoreFootprint`, not an extend — do not let long-idle accounts lapse.
- Extend your own storage key holding the account id with its own TTL
  discipline (the reference vault uses ~30-day threshold / ~180-day extend).

## Common mistakes

- **A controller call between authorize and supply/repay** — the token-
  transfer authorization applies to the next sub-invocation only; reorder so
  reads happen first.
- **Authorizing the wrong transfer shape** — it must be exactly
  `transfer(your_contract, pool, amount)` on the asset contract; a mismatch
  reverts the whole call with an auth error.
- **Wrong spoke_id** — `0` reverts `SpokeNotFound` on creation (spoke rules:
  `lending-protocol-fundamentals`); a mismatch with an existing account's
  spoke reverts `SpokeMismatch`.
- **Recomputing underlying from raw scaled positions** — use
  `get_collateral_amount`; raw shares need index multiplication plus a
  27-decimals-to-asset-decimals rescale.
- **Strategy verbs on-chain** — `multiply`/`swap_*` need aggregator swap
  bytes produced by the off-chain quote server; they are not composable from
  pure on-chain code (see `using-lending-sdk`).
