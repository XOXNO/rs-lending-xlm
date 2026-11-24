# Swap Aggregator

Soroban DEX router running multi-hop, multi-venue routes built off-chain and
passed as bytes. Used by controller strategies (`multiply`, `swap_collateral`,
`swap_debt`, `repay_debt_with_collateral`) and callable by anyone for a plain
swap.

| | |
| --- | --- |
| Owner | OZ `Ownable`, two-step |
| Trust | Untrusted by the controller, which checks balance deltas |
| Venues | Aquarius, Comet, Curve, Phoenix, Soroswap, Sushi |
| Client | [`interfaces/swap-aggregator`](../../interfaces/swap-aggregator); drops `Env` |

## Swap

```text
execute_strategy(sender, total_in, swap_xdr) -> amount_out
```

`swap_xdr` decodes to a `StrategyPayload`:

- `amounts`: amount registry (`total_min_out`, fixed inputs, burn floors,
  mint min-shares);
- `assets`: address registry (tokens, pools, LP share tokens);
- `ops`: packed instructions: a 10-byte header (version, `token_in`,
  `token_out`, `min_out`, `referral_id`, instruction and weight counts), one
  5-byte record per instruction, then u24 split weights. Instructions index the
  registries by `u8`; layout in `src/program.rs`.

Steps:

1. Authorize `sender` and pull `total_in`.
2. Take the fee: input token before the hops, or output token after.
3. Run the hops. Adapters return nothing; the dispatcher credits only the
   router's **measured** balance change per hop, never a venue report or
   payload amount.
4. Revert if output < `total_min_out`.
5. For a SAC `token_out`, call the SAC `trust` (CAP-73) for `sender`. For a
   `G...` sender it opens a missing trustline, which needs the sender's
   signature and 0.5 XLM reserve. For a `C...` sender or an existing trustline
   it does nothing. A failed call is ignored; step 6 then reverts.
6. Send `token_out` to `sender`.
7. Move leftovers to the admin bucket; `ExcessiveResidual` if one exceeds its
   allowance.
8. Publish `SwapEvent`.

Version 1 sends all net output. Version 2 sends exactly `total_min_out`.
For version 2, the router measures the sender's credit and reverts if it
differs from the target. The admin fee bucket receives any net output above
the target.

## Fees

Fee, split and residual formulas:
[formulas](../../docs/reference/formulas.md#swap-router-fees-and-splits).
`referral_id == 0`, or an unknown or inactive referral, pays no static or
referral fee; the off-chain quote model matches. Version 1 charges on input
unless only the output token is fee-whitelisted. Version 2 calculates both fees
on the fixed payout and deducts them from gross output. Leftovers go to the
admin bucket on every swap.

For a 0.50% router fee: `make mainnet setAggregatorFee 5` and
`make mainnet addAggregatorReferral OWNER 45` with the admin signer
(`SIGNER=ledger` for the mainnet owner); pass the returned ID as `referral_id`
in quote requests. DEX pool and network fees are extra.
`make mainnet addAggregatorWhitelist TOKEN` picks the preferred fee token; it
does not restrict tradable tokens.

Claim with `make mainnet claimAggregatorReferralFees ID TOKEN... SIGNER=KEY`
from any funded signer; payment goes to the stored referral owner. A `G...`
owner must exist on-chain with authorized, sufficient trustlines for each
issued token. Query a balance:

```sh
make view-id NETWORK=mainnet CONTRACT_ID=ROUTER \
  FN=referral_fee_balance ARGS='--id ID --token TOKEN'
```

`sweep_balance` sends only the balance above fee backing, read from the
per-token `ReservedTotal` (sum of that token's fee buckets). **Do not upgrade
in place from a build without this counter**: it reads zero, `sweep_balance`
treats fee backing as free, and fee claims fail `InternalInvariant`.

## Entrypoints

Signatures and authority are in the
[endpoint reference](../../docs/reference/endpoints.md#swap-aggregator).
`set_static_fee` takes at most `FEE_CAP`. `admin()` panics `NotAdmin` if the
owner is unset; `get_owner()` returns `None`.

## Storage and TTL

All keys are variants of `DataKey` in [`src/types.rs`](src/types.rs).

| Key | Tier | Holds |
| --- | --- | --- |
| `StaticFeeBps` | instance | Static fee, in BPS |
| `WhitelistedTokens` | instance | Preferred fee tokens |
| `ReferralCounter` | instance | Highest referral id issued |
| OpenZeppelin `Ownable` owner | instance | The admin |
| `Referral(id)` | persistent | `ReferralConfig` |
| `AdminFee(token)`, `ReferralFee(id, token)` | persistent | Fee buckets, in token amount |
| `ReservedTotal(token)` | persistent | Sum of that token's fee buckets |

The constructor and state-changing entrypoints renew the instance TTL
(`renew_instance`). Persistent entries extend with `TTL_THRESHOLD_SHARED` /
`TTL_BUMP_SHARED` on read and on fee-bucket change; an emptied bucket is
removed.

## Events

Each swap publishes `SwapEvent` for volume and fee indexing
([fields](../../docs/reference/events.md#swap-aggregator-events)). For version 2,
`static_fee` includes the fixed-output surplus in `fee_token`. Ownership
changes publish the stock OpenZeppelin `Ownable` events
([inherited events](../../docs/reference/events.md#inherited-events)).

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
  venues/         per-DEX hop adapters (aquarius/ and curve.rs have LP mint/burn)
  errors.rs       error codes
  events.rs       SwapEvent
tests/unit/       unit tests, mounted by #[path] from lib.rs
```

## References

- Errors: [`docs/reference/errors.md`](../../docs/reference/errors.md#swap-aggregator)
- Curve fixtures: [`tests/fixtures/curve/README.md`](tests/fixtures/curve/README.md)
