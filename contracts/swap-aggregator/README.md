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

`swap_xdr` → `StrategyPayload` with three fields: `amounts` (the amount
registry — `total_min_out`, fixed inputs, burn floors, mint min-shares),
`assets` (the address registry — tokens, pools, LP share tokens), and `ops`
(the packed instruction stream: a 10-byte header carrying the version,
`token_in`, `token_out`, `min_out`, `referral_id` and the instruction and weight
counts, then one 5-byte record per instruction, then the u24 split weights).
Instructions reference the two registries by `u8` index. The byte layout lives
in `src/program.rs`.

1. Auth `sender`, pull `total_in`.
2. Optional fee: on the input token before the hops, or on the output token
   after them.
3. Walk hops; credit only **measured** balance deltas.
4. Revert if out `< total_min_out`.
5. Send `token_out` to `sender`.
6. Accrue leftover balances to the admin bucket; revert with
   `ExcessiveResidual` if one exceeds its residual allowance.

The router credits only measured balance changes, never a venue report or a
payload amount. Venue adapters return nothing: the dispatcher measures the
router's own balance delta for every hop.

## Fees

The static protocol fee applies only together with an active referral. A swap
with `referral_id == 0`, or with an unknown or deactivated referral, pays no
static or referral fee. This matches how the off-chain quote model prices
routes. The fee is taken on the input token, unless only the output token is
fee-whitelisted; then it is taken on the output token. Each fee is
`balance * fee_bps / 10_000`, rounded down. Leftover balance after settlement
accrues to the admin bucket on every swap, with or without a referral.

`sweep_balance` sends `recipient` only the balance above fee backing. It reads
the per-token `ReservedTotal` counter, which fee accrual and claims keep equal
to the sum of that token's fee buckets. Do not upgrade in place from a build
without the counter: the counter reads zero, so `sweep_balance` treats fee
backing as free and fee claims fail with `InternalInvariant`.

## Admin

| Area | Entrypoints |
| --- | --- |
| Fees | `set_static_fee`, `claim_admin_fees`, `sweep_balance` |
| Whitelist | `add_to_whitelist`, `remove_from_whitelist` |
| Referrals | `add_referral`, `set_referral_*`, `claim_referral_fees` (permissionless) |
| Upgrade | `upgrade` |

Fee cap (`FEE_CAP`): 1000 bps for the static fee, for each referral fee, and for
their sum at swap time. A higher value fails with `FeeTooHigh`.

## Layout

```text
src/
  lib.rs          Thin public API (Router + Ownable)
  execute/        Strategy orchestration (mod.rs) + residual accrual
  program.rs      Packed instruction stream: byte layout, decode, validation
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
  venues/         Per-venue adapter cases (incl. Aquarius LP + math)
  admin.rs · chained_hops.rs · execute_strategy.rs · fee_buckets.rs ·
  fees.rs · payload_wire_format.rs · program_decode.rs · splits.rs ·
  sweep.rs · vault.rs
```

## Entrypoints

Signatures are copied from `contracts/swap-aggregator/src/`. The `Env` argument is
dropped by the generated client, so a client call takes one fewer argument than
the signature shows.

| Entrypoint | Signature | Notes | What it does |
| --- | --- | --- | --- |
| `__constructor` | `pub fn __constructor(env: Env, admin: Address)` | — | Set `admin` as Ownable owner. |
| `set_static_fee` | `fn set_static_fee(env: Env, fee_bps: u32)` | owner-only | Set the protocol static fee in bps (`<= FEE_CAP`). |
| `add_to_whitelist` | `fn add_to_whitelist(env: Env, token: Address)` | owner-only | Mark `token` as fee-whitelisted (selects which token pays the fee). |
| `remove_from_whitelist` | `fn remove_from_whitelist(env: Env, token: Address)` | owner-only | Remove `token` from the fee whitelist. |
| `upgrade` | `fn upgrade(env: Env, new_wasm_hash: BytesN<32>)` | owner-only | Upgrade contract WASM. |
| `add_referral` | `fn add_referral(env: Env, owner: Address, fee_bps: u32) -> u64` | owner-only | Create a referral; returns the new id. |
| `set_referral_fee` | `fn set_referral_fee(env: Env, id: u64, fee_bps: u32)` | owner-only | Update a referral's fee bps. |
| `set_referral_active` | `fn set_referral_active(env: Env, id: u64, active: bool)` | owner-only | Activate or deactivate a referral. |
| `set_referral_owner` | `fn set_referral_owner(env: Env, id: u64, new_owner: Address)` | owner-only | Transfer claim rights for a referral. |
| `claim_admin_fees` | `fn claim_admin_fees(env: Env, recipient: Address, tokens: Vec<Address>)` | owner-only | Pay out accrued admin fee balances for `tokens`. |
| `claim_referral_fees` | `fn claim_referral_fees(env: Env, id: u64, tokens: Vec<Address>)` | permissionless | Pay out accrued fees for referral `id` to its configured owner. |
| `sweep_balance` | `fn sweep_balance(env: Env, recipient: Address, tokens: Vec<Address>)` | owner-only | Recover non-fee token balances to `recipient`. |
| `admin` | `fn admin(env: Env) -> Address` | — | Returns the current Ownable owner; panics with `Error::NotAdmin` if unset. |
| `static_fee_bps` | `fn static_fee_bps(env: Env) -> u32` | — | Returns the protocol static fee in basis points. |
| `referral` | `fn referral(env: Env, id: u64) -> Option<ReferralConfig>` | — | Returns the referral config for `id`, or `None` if it does not exist. |
| `referral_counter` | `fn referral_counter(env: Env) -> u64` | — | Returns the highest referral id issued so far. |
| `is_whitelisted` | `fn is_whitelisted(env: Env, token: Address) -> bool` | — | Returns whether `token` is on the fee whitelist. |
| `whitelisted_tokens` | `fn whitelisted_tokens(env: Env) -> Vec<Address>` | — | Returns the full fee-whitelist token list. |
| `admin_fee_balance` | `fn admin_fee_balance(env: Env, token: Address) -> i128` | — | Returns the accrued admin fee balance for `token`. |
| `referral_fee_balance` | `fn referral_fee_balance(env: Env, id: u64, token: Address) -> i128` | — | Returns the accrued referral fee balance for `(id, token)`. |
| `execute_strategy` | `fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128` | `sender` auth | Decode `swap_xdr` as `StrategyPayload`, execute it, and return the delivered `token_out` amount. |
| `get_owner` | `fn get_owner(e: &Env) -> Option<Address>` | — | Returns the current owner, or `None` if it was never set. |
| `transfer_ownership` | `fn transfer_ownership(e: &Env, new_owner: Address, live_until_ledger: u32)` | owner-only | Starts a two-step ownership transfer to `new_owner`, acceptable until ledger `live_until_ledger`. |
| `accept_ownership` | `fn accept_ownership(e: &Env)` | pending owner | Completes a pending ownership transfer. |

Error codes: [`../../docs/reference/errors.md`](../../docs/reference/errors.md).
Events: no custom events; `Ownable` ownership events only. See
[`../../docs/reference/events.md`](../../docs/reference/events.md).
