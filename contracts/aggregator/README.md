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

1. Pulls at most `total_in` of the input token from `sender`.
2. Dispatches each hop to its venue adapter and credits the router's **real
   balance delta**, not the venue's self-reported output.
3. Applies the static protocol fee and any referral fee.
4. Rejects the swap when the aggregate output is below `total_min_out`
   (`SlippageExceeded`) — the router owns the slippage gate.
5. Returns the delivered output amount.

## Venues

One adapter per venue under `src/venues/`: Aquarius, Comet, Phoenix,
Soroswap, Sushi.

## Administration

Admin-gated configuration: `set_admin`, `set_static_fee`,
`add_to_whitelist` / `remove_from_whitelist` (tradable tokens), `upgrade`,
and fee custody (`claim_admin_fees`, `sweep_balance`). Referral programs are
managed with `add_referral`, `set_referral_fee`, `set_referral_active`,
`set_referral_owner`, `claim_referral_fees`.

Read surface: `admin`, `static_fee_bps`, `referral`, `referral_counter`,
`is_whitelisted`, `whitelisted_tokens`, `admin_fee_balance`,
`referral_fee_balance`.

## Trust model

The router is governance-approved but not fully trusted by the lending
protocol: the controller binds `sender` and `total_in` on the wire, forwards
route bytes unchanged, and reverts on any input overspend or non-positive
output delta. An invalid referral id does not brick the swap.
