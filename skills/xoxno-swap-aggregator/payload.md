# StrategyPayload and `execute_strategy`

The `routeXdr` wire format, and how the router decodes, executes, authorizes, and charges for it. Related: [api.md](api.md) (request/response fields), [composition.md](composition.md) (lending or own-contract embedding), [addresses](../xoxno-lending/addresses.md), [lending ABI](../xoxno-lending-contracts/abi.md), [troubleshooting](../xoxno-lending-troubleshooting/SKILL.md) (errors by contract id).

Sources: `rs-lending-xlm/contracts/swap-aggregator/src/{lib,types,program,execute/mod,execute/residual,fees,constants,vault,venues/mod,venues/auth}.rs`, `interfaces/swap-aggregator/src/lib.rs`, `common/src/token.rs`, and the swap encoder in the [pinned SDK](../xoxno-lending/SKILL.md#verified-against).

## Entry point and contract surface

```rust
// interfaces/swap-aggregator/src/lib.rs
#[contractclient(name = "SwapAggregatorClient")]
pub trait SwapAggregatorInterface {
    fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128;
    // Additional fee, referral, whitelist, recovery, and view methods are omitted here.
}
```

`swap_xdr` is the XDR of a `StrategyPayload` ScVal, decoded before execution
(failure: `InvalidRouteXdr` (13)). `total_in` is gross input in token base
units: the quote's `amountIn`. The return value is output after output-side
fees.

The [interface](https://github.com/XOXNO/rs-lending-xlm/blob/main/interfaces/swap-aggregator/src/lib.rs)
lists every aggregator method. The
[router](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/swap-aggregator/src/lib.rs)
also exports the `stellar_access::ownable::Ownable` entrypoints `get_owner`,
`transfer_ownership`, and `accept_ownership`, but not `renounce_ownership`.

## `StrategyPayload` ScVal

```rust
// contracts/swap-aggregator/src/types.rs
#[contracttype]
pub struct StrategyPayload {
    pub amounts: Vec<i128>,   // amount registry: min-out, fixed inputs, burn floors, mint min-shares
    pub assets: Vec<Address>, // address registry: tokens, pools, LP share tokens
    pub ops: Bytes,           // packed program: header, instruction records, split weights
}
```

The payload is an `ScVal::Map` with symbol keys `amounts`, `assets`, `ops`, in
that order (derived by `#[contracttype]`; the server and SDK encoders emit the
same layout, and the aggregator's `tests/strategy_payload_abi.rs` checks it).
`routeXdr` is the base64 of this map's XDR. Swap payloads encode quote paths,
minimum output, and referral ID; liquidity payloads come from the LP
builders. Instructions use `u8` indices into both registries, and shared
addresses appear once.

A server `transaction` passes the same bytes as `routeXdr` in `swap_xdr`. LP
conversion uses separate ordered transactions; its optional `routeXdr` is an
atomic alternative.

## Packed program (`ops`) — versions 1 and 2

Layout from `program.rs`, pinned by contract and quote-server encoder tests.

| Offset | Size | Field | Meaning |
|---|---|---|---|
| `[0]` | 1 | `version` | `1` = exact input (`VERSION` / `PROGRAM_VERSION` / `STELLAR_PROGRAM_VERSION`; `/api/v1/config.protocolVersion`); `2` = fixed output (`FIXED_OUT_VERSION`); any other value → `InvalidRouteXdr = 13`. Both versions share this layout |
| `[1]` | 1 | `token_in` | Index into `assets` |
| `[2]` | 1 | `token_out` | Index into `assets`; must differ from `token_in` (`SameToken = 25`) |
| `[3]` | 1 | `min_out` | **Index into `amounts`** of total minimum output; encoders write `0` (`amounts[0] = total_min_out`). An index, never the amount |
| `[4..8]` | 4 | `referral_id` | `u32` big-endian; `0` = no referral, no fee |
| `[8]` | 1 | `op_count` | Number of instructions, `1..=48` for both versions. |
| `[9]` | 1 | `weight_count` | `0..=32` (`MAX_WEIGHTS`); above → `EmptyBatch = 1` |
| `[10 + 5·i]` | 5 each | instruction `i` | Below |
| `[10 + 5·op_count + 3·j]` | 3 each | weight `j` | `u24` big-endian ppm in `1..=1_000_000` (`ZeroSplitPpm = 11`, `SplitPpmMismatch = 12`) |

Length must be exactly `10 + 5·op_count + 3·weight_count`, at most `10 + 5·48 + 3·32 = 346` bytes (`MAX_PROGRAM_BYTES`), else `InvalidRouteXdr`. Registry caps: `assets` `1..=256` (`MAX_ASSETS`), `amounts` `1..=126` (`MAX_AMOUNTS = MODE_PPM_BASE − MODE_FIXED_BASE`; `min_out` must resolve, so an empty registry is `InvalidRouteXdr`).

Instruction record (5 bytes):

| Byte | Field | Swap (`opcode 0..=4` or `7`) | Burn (`5`) | Mint (`6`) |
|---|---|---|---|---|
| `[0]` | `opcode` | `0` Soroswap, `1` Aquarius (encoder also maps Aquarius CLMM here), `2` Phoenix, `3` Sushi, `4` CometDex, `7` Curve | Aquarius withdraw | Aquarius deposit |
| `[1]` | `mode` | any | must be `All` (`0`) | must be `All` (`0`) |
| `[2]` | `idx_a` | pool → `assets` | pool | pool |
| `[3]` | `idx_b` | `token_in` → `assets` | share token → `assets` | share token → `assets` |
| `[4]` | `idx_c` | `token_out` → `assets` (≠ `idx_b`) | start of per-constituent floor run in `amounts` | `mint_min_shares` index in `amounts` |

The table maps opcodes to on-chain adapters; it does **not** prove a
production pool or the quote server is ABI-compatible with them. Treat venue
compatibility (including Aquarius CLMM/stable, Phoenix stable, Sushi
concentrated, and Comet weighted pools) as unverified until a deployment test
pins pool contracts and quote-server revision.

Curve requires a router build with opcode `7` and a matching quote-server
encoder. Its adapter reads `pool_info().coins` and picks the pool call from
the hop's tokens:

| Hop | Pool call | Nested auth |
|---|---|---|
| `token_in` and `token_out` are both coins | `exchange(router, i, j, amount_in, 0, router)` | Exact transfer of `amount_in` to the pool |
| `token_out` is the pool (LP mint) | `add_liquidity(router, amounts, 0, router)`, with `amount_in` at coin `i` and `0` elsewhere | Exact transfer of `amount_in` to the pool |
| `token_in` is the pool (one-coin LP burn) | `remove_liquidity_one_coin(router, amount_in, j, 0, router)` | None: the pool burns its own LP |

The plain router address is accepted as a `MuxedAddress` receiver, and
metadata reads precede transfer authorization. The coin list must hold 2 to
256 distinct addresses, none the pool, else `BrokenTokenChain`. Every Curve
call passes `0` as its own minimum, so the payload `min_out` is the only
floor; measured balance checks still apply.

Reverse v2 accepts only swaps in `Fixed` mode. The indexed `amounts` value gives the hop’s positive relative weight among producers of the same output token. Pools must be unique and producers must precede consumers. The router sizes hops backwards from the gross output target before executing them forwards.

Version 1 mode byte → input sizing (`Mode::from_u8` in `program.rs`, `resolve_amount` in `execute/mod.rs`):

| Byte value | Mode | Input amount |
|---|---|---|
| `0` | `All` | Whole vault balance of `token_in` |
| `1` | `Prev` | Previous instruction's exact output. Predecessor must be a Swap (its `token_out`) or Mint (its share token) equal to this `token_in`, else `BrokenTokenChain = 4` (also on instruction 0) |
| `2..=127` | `Fixed(v − 2)` | Absolute `amounts[v − 2]` |
| `128..=255` | `Ppm(v − 128)` | `vault_balance(token_in) · weights[v − 128] / 1_000_000`, floored, at execution time |

An unrecognized opcode, out-of-range registry index, or non-`All` mode on
Burn/Mint → `InvalidRouteXdr = 13`, checked by `Program::decode` before any
venue call. The `Prev` check runs first. A Burn or Mint with mode `Prev`
therefore fails with `BrokenTokenChain = 4` unless its predecessor produces
its share token. Checks needing pool metadata run during execution: an Aquarius
burn reads `get_tokens` and `share_id` and uses the token count to validate
the complete burn-floor run.

## `execute_strategy` semantics (`execute/mod.rs::run`)

In order:

1. Require sender authorization.
2. Reject nonpositive `total_in` with `InvalidAmount` (3).
3. Decode the packed program and validate registry indices.
4. Reject a nonpositive output floor with `SlippageExceeded` (5).
5. Transfer input from the sender and credit the measured balance increase
   (token transfer fees reduce it; fee reserves do not cover the difference).
6. Version 1 only: apply input-side fees when
   `referral_id != 0 && (!out_whitelisted || in_whitelisted)`.
7. Version 1 executes the encoded input allocations. Version 2 requires full receipt of the input cap, calculates the fees once on the payout, prices the graph for payout plus fees, executes the hops, and refunds unused input to the sender.
8. Version 1: apply output-side fees if fees were not taken from input.
   Version 2 reserves the previously calculated fees. Gross output must cover the payout and both fees. The admin accrues the protocol fee plus any final output surplus.
9. Reject output below the encoded floor with `SlippageExceeded` (5).
10. Open the sender's missing output trustline (Stellar Asset Contract only),
    then transfer the output to the sender. Version 1 sends all net output.
    Version 2 sends exactly `min_out` and measures the receipt; any other
    received amount fails with `SlippageExceeded` (5).
11. Credit permitted residual balances to admin revenue.
12. Return output in token base units.

| Instruction | Steps | Failure |
|---|---|---|
| Swap | Withdraw positive input from vault, measure balances around venue call, credit measured output | `ZeroOutput` (7) on zero output; `InvalidAmount` (3) on spend mismatch |
| Burn | Validate metadata and constituent floors, burn whole share balance, credit constituents | `MinAmountsNotMet` (28) when a receipt misses its floor |
| Mint | Validate metadata and positive share floor, deposit available constituents | `MinSharesNotMet` (27) when shares miss the floor |

Each residual token may keep `max(credited / 1_000_000, 1_000)`, where
`credited` is its total deposits during the call; a larger remainder raises
`ExcessiveResidual` (29). For v1, set `total_in` to the input to spend. For v2, the source refund is paid before this residual limit applies.

The vault lives only for the invocation, tracking spendable balances and total
credits between instructions; afterwards the router holds no balance for the
sender.

## Authorization model

For an account sender, authorization covers the router call and the nested
input-token transfer. When the sender has no trustline for a Stellar Asset
Contract output token, the tree also holds `token_out.trust(sender)`.
Simulation produces this tree, so check it before signing. Allow only these
exact children, and refuse every other child. Do not transfer or approve
tokens before calling the router: tokens sent earlier do not enter the
invocation's vault.

The router authorizes venue calls itself, with no extra sender signatures:

| Venue | Router authorization |
| --- | --- |
| Phoenix, Sushi, Aquarius, Curve (exchange and LP mint) | Exact nested router→pool transfer |
| Curve one-coin LP burn | None; pool burns router's LP |
| Comet | Approve input, authorize `transfer_from`, then clear the allowance |
| Soroswap | Transfer input directly to the pool |

A calling contract is the router's sender. Complete its reads first,
authorize the exact nested input-token transfer, immediately call
`execute_strategy`, then measure its output balance increase. The lending
controller checks overspend and missing output with `RouterOverspend` (501)
and `NoSwapOutput` (502); see [composition.md](composition.md).

## Fees and referrals (`fees.rs`, `constants.rs`, `lib.rs`)

| Rule | Value |
|---|---|
| Referral `0` | No static or referral fee, `fee_on_input = false`. Version 1 charges nothing; version 2 still books all gross output above the payout as admin revenue |
| Unknown or `active == false` referral | On-chain: no fee, like `0` (`apply_fees_on_token` returns early; `ReferralNotFound = 22` only from owner setters and `claim_referral_fees`, never `execute_strategy`). Quote server: `400 invalid_request` `unknown referral N` (`lp_fee_for`), so server `routeXdr` carries `0` or an active id |
| Active referral `id` | Both versions use `static_fee_bps = cfg.protocol_fee_bps.unwrap_or(global_static_fee_bps)`. Version 1: `static_fee = balance · static_fee_bps / 10_000`, `referral_fee = balance · cfg.fee_bps / 10_000`, each floored, both from the same gross balance in one `vault.withdraw`; `combined_bps == 0` or `total <= 0` → nothing charged. Version 2: the same rates on the payout, `min_out` |
| Cap | `static_fee_bps + referral.fee_bps <= FEE_CAP = 1_000` (10%), else `FeeTooHigh = 21`; `add_referral` and `set_referral_fee` enforce the combined rate using the override or current global fee; quote server answers 400 above it |
| Side | Version 1: input unless output is whitelisted and input is not (`add_to_whitelist` / `whitelisted_tokens`); echoed as `feeOnInput` and `/api/v1/referrals/{id}.feePolicy`. Version 2: always the output token; the whitelist is not read |
| Referral id range | `u64` on-chain (`referral_counter` from 1), `u32` on the wire; server rejects `referral_id > 4_294_967_295` |
| Claims | `claim_referral_fees(id, tokens)` pays the referral `owner`, callable by anyone; `claim_admin_fees`, `sweep_balance` owner-only |

The quote server applies the same formula: `amountIn` is gross, `amountOut`/`amountOutMin` net.

## Router errors

Controller and router codes overlap; the panicking contract selects the
namespace. Enum:
[`errors.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/swap-aggregator/src/errors.rs).

| Code | Error | Meaning |
|---|---|---|
| 1 | `EmptyBatch` | Empty/over-cap instructions or over-cap weights |
| 3 | `InvalidAmount` | Nonpositive amount, vault overdraft, or spend mismatch |
| 4 | `BrokenTokenChain` | Invalid `Prev`; malformed Aquarius `get_tokens` or Curve `pool_info().coins`, duplicate or missing hop tokens; Sushi `token0`/`token1` not matching the hop pair |
| 5 | `SlippageExceeded` | Nonpositive `min_out` or delivered output below it |
| 7 | `ZeroOutput` | Venue or LP leg produced nothing usable |
| 9 | `IntegerOverflow` | Checked conversion or arithmetic overflow |
| 11 / 12 | `ZeroSplitPpm` / `SplitPpmMismatch` | Split weight zero or above 1,000,000 ppm |
| 13 | `InvalidRouteXdr` | Malformed map or program |
| 20 / 21 / 22 | `NotAdmin` / `FeeTooHigh` / `ReferralNotFound` | `admin()` found no owner; fee above `FEE_CAP`; unknown referral in an owner setter or `claim_referral_fees`. Owner-only calls fail with `stellar_access` errors, not `NotAdmin` |
| 25 | `SameToken` | Identical program input/output or hop pair |
| 26 | `LpTokenMismatch` | Aquarius `share_id` differs from declared LP token; see [`assert_share_token`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/swap-aggregator/src/venues/aquarius/pool.rs) |
| 27 / 28 | `MinSharesNotMet` / `MinAmountsNotMet` | LP mint or burn floor failed; `28` also when the burn-floor run is shorter than the pool's token count |
| 29 | `ExcessiveResidual` | Leftover above per-token allowance |
| 30 | `InternalInvariant` | Fee-reservation accounting hit an impossible state, failed closed; see [`storage.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/swap-aggregator/src/storage.rs) and report it |

## Verify `routeXdr` before signing

Use the published SDK verifier; do not copy a route decoder into the application.

```ts
import { verifyStellarRouteBytes } from '@xoxno/sdk-js/stellar-lending'

export function checkRoute(
  routeXdr: string, tokenIn: string, tokenOut: string,
  amountIn: string, acceptedMinOut: string,
) {
  return verifyStellarRouteBytes(routeXdr, {
    tokenIn, tokenOut, amountIn, minOut: acceptedMinOut,
  })
}
```

Take expectations from the caller's request. The verifier checks structure,
token pair, encoded output floor, and fixed input limits, not venue behavior,
liquidity, or feasibility; simulate the complete transaction before signing.

## Public SDK helpers

Verified public exports in the [pinned SDK](../xoxno-lending/SKILL.md#verified-against):

| Export | Use |
| --- | --- |
| `mapQuoteResponseToStrategySwap` | Check and use a quote route against caller-owned `expected` |
| `verifyStellarRouteBytes` | Decode and check a route against caller expectations |
| `decodeStellarRouteBytes` | Decode for inspection |
| `encodeStrategyPayloadToRouteXdr` | Encode a custom payload; integrations prefer the quote route |
| `buildStellarExecuteStrategyTx` | Build unsigned direct router call |
| `prepareStellarBuiltTx` | Simulate and prepare |
| `assertStellarPreparedTxAuth` | Check prepared invocation and auth against `builtXdr`, caller, deployment |

The authorization checker does not establish every field of an externally
built envelope: build locally and compare preparation with that built XDR, as
the [standalone example](SKILL.md#example-2-prepare-a-standalone-swap) does.
