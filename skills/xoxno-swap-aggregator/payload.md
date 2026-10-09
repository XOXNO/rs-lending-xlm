# StrategyPayload and `execute_strategy`

The wire format the quote server puts in `routeXdr`, how the router decodes and executes it, what it authorizes, what it charges, and how to verify a payload before signing. Request/response fields are in [api.md](api.md); embedding a payload in a lending transaction or your own contract is in [composition.md](composition.md); router addresses in [../xoxno-lending/addresses.md](../xoxno-lending/addresses.md); lending-side signatures in [../xoxno-lending-contracts/abi.md](../xoxno-lending-contracts/abi.md); error resolution by contract id in [../xoxno-lending-troubleshooting/SKILL.md](../xoxno-lending-troubleshooting/SKILL.md).

Sources: `rs-lending-xlm/contracts/swap-aggregator/src/{lib,types,program,execute/mod,execute/residual,fees,constants,vault,venues/mod,venues/auth}.rs`, `interfaces/swap-aggregator/src/lib.rs`, `common/src/token.rs`, `arb-algo/stellar-indexer/src/transaction/abi.rs` (commit `54de209a`), `sdk-js/src/sdk/stellar/{swap,scval-encode}.ts` (1.0.228).

## Entry point and contract surface

```rust
// interfaces/swap-aggregator/src/lib.rs
#[contractclient(name = "SwapAggregatorClient")]
pub trait SwapAggregatorInterface {
    fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128;
    // Additional fee, referral, whitelist, recovery, and view methods are omitted here.
}
```

`swap_xdr` contains the XDR serialization of a `StrategyPayload` ScVal.
The router decodes it before execution. Decoding failure raises
`InvalidRouteXdr` (13). `total_in` is gross input in token base units.
Use the quote's `amountIn`. The return value is output after output-side fees.

The snippet shows only `execute_strategy`. The
[interface](../../interfaces/swap-aggregator/src/lib.rs) lists every aggregator method. The
[router implementation](../../contracts/swap-aggregator/src/lib.rs) also exports the
`stellar_access::ownable::Ownable` entrypoints `get_owner`, `transfer_ownership`, and
`accept_ownership`. It does not export `renounce_ownership`.

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

The payload is an `ScVal::Map`. Its symbol keys are `amounts`, `assets`, and
`ops`, in that order. `routeXdr` is the base64 encoding of this map's XDR.
Swap payloads use quote paths, minimum output, and referral ID.
Liquidity payloads use the LP builders. Instructions use `u8` indices into
both registries. Shared addresses appear once.

For `transaction`, the `swap_xdr` argument contains the same bytes as
`routeXdr`. LP conversion uses separate ordered transactions.
Its optional `routeXdr` is an atomic alternative.

The contract derives this map with `#[contracttype]`. The server encoder and
SDK encoder emit the same layout. The aggregator's
`tests/strategy_payload_abi.rs` checks the wire format.

## Packed program (`ops`) — VERSION 1

Byte layout from `program.rs` (contract) and `abi.rs` (encoder), pinned on both sides by tests.

| Offset | Size | Field | Meaning |
|---|---|---|---|
| `[0]` | 1 | `version` | Must equal `1` (`VERSION` / `PROGRAM_VERSION` / `STELLAR_PROGRAM_VERSION`; `/api/v1/config.protocolVersion`) |
| `[1]` | 1 | `token_in` | Index into `assets` |
| `[2]` | 1 | `token_out` | Index into `assets`; must differ from `token_in` (`SameToken = 25`) |
| `[3]` | 1 | `min_out` | **Index into `amounts`** of the total minimum output. Every encoder writes `0` here (`amounts[0] = total_min_out`); read it as an index, never as the amount |
| `[4..8]` | 4 | `referral_id` | `u32` big-endian; `0` = no referral, no fee |
| `[8]` | 1 | `op_count` | `1..=48` (`MAX_OPS`); `0` or above `48` → `EmptyBatch = 1` |
| `[9]` | 1 | `weight_count` | `0..=32` (`MAX_WEIGHTS`); above → `EmptyBatch = 1` |
| `[10 + 5·i]` | 5 each | instruction `i` | See below |
| `[10 + 5·op_count + 3·j]` | 3 each | weight `j` | `u24` big-endian parts-per-million, each in `1..=1_000_000` (`ZeroSplitPpm = 11`, `SplitPpmMismatch = 12`) |

Total length must equal `10 + 5·op_count + 3·weight_count` exactly, at most `10 + 5·48 + 3·32 = 346` bytes (`MAX_PROGRAM_BYTES`), else `InvalidRouteXdr`. Registry caps: `assets` `1..=256` (`MAX_ASSETS`), `amounts` `1..=126` (`MAX_AMOUNTS = MODE_PPM_BASE − MODE_FIXED_BASE`; the `min_out` index must resolve, so an empty registry is `InvalidRouteXdr`).

Instruction record (5 bytes):

| Byte | Field | Swap (`opcode 0..=4`) | Burn (`5`) | Mint (`6`) |
|---|---|---|---|---|
| `[0]` | `opcode` | `0` Soroswap, `1` Aquarius (the encoder also maps Aquarius CLMM pools here), `2` Phoenix, `3` Sushi, `4` CometDex | Aquarius withdraw | Aquarius deposit |
| `[1]` | `mode` | any | must be `All` (`0`) | must be `All` (`0`) |
| `[2]` | `idx_a` | pool → `assets` | pool | pool |
| `[3]` | `idx_b` | `token_in` → `assets` | share token → `assets` | share token → `assets` |
| `[4]` | `idx_c` | `token_out` → `assets` (≠ `idx_b`) | first index of the per-constituent floor run in `amounts` | index of `mint_min_shares` in `amounts` |

The table maps each opcode to its on-chain adapter. It does **not** prove that a
production pool or the quote server is ABI-compatible with that adapter. Treat venue compatibility as unverified until a deployment test pins pool
contracts and quote-server revision. This includes Aquarius CLMM/stable,
Phoenix stable, Sushi concentrated, and Comet weighted pools.

Mode byte → input sizing (`Mode::from_u8` in `program.rs`, `resolve_amount` in `execute/mod.rs`):

| Byte value | Mode | Input amount |
|---|---|---|
| `0` | `All` | Entire vault balance of `token_in` at that instruction |
| `1` | `Prev` | Exactly the previous instruction's output. The predecessor must be a Swap (produces its `token_out`) or a Mint (produces its share token) and that token must equal this `token_in`, else `BrokenTokenChain = 4`; `Prev` on instruction 0 is `BrokenTokenChain` |
| `2..=127` | `Fixed(v − 2)` | `amounts[v − 2]`, an absolute amount |
| `128..=255` | `Ppm(v − 128)` | `vault_balance(token_in) · weights[v − 128] / 1_000_000`, rounded down, measured at execution time |

Unrecognized opcode, an out-of-range registry index, or a mode other than `All` on
Burn/Mint → `InvalidRouteXdr = 13`. `Program::decode` runs these checks before any venue
call. Checks that need pool metadata run during execution. An Aquarius burn reads
`get_tokens` and `share_id`. It uses the token count to validate the complete
burn-floor run.

## `execute_strategy` semantics (`execute/mod.rs::run`)

The router applies these checks and effects in order:

1. Require sender authorization.
2. Reject nonpositive `total_in` with `InvalidAmount` (3).
3. Decode the packed program and validate registry indices.
4. Reject a nonpositive output floor with `SlippageExceeded` (5).
5. Transfer input from the sender and credit the measured balance increase.
   Transfer fees can reduce this credit. Fee reserves do not cover the difference.
6. Apply input-side fees when
   `referral_id != 0 && (!out_whitelisted || in_whitelisted)`.
7. Execute the instructions described below.
8. Apply output-side fees if fees were not taken from input.
9. Reject output below the encoded floor with `SlippageExceeded` (5).
10. Transfer the output to the sender.
11. Credit permitted residual balances to admin revenue.
12. Return output in token base units.

For each swap, resolve a positive input and withdraw it from the vault.
Measure token balances around the venue call. Reject zero output with
`ZeroOutput` (7). Reject a spend mismatch with `InvalidAmount` (3).
Credit the measured output.

For a burn, validate pool metadata and constituent floors.
Burn the whole share balance. Require each receipt to meet its floor,
or fail with `MinAmountsNotMet` (28). Credit each constituent.

For a mint, validate metadata and a positive share floor.
Deposit available constituents. Require the received shares to meet the
floor, or fail with `MinSharesNotMet` (27).

Each residual token has an allowance of `max(credited / 1_000_000, 1_000)`.
`credited` is the token's total deposits during this call.
A larger remainder raises `ExcessiveResidual` (29) and reverts the call.
The router does not refund unused input. Set `total_in` to the route's
required input.

The vault exists only for the invocation. It tracks spendable balances and
total credits while the router holds tokens between instructions.
After the call, the router holds no balance for the sender.

## Authorization model

For an account sender, authorization covers the router call and nested
input-token transfer. Simulation produces this authorization tree.
Check it before signing. Do not transfer or approve tokens before calling
the router. Tokens sent earlier do not enter the invocation's vault.

The router authorizes venue calls itself:

| Venue | Router authorization |
| --- | --- |
| Phoenix, Sushi, Aquarius | Exact nested token transfer from router to pool |
| Comet | Approve input, authorize `transfer_from`, then clear the allowance |
| Soroswap | Transfer input directly to the pool |

These venue authorizations do not require separate sender signatures.

A calling contract is the router's sender. Complete its reads first.
Authorize the exact nested input-token transfer. Immediately call
`execute_strategy`. Measure the calling contract's output balance increase.
The lending controller checks overspend and missing output through
`RouterOverspend` (501) and `NoSwapOutput` (502).
See [composition.md](composition.md).

## Fees and referrals (`fees.rs`, `constants.rs`, `lib.rs`)

| Rule | Value |
|---|---|
| Referral `0` | No static fee, no referral fee, `fee_on_input = false`; nothing is charged on either side |
| Unknown or `active == false` referral | On-chain: same as `0`, no fee (`apply_fees_on_token` returns early; `ReferralNotFound = 22` is raised only by the owner setters and `claim_referral_fees`, never by `execute_strategy`). Quote server: rejected before routing with `400 invalid_request` `unknown referral N` (`lp_fee_for`), so a `routeXdr` from the server always carries `0` or an active id |
| Active referral `id` | `static_fee = balance · static_fee_bps / 10_000` and `referral_fee = balance · cfg.fee_bps / 10_000`, each floored separately, both taken from the same gross balance in one `vault.withdraw`; `combined_bps == 0` or `total <= 0` → nothing charged |
| Cap | `static_fee_bps + referral.fee_bps <= FEE_CAP = 1_000` (10%); above → `FeeTooHigh = 21`. `set_static_fee`, `add_referral`, `set_referral_fee` enforce the same cap per value; the quote server answers 400 when the combination exceeds it |
| Side | Input token unless the output is whitelisted and the input is not (`add_to_whitelist` / `whitelisted_tokens`); the quote server echoes it as `feeOnInput` and `/api/v1/referrals/{id}.feePolicy` |
| Referral id range | `u64` on-chain (`referral_counter` increments from 1), `u32` on the wire; the quote server rejects `referral_id > 4_294_967_295` |
| Claims | `claim_referral_fees(id, tokens)` pays the referral's `owner` and has no auth gate (anyone may trigger it); `claim_admin_fees` and `sweep_balance` are owner-only |

The quote server mirrors this in `quote/fee.rs::compute_fee` (`net_after_fees`, `gross_for_net_fees`) so `amountIn` is already gross and `amountOut`/`amountOutMin` already net.

## Router errors

Use the contract that panicked to select the namespace; controller and router numeric
codes overlap. The canonical enum is
[`errors.rs`](../../contracts/swap-aggregator/src/errors.rs).

| Code | Error | Meaning |
|---|---|---|
| 1 | `EmptyBatch` | Empty/over-cap instruction batch or over-cap weights |
| 3 | `InvalidAmount` | Nonpositive amount, vault overdraft, or measured spend mismatch |
| 4 | `BrokenTokenChain` | Invalid `Prev` dependency; an Aquarius `get_tokens` list that is empty, longer than 256, has duplicates, or omits a hop token; or Sushi `token0`/`token1` does not match the hop pair |
| 5 | `SlippageExceeded` | Nonpositive `min_out` or delivered output below it |
| 7 | `ZeroOutput` | Venue or LP leg produced no usable output |
| 9 | `IntegerOverflow` | Checked conversion or arithmetic overflow |
| 11 / 12 | `ZeroSplitPpm` / `SplitPpmMismatch` | Split weight is zero or above 1,000,000 ppm |
| 13 | `InvalidRouteXdr` | Strategy map or packed program malformed |
| 20 / 21 / 22 | `NotAdmin` / `FeeTooHigh` / `ReferralNotFound` | `admin()` found no owner; a fee above `FEE_CAP`; an unknown referral id in an owner setter or `claim_referral_fees`. Owner-only calls fail with `stellar_access` errors, not `NotAdmin` |
| 25 | `SameToken` | Program input/output or a hop pair is identical |
| 26 | `LpTokenMismatch` | Aquarius `share_id` differs from the declared LP token; see [`assert_share_token`](../../contracts/swap-aggregator/src/venues/aquarius/pool.rs) |
| 27 / 28 | `MinSharesNotMet` / `MinAmountsNotMet` | LP mint or burn floor failed; `28` also when the burn-floor run is shorter than the pool's token count |
| 29 | `ExcessiveResidual` | A leftover exceeds the per-token residual allowance |
| 30 | `InternalInvariant` | Fee-reservation accounting reached an impossible state and failed closed; see [`storage.rs`](../../contracts/swap-aggregator/src/storage.rs) and report it |

## Verify `routeXdr` before signing

Use the published SDK verifier. Do not copy a route decoder into the application.

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
token pair, encoded output floor, and fixed input limits. It does not prove
venue behavior, liquidity, or transaction feasibility. Simulate the complete
transaction before signing.

## Public SDK helpers

Verified public exports in `@xoxno/sdk-js` version `1.0.228`:

| Export | Use |
| --- | --- |
| `mapQuoteResponseToStrategySwap` | Check and use a quote route. Pass caller-owned `expected` values. |
| `verifyStellarRouteBytes` | Decode and check an existing route against caller expectations. |
| `decodeStellarRouteBytes` | Decode a route for inspection. |
| `encodeStrategyPayloadToRouteXdr` | Encode a custom local payload. Prefer the quote route for integrations. |
| `buildStellarExecuteStrategyTx` | Build an unsigned direct router invocation. |
| `prepareStellarBuiltTx` | Simulate and prepare the built transaction. |
| `assertStellarPreparedTxAuth` | Check prepared invocation and authorization against `builtXdr`, caller, and deployment. |

The authorization checker does not establish every requested field of an
externally built envelope. Use the local builder and compare preparation
with that built XDR. The
[standalone example](SKILL.md#example-2-prepare-a-standalone-swap) uses this flow.
