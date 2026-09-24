# StrategyPayload and `execute_strategy`

The wire format the quote server puts in `routeXdr`, how the router decodes and executes it, what it authorizes, what it charges, and how to verify a payload before signing. Request/response fields are in [api.md](api.md); embedding a payload in a lending transaction or your own contract is in [composition.md](composition.md); router addresses in [../xoxno-lending/addresses.md](../xoxno-lending/addresses.md); lending-side signatures in [../xoxno-lending-contracts/abi.md](../xoxno-lending-contracts/abi.md); error resolution by contract id in [../xoxno-lending-troubleshooting/SKILL.md](../xoxno-lending-troubleshooting/SKILL.md).

Sources: `rs-lending-xlm/contracts/swap-aggregator/src/{lib,types,program,execute/mod,execute/residual,fees,constants,vault,venues/mod,venues/auth}.rs`, `interfaces/swap-aggregator/src/lib.rs`, `common/src/token.rs`, `arb-algo/stellar-indexer/src/transaction/abi.rs` (commit `f2d5fe9`), `sdk-js/src/sdk/stellar/{swap,scval-encode}.ts` (1.0.214).

## Entry point and contract surface

```rust
// interfaces/swap-aggregator/src/lib.rs
#[contractclient(name = "SwapAggregatorClient")]
pub trait SwapAggregatorInterface {
    fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128;
    // Additional fee, referral, whitelist, recovery, and view methods are omitted here.
}
```

`swap_xdr` is the XDR serialization of the `StrategyPayload` ScVal: `lib.rs` runs `StrategyPayload::from_xdr(&env, &swap_xdr)` and panics with `Error::InvalidRouteXdr = 13` on failure, then `execute::run`. `total_in` is the gross input in atomic units — the quote's `amountIn`. The return value is the delivered output in atomic units of `token_out`, net of any output-side fee.

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

On the wire this is `ScVal::Map` with `Symbol` keys in lexicographic order `amounts`, `assets`, `ops`. `#[contracttype]` derives this layout, `abi.rs::StrategyPayload::to_scval` and `scval-encode.ts::scStruct` (sorted keys) emit it, and `arb-algo/stellar-indexer/tests/strategy_payload_abi.rs` pins it. `routeXdr` = base64 of that map's XDR (`abi.rs::to_xdr_bytes`, `Limits::none()`). A swap quote builds it with `build_strategy_payload_xdr_from_quote(quote, referral_id)` from the quote's `paths` (or its single `hops` path), `amountOutMin`, and the request's `referral_id`; a liquidity quote builds it with `builder/lp.rs::build_lp_strategy_payload`, or `builder/lp.rs::build_convert_strategy_payload` for an LP-to-LP conversion. The standalone envelope's `swap_xdr` argument carries the same bytes as `routeXdr`. Instructions address both registries by `u8` index, so an address shared by several hops is carried once.

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
production pool or the quote server is ABI-compatible with that adapter. Treat production
compatibility for Aquarius CLMM and stable pools, Phoenix stable pools, Sushi concentrated
pools, and Comet weighted pools as unverified until a deployment test pins those pool
contracts and the quote-server revision.

Mode byte → input sizing (`Mode::from_u8` in `program.rs`, `resolve_amount` in `execute/mod.rs`):

| Byte value | Mode | Input amount |
|---|---|---|
| `0` | `All` | Entire vault balance of `token_in` at that instruction |
| `1` | `Prev` | Exactly the previous instruction's output. The predecessor must be a Swap (produces its `token_out`) or a Mint (produces its share token) and that token must equal this `token_in`, else `BrokenTokenChain = 4`; `Prev` on instruction 0 is `BrokenTokenChain` |
| `2..=127` | `Fixed(v − 2)` | `amounts[v − 2]`, an absolute amount |
| `128..=255` | `Ppm(v − 128)` | `vault_balance(token_in) · weights[v − 128] / 1_000_000`, rounded down, measured at execution time |

Unrecognized opcode, an out-of-range registry index, or a mode other than `All` on
Burn/Mint → `InvalidRouteXdr = 13`. `Program::decode` runs these checks before any venue
call. Checks that need pool metadata run during execution: an Aquarius Burn first calls
`get_tokens` and `share_id`, then uses the pool's token count to validate the burn-floor
run.

## `execute_strategy` semantics (`execute/mod.rs::run`)

Order of operations, with the error raised on failure:

1. `sender.require_auth()`.
2. `total_in <= 0` → `InvalidAmount = 3`.
3. `Program::decode(ops, assets.len(), amounts.len())` — packed-program checks above;
   venue-dependent LP checks happen during execution.
4. `amounts[min_out] <= 0` → `SlippageExceeded = 5`.
5. **Measured input credit:** `transfer_amount_measured(token_in, sender → router, total_in)` (`common/src/token.rs`) transfers under the sender's auth and credits the vault with `balance_after − balance_before`, not the declared `total_in`. A fee-on-transfer input shrinks the credited amount instead of drawing on the fee reserve.
6. **Fee side:** `fee_on_input = referral_id != 0 && (!out_whitelisted || in_whitelisted)`; when true, `fees::apply_fees_on_token(token_in)` debits the vault before any hop.
7. Instruction loop. Swap: resolve the input by mode (`InvalidAmount` if `<= 0`), `vault.withdraw`, `venues::dispatch_hop` — measures the router's own `token_in`/`token_out` balances around the venue call; received `<= 0` → `ZeroOutput = 7`; spent `!= amount_in` → `InvalidAmount` — then `vault.deposit(token_out, received)`. Burn: reads Aquarius pool tokens and share id, validates the full constituent-floor span, withdraws the vault's whole share-token balance, checks each receipt against its floor (`MinAmountsNotMet = 28`), and deposits every constituent. Mint: reads and validates Aquarius metadata, deposits the vault's constituent balances, and requires `mint_min_shares > 0` and `shares >= mint_min_shares` (`MinSharesNotMet = 27`).
8. When the fee was not on input: `apply_fees_on_token(token_out)`.
9. **Min-out check:** `total_out = vault.balance_of(token_out)`; `total_out < amounts[min_out]` → `SlippageExceeded = 5`.
10. `vault.withdraw(token_out, total_out)` and `token_out.transfer(router → sender, total_out)`.
11. **Residual rule** (`execute/residual.rs`): every remaining vault balance is accrued to the admin fee bucket, but only up to `residual_allowance(credited) = max(credited / 1_000_000, 1_000)` per token (`constants.rs`), where `credited` is that token's lifetime deposits in this call. A larger leftover → `ExcessiveResidual = 29` and the whole call reverts. Unspent input is never refunded to the sender; size `total_in` to what the route consumes.
12. Return `total_out` (`i128`).

The vault (`vault.rs`) is an invocation-local ledger, not on-chain storage: the router custodies the tokens between hops and the vault tracks spendable balances plus lifetime credits. Nothing stays in router custody for the sender after the call.

## Authorization model

The only signature is the sender's. `execute_strategy` calls `sender.require_auth()`, and the sender's auth entry must cover the nested `token_in.transfer(sender, router, total_in)`; simulation produces that tree (`transaction.simulated = true` envelopes already carry it — `attach_simulated_transaction` copies the simulator's `auth` into the op — and `simulateTransaction` produces it for a locally built one). Do **not** `transfer` or `approve` tokens to the router beforehand: the router pulls the input itself, and tokens sent ahead of time are not credited.

Every venue call is self-authorized by the router with invoker-contract auth (`venues/auth.rs::authorize_as_current` → `env.authorize_as_current_contract`): Phoenix and Sushi (`HopContext::authorize_pool_pull`) and Aquarius (`aquarius/pool.rs::invoke_pool_swap`) register `token_in.transfer(router, pool, amount_in)` before the pool pulls; Comet registers `token_in.approve(router, pool, amount_in, expiry)`, then `swap_exact_amount_in` with a nested entry for the pool's `transfer_from`, then clears the allowance; Soroswap transfers from the router to the pool directly. None of these appear in the sender's auth tree.

A contract calling the router (the lending controller in `contracts/controller/src/strategies/swap.rs`, or your own) is the `sender`: it runs `authorize_transfer_as_current(token_in, self, router, amount_in)` (`common/src/token.rs`) immediately before `execute_strategy(self, amount_in, swap)` and measures its own balance deltas afterward (`RouterOverspend = 501`, `NoSwapOutput = 502` in `common/src/errors.rs`). Details in [composition.md](composition.md).

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

`verifyRouteBytes` works for standalone and composed quotes; it does not require a
`transaction`. `verifyStandaloneEnvelope` adds envelope checks. `verifyRoutePayload`
combines both for the standalone signing path. The envelope's actual
`InvokeContractArgs.contract_address` is authoritative; `transaction.routerContract`
is untrusted response metadata and is intentionally unused. Copy this snippet into an
application module such as `verify-route-payload.ts`; the other examples import that
module.

```ts
import { Buffer } from 'node:buffer'
import { scValToNative, StrKey, xdr } from '@stellar/stellar-sdk' // ^16

interface RouteQuote {
  from: string
  to: string
  amountIn: string
  amountOutMin: string
  routeXdr: string
}

interface StandaloneQuote extends RouteQuote {
  transaction: { envelopeXdr: string; simulated: boolean }
}

interface ExpectedRoute {
  tokenIn: string
  tokenOut: string
}

interface ExpectedStandalone extends ExpectedRoute {
  router: string
  signer: string
  totalIn: string
}

function readEd25519Source(source: xdr.MuxedAccount, label: string): string {
  if (source.switch().value !== xdr.CryptoKeyType.keyTypeEd25519().value) {
    throw new Error(`${label} must be an unmuxed Ed25519 account`)
  }
  return StrKey.encodeEd25519PublicKey(source.ed25519())
}

export function verifyRouteBytes(quote: RouteQuote, expected: ExpectedRoute): void {
  if (quote.from !== expected.tokenIn || quote.to !== expected.tokenOut) {
    throw new Error('quote pair does not match requested pair')
  }
  const payload = scValToNative(xdr.ScVal.fromXDR(quote.routeXdr, 'base64')) as {
    amounts: bigint[]
    assets: string[]
    ops: Buffer
  }
  const ops = payload.ops
  if (ops.length < 10) throw new Error('program header is truncated')
  if (ops[0] !== 1) throw new Error(`unsupported program version ${ops[0]}`)
  const tokenIn = payload.assets[ops[1]]
  const tokenOut = payload.assets[ops[2]]
  if (tokenIn !== expected.tokenIn || tokenOut !== expected.tokenOut) {
    throw new Error('encoded token pair does not match requested pair')
  }
  // ops[3] indexes amounts; it is not the amount.
  const minOut = payload.amounts[ops[3]]
  if (minOut === undefined || minOut !== BigInt(quote.amountOutMin)) {
    throw new Error('encoded minimum output does not match quote.amountOutMin')
  }
  const opCount = ops[8]
  const weightCount = ops[9]
  if (ops.length !== 10 + 5 * opCount + 3 * weightCount) throw new Error('program length mismatch')
}

export function verifyStandaloneEnvelope(
  quote: StandaloneQuote,
  expected: ExpectedStandalone,
): void {
  // The standalone envelope must invoke execute_strategy(sender, total_in, swap_xdr)
  // with swap_xdr == routeXdr bytes and total_in == the caller-requested amount.
  const envelope = xdr.TransactionEnvelope.fromXDR(quote.transaction.envelopeXdr, 'base64')
  const tx = envelope.v1().tx()
  if (envelope.v1().signatures().length !== 0 || tx.operations().length !== 1) {
    throw new Error('expected one unsigned invocation')
  }
  const source = readEd25519Source(tx.sourceAccount(), 'transaction source')
  if (source !== expected.signer) throw new Error('transaction source does not match expected signer')

  const operation = tx.operations()[0]
  const operationSource = operation.sourceAccount()
  if (operationSource && readEd25519Source(operationSource, 'operation source') !== expected.signer) {
    throw new Error('operation source does not match expected signer')
  }
  const body = operation.body()
  if (body.switch().value !== xdr.OperationType.invokeHostFunction().value) {
    throw new Error('wrong operation type')
  }
  const hostFunction = body.invokeHostFunctionOp().hostFunction()
  if (
    hostFunction.switch().value !==
    xdr.HostFunctionType.hostFunctionTypeInvokeContract().value
  ) {
    throw new Error('wrong host function type')
  }
  const call = hostFunction.invokeContract()
  const invokedRouter = scValToNative(xdr.ScVal.scvAddress(call.contractAddress())) as string
  if (invokedRouter !== expected.router) throw new Error('invoked router does not match expected router')
  if (call.functionName().toString() !== 'execute_strategy') throw new Error('wrong function')
  if (call.args().length !== 3) throw new Error('execute_strategy must have three arguments')
  const sender = scValToNative(call.args()[0]) as string
  if (sender !== expected.signer || sender !== source) {
    throw new Error('sender argument does not match signer/source')
  }
  if (BigInt(quote.amountIn) !== BigInt(expected.totalIn)) {
    throw new Error('quote amountIn does not match caller-requested totalIn')
  }
  const totalIn = scValToNative(call.args()[1]) as bigint
  if (totalIn !== BigInt(expected.totalIn)) {
    throw new Error('envelope total_in does not match caller-requested totalIn')
  }
  if (!Buffer.from(call.args()[2].bytes()).equals(Buffer.from(quote.routeXdr, 'base64'))) {
    throw new Error('swap_xdr does not equal routeXdr')
  }
}

export function verifyRoutePayload(
  quote: StandaloneQuote,
  expected: ExpectedStandalone,
): void {
  verifyRouteBytes(quote, expected)
  verifyStandaloneEnvelope(quote, expected)
}
```

Pass the pair and `totalIn` from caller-owned request state, not values copied from the
response. `router` is the `aggregator` address from `configs/networks.json`;
`/api/v1/config.router` is only a deployment cross-check. Pass the wallet account as
`signer`. For composition, call `verifyRouteBytes` before building and then simulate the
complete controller/contract transaction. For standalone use, call
`verifyRoutePayload` before changing sequence or signing, prepare/simulate, and inspect
`execute_strategy`'s delivered `i128`; a result below `amountOutMin` must be re-quoted.

## SDK encoder surface (`@xoxno/sdk-js` 1.0.214, `src/sdk/stellar/`)

| Export | Use |
|---|---|
| `asStellarStrategySwapBytes(steps: unknown): xdr.ScVal` (`scval-encode.ts`) | Normalizes any accepted form into the `Bytes` argument. Accepts: base64 `routeXdr` string; `0x`-prefixed hex string; `Uint8Array`; `{ routeXdr }`; `{ swapXdr }`; `{ bytes: string \| Uint8Array }`; or a decoded `StellarStrategyPayloadInput` object (validated by `asStellarStrategyPayload`, then re-encoded locally). An empty `Uint8Array`, bare or as `bytes`, yields empty bytes (`emptyStrategySwapBytes`, the same-token passthrough case); an empty string throws |
| `encodeStrategyPayload(payload): xdr.ScVal` | Lowers `{ paths, tokenIn, tokenOut, totalMinOut, referralId?, burnPool?, burnMinAmounts?, mintPool?, mintMinShares?, mintPoolTokens?, preSwapAmount?, preSwapFromA? }` into the `StrategyPayload` map. Throws on same-token, broken chain, more than 48 ops, 32 weights, 256 assets, or 126 amounts, a weight outside `1..=1_000_000`, or a referral outside u32 |
| `encodeStrategyPayloadToBytes(payload): xdr.ScVal` | `scvBytes` of the map's XDR — the contract argument form |
| `encodeStrategyPayloadToRouteXdr(payload): string` (`swap.ts`) | Base64 form, equivalent to the server's `routeXdr` |
| `mapQuoteResponseToStrategySwap(quote, { referralId? })` (`swap.ts`) | Returns `{ routeXdr }` when the quote has a non-empty one; otherwise falls back to `mapQuoteResponseToStrategyPayload`. **Use this** |
| `mapQuoteResponseToStrategyPayload(quote, { referralId? })` | Rebuilds a payload from `paths` (or one path from the flat `hops` when `paths` is absent) with `referralId ?? 0` and `totalMinOut = quote.amountOutMin` (throws when `amountOutMin` is missing). Loses the server-chosen referral and the Rust sweep slack. Inferred: on a multi-path quote fetched without `includePaths`, the concatenated `hops` do not chain and `encodeStrategyPayload` throws `hop does not chain onto its predecessor's output`. Only for tests and custom local routes |
| `buildStellarExecuteStrategyTx({ network, caller, sourceSequence, routerAddress, totalIn, referralId?, fee?, timeoutSeconds? }, swap)` (`swap.ts`) | Unsigned XDR calling `execute_strategy(caller, totalIn, swap)`; `totalIn` = `quote.amountIn`; `swap` = any `asStellarStrategySwapBytes` form; `sourceSequence` is the account's current sequence (`TransactionBuilder` increments). `referralId` applies only when `swap` is a decoded payload without one |
| `STELLAR_SWAP_VENUE_OPCODE`, `STELLAR_PROGRAM_VERSION = 1`, `PPM_DENOMINATOR = 1_000_000` (`scval-encode.ts`) | Constants mirroring `program.rs` |

Standalone sign-and-submit is Example 2 in [SKILL.md](SKILL.md). `buildStellarExecuteStrategyTx` takes `routerAddress` from `configs/networks.json` `aggregator`; assert it equals `/api/v1/config.router`.
