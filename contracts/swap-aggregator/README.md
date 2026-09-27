# Swap Aggregator

DEX router for Soroban. It runs multi-hop, multi-venue routes built off-chain
and passed as bytes. The controller strategies (`multiply`,
`swap_collateral`, `swap_debt`, `repay_debt_with_collateral`) use it, and
anyone can call it for a plain swap.

| | |
| --- | --- |
| Owner | OZ `Ownable`, two-step |
| Trust | Untrusted by the controller, which checks balance deltas |
| Venues | Aquarius, Comet, Phoenix, Soroswap, Sushi |
| Client | [`interfaces/swap-aggregator`](../../interfaces/swap-aggregator) |

Full signatures are in `contracts/swap-aggregator/src/lib.rs`; the generated
client drops the `Env` argument.

## Swap

```text
execute_strategy(sender, total_in, swap_xdr) -> amount_out
```

`swap_xdr` decodes to a `StrategyPayload`:

- `amounts`: the amount registry (`total_min_out`, fixed inputs, burn floors,
  mint min-shares);
- `assets`: the address registry (tokens, pools, LP share tokens);
- `ops`: the packed instruction stream. A 10-byte header (version, `token_in`,
  `token_out`, `min_out`, `referral_id`, instruction and weight counts), one
  5-byte record per instruction, then u24 split weights. Instructions index
  the registries by `u8`. The byte layout is in `src/program.rs`.

Steps:

1. Authorize `sender` and pull `total_in`.
2. Take the fee, on the input token before the hops or on the output token
   after them.
3. Run the hops. Adapters return nothing; the dispatcher credits only the
   router's **measured** balance change per hop, never a venue report or a
   payload amount.
4. Revert if the output is below `total_min_out`.
5. Send `token_out` to `sender`.
6. Move leftover balances to the admin bucket. Revert with `ExcessiveResidual`
   if one exceeds its allowance.

## Fees

- The static fee applies only with an active referral. `referral_id == 0`, or
  an unknown or inactive referral, pays no static and no referral fee. The
  off-chain quote model prices routes the same way.
- The fee is on the input token, unless only the output token is
  fee-whitelisted.
- Each fee is `balance * fee_bps / 10_000`, rounded down. `FEE_CAP` (1000 bps)
  bounds the static fee, each referral fee and their sum; above it,
  `FeeTooHigh`.
- Leftover balance goes to the admin bucket on every swap, with or without a
  referral.

`sweep_balance` sends only the balance above fee backing. It reads the
per-token `ReservedTotal` counter, which equals the sum of that token's fee
buckets. **Do not upgrade in place from a build without this counter**: it
reads zero, `sweep_balance` treats fee backing as free, and fee claims fail
with `InternalInvariant`.

## Entrypoints

| Entrypoint | Caller | Does |
| --- | --- | --- |
| `__constructor(admin)` | deployer, once | Sets the owner |
| `execute_strategy(sender, total_in, swap_xdr) -> i128` | `sender` | Runs the route; returns the `token_out` delivered |
| `claim_referral_fees(id, tokens)` | anyone | Pays referral `id`'s fees to its owner |
| `set_static_fee(fee_bps)` | owner | Static fee, `<= FEE_CAP` |
| `add_to_whitelist(token)`, `remove_from_whitelist(token)` | owner | Fee whitelist (chooses which token pays) |
| `add_referral(owner, fee_bps) -> u64` | owner | New referral id |
| `set_referral_fee(id, fee_bps)`, `set_referral_active(id, active)`, `set_referral_owner(id, new_owner)` | owner | Edit a referral |
| `claim_admin_fees(recipient, tokens)` | owner | Pays out admin fee balances |
| `sweep_balance(recipient, tokens)` | owner | Recovers balance above fee backing |
| `upgrade(new_wasm_hash)` | owner | Replaces the Wasm |
| `transfer_ownership(new_owner, live_until_ledger)`, `accept_ownership()` | owner, pending owner | Two-step ownership transfer |

| View | Returns |
| --- | --- |
| `admin()` | Owner; panics `NotAdmin` if unset |
| `get_owner()` | Owner, or `None` |
| `static_fee_bps()` | Static fee |
| `referral(id)`, `referral_counter()` | Referral config (or `None`); highest id issued |
| `is_whitelisted(token)`, `whitelisted_tokens()` | Fee whitelist |
| `admin_fee_balance(token)`, `referral_fee_balance(id, token)` | Accrued fees |

## Layout

```text
src/
  lib.rs          public API (Router + Ownable)
  execute/        strategy orchestration + residual accrual
  program.rs      packed instruction stream: layout, decode, validation
  fees.rs         static + referral fee apply/claim
  storage.rs      keys, TTL, fee buckets, whitelist, referrals
  constants.rs    fee cap, PPM, residual policy
  math.rs         checked arithmetic
  types.rs        StrategyPayload, venues, storage keys
  vault.rs        invocation-local balance ledger
  venues/         per-DEX hop adapters (aquarius/ has LP mint/burn)
  errors.rs       error codes
tests/unit/       unit tests, mounted by #[path] from lib.rs
```

## References

- Errors: [`docs/reference/errors.md`](../../docs/reference/errors.md)
- Events: only the `Ownable` ownership events. See
  [`docs/reference/events.md`](../../docs/reference/events.md)
