# Aggregator Router

In-repo DEX aggregation router for Stellar Soroban. It executes multi-hop,
multi-venue swap routes built off-chain and passed in as an opaque XDR
payload. The lending controller calls it for strategy swaps (`multiply`,
`swap_collateral`, `swap_debt`, `repay_debt_with_collateral`) and treats its
output as untrusted, verifying its own balance deltas
([ADR 0005](../../architecture/decisions/0005-strategy-aggregator-output-validated-by-balance-delta.md)).

## Entrypoint

```text
execute_strategy(sender: Address, total_in: i128, swap_xdr: Bytes) -> i128
```

`swap_xdr` decodes to a `StrategyPayload` (route hops, venues, splits,
`total_min_out`, optional referral id). The router:

1. Requires auth from `sender` and pulls at most `total_in` of the input token
   into an invocation-local `Vault`.
2. Walks the split paths, dispatching each hop to the corresponding venue
   adapter. The `Vault` tracks **real** token balance deltas on the router
   (pulls from sender, transfers through venues, final output). It never trusts
   a venue's reported `amount_out`.
3. Applies the static protocol fee (and optional referral fee) on the
   appropriate side (input or output, depending on whitelist state).
4. After all paths complete, checks that the measured output >= `total_min_out`.
   If not, reverts with `SlippageExceeded`. The router owns the slippage check.
5. Transfers the final output back to `sender` and returns the amount.

This real-delta tracking (see `vault.rs`) is the core of the untrusted-output
safety model.

## Venues

One adapter per venue under `src/venues/`: Aquarius, Comet (CometDex), Phoenix,
Soroswap, Sushi.

Each venue adapter is also untrusted. The router performs before/after balance
checks on every hop (`dispatch_hop`) to ensure the exact input was spent and a
positive output was delivered to the router's address. Only those measured
deltas are trusted downstream.

## Administration

Owner-gated (OZ `Ownable`): the initial admin is passed to the constructor.
Subsequent ownership transfers use the standard two-step `transfer_ownership` /
`accept_ownership` / `renounce_ownership` flow. Other admin operations include
`set_static_fee`, `add_to_whitelist` / `remove_from_whitelist` (tradable
tokens), `upgrade`, and fee custody (`claim_admin_fees`, `sweep_balance`).
Referral programs are managed with `add_referral`, `set_referral_fee`,
`set_referral_active`, `set_referral_owner`, `claim_referral_fees`.

Read surface: `admin`, `static_fee_bps`, `referral`, `referral_counter`,
`is_whitelisted`, `whitelisted_tokens`, `admin_fee_balance`,
`referral_fee_balance`.

## Trust model

The router is governance-approved but not fully trusted by the lending
protocol:

- The controller (or direct caller) binds `sender` and `total_in`; the router
  requires auth from that sender.
- Route bytes (`StrategyPayload`) are forwarded opaquely.
- The router reverts on input overspend (vault would go negative) or if the
  final measured output delta is non-positive or below `total_min_out`.
- Venues (AMM adapters) are also untrusted; only the router's observed balance
  changes matter.
- An invalid or inactive `referral_id` is silently ignored for the fee path
  (does not brick the swap).

See `ADR 0005` for the controller-side balance-delta verification that wraps
this router.
