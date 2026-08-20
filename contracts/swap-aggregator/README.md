# Swap Aggregator

DEX swap router for Soroban. Executes multi-hop, multi-venue routes built
off-chain and passed as XDR. Used by controller strategies
(`multiply`, `swap_collateral`, `swap_debt`, `repay_debt_with_collateral`).

| | |
| --- | --- |
| Owner | OZ `Ownable` (two-step) |
| Trust | Untrusted by controller — balance-delta checked |
| Venues | Aquarius · Comet · Phoenix · Soroswap · Sushi |

## Entrypoint

```text
execute_strategy(sender, total_in, swap_xdr) -> i128
```

`swap_xdr` → `StrategyPayload`: `token_in` / `token_out`, split `paths`,
`total_min_out`, optional `referral_id`.

1. Auth `sender`, pull `total_in`.
2. Optional fees (input or output side).
3. Walk hops; credit only **measured** balance deltas.
4. Revert if out `< total_min_out`.
5. Send `token_out` to `sender`.

Nothing from venues or payload amount fields is truth — only real balance
changes count.

## Admin

| Area | Entrypoints |
| --- | --- |
| Fees | `set_static_fee`, `claim_admin_fees`, `sweep_balance` |
| Whitelist | `add_to_whitelist`, `remove_from_whitelist` |
| Referrals | `add_referral`, `set_referral_*`, `claim_referral_fees` |
| Upgrade | `upgrade` |

Fee cap: 1000 bps (static and referral).

## Layout

```text
src/
  lib.rs          Thin public API (Router + Ownable)
  execute/        Strategy orchestration, paths, validate, residual
  fees.rs         Static + referral fee apply/claim
  storage.rs      Keys, TTL, fee buckets, whitelist, referrals
  constants.rs    Fee cap, PPM, residual policy
  math.rs         Checked arithmetic
  types.rs        StrategyPayload, venues, storage keys
  vault.rs        Invocation-local balance ledger
  venues/         Per-DEX hop adapters + auth helpers
    aquarius/     Hop swap + LP mint/burn
  errors.rs       Error codes
tests/unit/       Unit tests (wired via `#[path]` from lib.rs)
  support/        Shared helpers + mock pools/tokens
  venues/         Per-venue adapter cases
  execute_strategy.rs · splits.rs · admin.rs · fees.rs · sweep.rs · vault.rs
```

## Entrypoints

Signatures are copied from `contracts/swap-aggregator/src/`. The `Env` argument is
dropped by the generated client, so a client call takes one fewer argument than
the signature shows.

| Entrypoint | Signature | Notes | What it does |
| --- | --- | --- | --- |
| `__constructor` | `pub fn __constructor(env: Env, admin: Address)` | — | Set `admin` as Ownable owner and zero the fee and referral state. |
| `set_static_fee` | `fn set_static_fee(env: Env, fee_bps: u32)` | owner-only | Set the protocol static fee in bps (`<= FEE_CAP`). |
| `add_to_whitelist` | `fn add_to_whitelist(env: Env, token: Address)` | owner-only | Mark `token` as fee-whitelisted (affects input-side fee selection). |
| `remove_from_whitelist` | `fn remove_from_whitelist(env: Env, token: Address)` | owner-only | Remove `token` from the fee whitelist. |
| `upgrade` | `fn upgrade(env: Env, new_wasm_hash: BytesN<32>)` | owner-only | Upgrade contract WASM. |
| `add_referral` | `fn add_referral(env: Env, owner: Address, fee_bps: u32) -> u64` | owner-only | Create a referral; returns the new id. |
| `set_referral_fee` | `fn set_referral_fee(env: Env, id: u64, fee_bps: u32)` | owner-only | Update a referral's fee bps. |
| `set_referral_active` | `fn set_referral_active(env: Env, id: u64, active: bool)` | owner-only | Activate or deactivate a referral. |
| `set_referral_owner` | `fn set_referral_owner(env: Env, id: u64, new_owner: Address)` | owner-only | Transfer claim rights for a referral. |
| `claim_admin_fees` | `fn claim_admin_fees(env: Env, recipient: Address, tokens: Vec<Address>)` | owner-only | Pay out accrued admin fee balances for `tokens`. |
| `claim_referral_fees` | `fn claim_referral_fees(env: Env, id: u64, tokens: Vec<Address>)` | — | Pay out accrued fees for referral `id` to its configured owner. |
| `sweep_balance` | `fn sweep_balance(env: Env, recipient: Address, tokens: Vec<Address>)` | owner-only | Recover non-fee token balances to `recipient`. |
| `admin` | `fn admin(env: Env) -> Address` | — | Returns the current Ownable owner; panics with `Error::NotAdmin` if unset. |
| `static_fee_bps` | `fn static_fee_bps(env: Env) -> u32` | — | Returns the protocol static fee in basis points. |
| `referral` | `fn referral(env: Env, id: u64) -> Option<ReferralConfig>` | — | Returns the referral config for `id`, or `None` if it does not exist. |
| `referral_counter` | `fn referral_counter(env: Env) -> u64` | — | Returns the highest referral id issued so far. |
| `is_whitelisted` | `fn is_whitelisted(env: Env, token: Address) -> bool` | — | Returns whether `token` is on the fee whitelist. |
| `whitelisted_tokens` | `fn whitelisted_tokens(env: Env) -> Vec<Address>` | — | Returns the full fee-whitelist token list. |
| `admin_fee_balance` | `fn admin_fee_balance(env: Env, token: Address) -> i128` | — | Returns the accrued admin fee balance for `token`. |
| `referral_fee_balance` | `fn referral_fee_balance(env: Env, id: u64, token: Address) -> i128` | — | Returns the accrued referral fee balance for `(id, token)`. |
| `execute_strategy` | `fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128` | — | Decode `swap_xdr` as `StrategyPayload` and execute it. |
| `get_owner` | `fn get_owner(e: &Env) -> Option<Address>` | — | Returns the current owner, or `None` if ownership has been renounced or was never set. |
| `transfer_ownership` | `fn transfer_ownership(e: &Env, new_owner: Address, live_until_ledger: u32)` | — | Starts a two-step ownership transfer to `new_owner`, acceptable until ledger `live_until_ledger`. |
| `accept_ownership` | `fn accept_ownership(e: &Env)` | — | Completes a pending ownership transfer. |
| `renounce_ownership` | `fn renounce_ownership(e: &Env)` | — | Clears the current owner. |

Error codes: [`../../docs/reference/errors.md`](../../docs/reference/errors.md).
Events: [`../../docs/reference/events.md`](../../docs/reference/events.md).
