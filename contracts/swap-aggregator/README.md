# Aggregator Router

DEX swap router for Stellar Soroban. Runs multi-hop, multi-venue routes built
off-chain and passed in as XDR.

The lending controller uses it for strategy swaps (`multiply`,
`swap_collateral`, `swap_debt`, `repay_debt_with_collateral`). The controller
treats the router as untrusted and checks its own balance deltas
([ADR 0005](../../architecture/decisions/0005-strategy-aggregator-output-validated-by-balance-delta.md)).

## Entrypoint

```text
execute_strategy(sender, total_in, swap_xdr) -> i128
```

`swap_xdr` decodes to `StrategyPayload`:

| Field | Meaning |
| --- | --- |
| `token_in` / `token_out` | Endpoint tokens |
| `paths` | Split routes (`split_ppm` + hops) |
| `total_min_out` | Aggregate min out (router-owned slippage) |
| `referral_id` | Optional fee referral (`0` = no fee) |

Flow:

1. Auth `sender`, pull `total_in` of `token_in`.
2. Optionally take fees (input or output side).
3. Walk each path hop; credit only **measured** balance deltas.
4. Revert if output `< total_min_out` (`SlippageExceeded`).
5. Send `token_out` back to `sender`; return the amount.

## Trust model

Nothing from venues or the payload amount fields is trusted as truth:

- Only real token balance changes on the router count (`dispatch_hop`, `vault`).
- Each hop must spend exactly its input and deliver positive output.
- Overspend or output below `total_min_out` reverts.
- Missing / inactive `referral_id` is a no-op (swap still runs).

Controller-side checks wrap this contract — see ADR 0005.

## Venues

Adapters under `src/venues/`:

`Aquarius` · `CometDex` · `Phoenix` · `Soroswap` · `Sushi`

## Admin

Owner: OZ `Ownable` (two-step transfer). Constructor sets the initial owner.

| Area | Entrypoints |
| --- | --- |
| Fees | `set_static_fee`, `claim_admin_fees`, `sweep_balance` |
| Whitelist | `add_to_whitelist`, `remove_from_whitelist` |
| Referrals | `add_referral`, `set_referral_fee`, `set_referral_active`, `set_referral_owner`, `claim_referral_fees` |
| Upgrade | `upgrade` |
| Reads | `admin`, `static_fee_bps`, `referral`, `referral_counter`, `is_whitelisted`, `whitelisted_tokens`, `admin_fee_balance`, `referral_fee_balance` |

Fee side: when a live referral is set, fee is on **input** unless only
`token_out` is whitelisted. Cap is 1000 bps for static and referral fees.

## Layout

```text
src/
  lib.rs       Entrypoints, fees, path execution
  types.rs     StrategyPayload, venues, storage keys
  vault.rs     Invocation-local balance ledger
  venues/      Per-DEX hop adapters
  errors.rs    Error codes
```
