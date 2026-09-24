# Composing aggregator routes into lending strategies and your own contract

One quote yields one `routeXdr`. Where it ends up decides how you quote. Wire format in
[payload.md](payload.md); request and response fields in [api.md](api.md); contract and
router addresses (`controller`, `pool`, `aggregator` in `configs/networks.json`) in
[../xoxno-lending/addresses.md](../xoxno-lending/addresses.md).

Sources: `arb-algo/scripts/check-stellar-{lending,lp}-composition.ts`,
`rs-lending-xlm/contracts/controller/src/strategies/{swap,legs,multiply,swap_debt,swap_collateral,repay_debt_with_collateral}.rs`,
`contracts/swap-aggregator/src/{execute/mod,program}.rs`, `common/src/token.rs`,
`sdk-js/src/sdk/stellar/{lending,scval-encode,swap,repay-swap,quote,prepare}.ts` (1.0.214).

## Standalone or composed

| | Standalone swap | Composed (controller verb or your contract) |
|---|---|---|
| Who calls `execute_strategy` | The user's account (`sender = G…`) | The controller (`sender = controller`) or your contract (`sender = your contract`) |
| Quote request | `slippage` + `sender` | `slippage`, **no `sender`** |
| What you take from the response | `transaction.envelopeXdr` (or `routeXdr` for `buildStellarExecuteStrategyTx`) | `routeXdr` only |
| `amountIn` you quote | What the user pays | What the calling contract will hand the router (table below) |
| Who simulates | The server (when `sender` is set) or you | **You**, on the composed transaction; the server never sees it |
| Auth | See [payload.md#authorization-model](payload.md#authorization-model) | Same |
| Min-out enforcement | Router `SlippageExceeded` | Router `SlippageExceeded`; the controller adds `RouterOverspend` / `NoSwapOutput` but no min-out of its own |

## Quote for composition

```ts
import { STELLAR_NETWORKS, getStellarAggregatorQuote } from '@xoxno/sdk-js/stellar-lending'
import { verifyRouteBytes } from './verify-route-payload.js'

const NET = STELLAR_NETWORKS.stellarMainnet

export async function quoteForComposition(
  tokenIn: string,
  tokenOut: string,
  amountTheContractWillSend: string,
) {
  const quote = await getStellarAggregatorQuote(
    { from: tokenIn, to: tokenOut, amountIn: amountTheContractWillSend, slippage: 0.005, maxHops: 2, maxSplits: 1 },
    { baseUrl: NET.quoteUrl },
  )
  if (!quote.routeXdr || !quote.amountOutMin || quote.amountIn !== amountTheContractWillSend) {
    throw new Error('quote mismatch or missing executable route')
  }
  verifyRouteBytes(
    { ...quote, routeXdr: quote.routeXdr, amountOutMin: quote.amountOutMin },
    { tokenIn, tokenOut },
  )
  return quote
}
```

`routeXdr` is present when `slippage` was sent (always for swaps; optional for
`convertLiquidity`; `pipeline.rs` clears it when `slippage` is absent). Pass it
untouched: `steps: { routeXdr: quote.routeXdr }` or
`mapQuoteResponseToStrategySwap(quote)` (returns `{ routeXdr }` when present). The
builders decode base64 into Soroban `Bytes` via `asStellarStrategySwapBytes`; the
controller forwards those bytes to the router as `swap_xdr` without decoding them.
Set `referralId` on the quote request: the server encodes it into `routeXdr`, and the
builders cannot add it later.

### Exact-out status

Exact-out preservation is **external, pinned quote-server behavior**, not an invariant
proved by this repository. At `arb-algo` commit `f2d5fe9`,
`preserve_output_minimum` keeps `amountOutMin >= requested amountOut`; this makes
exact-out useful when the target matters (for example, repaying N debt). The hosted
server can change, so check that `amountOutMin` is at least the `amountOut` you
requested before you build. The returned `amountIn` is what the router must receive.
Withdraw it unchanged. For a borrow (`multiply`, `swap_debt`), subtract any debt-asset
`initialPayment`, then borrow `grossForNet` of the rest. The router has no separate
maximum-input guard.

## `amountIn` per lending verb

The controller measures what it actually receives and hands exactly that to the router
(`swap_tokens(env, refund_to, token_in, amount_in, token_out, swap)` in
`strategies/swap.rs`). Quote the amount the router will see.

| Controller verb (SDK builder) | Router `token_in → token_out` | `amountIn` to quote | Where the amount comes from |
|---|---|---|---|
| `multiply` (`buildStellarMultiplyTx`) | `debt.asset → collateral.asset` | `net = debtToFlashLoan − fee` (+ an `initialPayment` made in the debt asset, if any) | `borrow_into_controller(…, charge_fee = true)` returns the measured receipt net of the pool's flash-loan fee; `swap_amount_in = amount_received + debt_extra` (`multiply.rs`) |
| `multiply` with `initialPayment` in a third asset (`convertSwap`) | `payment.asset → collateral.asset` | `initialPayment.amount` (measured receipt) | `collect_initial_multiply_payment` swaps the whole received payment |
| `swap_debt` (`buildStellarSwapDebtTx`) | `newDebt.asset → existingDebt.asset` | `newDebtAmount − fee` | Same flash-fee borrow path (`swap_debt.rs`, `borrow_into_controller`) |
| `swap_collateral` (`buildStellarSwapCollateralTx`) | `current.asset → newCollateral.asset` | `fromAmount` (measured withdrawal) | `withdraw_and_swap_from_supply` swaps the balance delta of the withdrawal (`legs.rs`) |
| `repay_debt_with_collateral` (`buildStellarRepayDebtWithCollateralTx`) | `collateral.asset → debt.asset` | `collateralAmount` (measured withdrawal) | Same withdrawal leg; then repays with the measured swap output |

`fee = round_half_up(gross × flashloan_fee / 10_000)`, min 1 when the rate is positive
([../xoxno-lending/math.md#flash-loan-and-strategy-fees](../xoxno-lending/math.md#flash-loan-and-strategy-fees)).
Read `flashloan_fee` (BPS) from `pool.get_sync_data(HubAssetKey).params` or the reserve
DTO's `flashloanFeeBps`. The strategy fee applies even when `is_flashloanable` is false.
Helpers `netAfterFlashFee` and `grossForNet`:
[../xoxno-lending-sdk/strategies.md#sizing-rule-per-verb-what-amountin-must-equal](../xoxno-lending-sdk/strategies.md#sizing-rule-per-verb-what-amountin-must-equal).

A withdrawal sized with the withdraw-all sentinel (`i128::MAX`, `WITHDRAW_ALL_SENTINEL`)
hands the router an amount you cannot know exactly in advance. Quote the current position
balance. The first hop may use `Mode::All` or `Mode::Ppm` and consume the measured
withdrawal, but `amountOutMin` stays the **absolute floor encoded for the quoted
amount**. A smaller actual input can give an output below that floor and revert with
`SlippageExceeded`. Re-read the position immediately before you build, quote slightly
less than the balance, or do not use withdraw-all when the amount can change materially.
Always simulate the complete transaction.

`accountNonce`, `mode`, `initialPayment` and `convertSwap` rules:
[../xoxno-lending-sdk/strategies.md#mode-initialpayment-convertswap](../xoxno-lending-sdk/strategies.md#mode-initialpayment-convertswap).

## Same market: empty bytes, not a placeholder

`repay_debt_with_collateral` with `collateral == debt` (same `hub_id` and `asset`) nets
supply against debt in the pool without any token movement and asserts
`swap.is_empty()`; any bytes revert `GenericError::InvalidPayments` (16). The
pass-through in `swap_tokens_or_passthrough` applies the same rule to every strategy
whose input and output asset coincide (for example `swap_collateral` between two hubs
of the same asset). Pass `buildSameTokenRepaySwapSteps()` or `new Uint8Array(0)` as
`steps`; `asStellarStrategySwapBytes` maps an empty `Uint8Array` to empty `Bytes`. A
non-empty route into a same-token swap would also fail inside the router with
`SameToken` (25). Do not quote at all in this case.

## Builder walkthroughs

- Repay debt with collateral (same-token aware, exact-out variant):
  [../xoxno-lending-sdk/strategies.md#complete-example-repay-usdc-debt-with-xlm-collateral-same-token-aware](../xoxno-lending-sdk/strategies.md#complete-example-repay-usdc-debt-with-xlm-collateral-same-token-aware)
- Multiply quoted net of the flash-loan fee, built with the gross amount:
  [../xoxno-lending-sdk/strategies.md#complete-example-3-long-on-xlm-funded-by-usdc-debt](../xoxno-lending-sdk/strategies.md#complete-example-3-long-on-xlm-funded-by-usdc-debt)

## Budget: why `maxSplits: 1`, `maxHops: 2`

The server's budget ladder (`build_attempt_ladder`) runs only when the server simulates,
and simulation requires `sender`. The server never budget-checks a composed quote. The
controller verb adds pool calls, oracle reads and account updates around the swap. For
composition, keep `maxSplits` at 1 and `maxHops` at 2 (`STELLAR_LENDING_QUOTE_MAX_HOPS`
/ `_MAX_SPLITS` in `xoxno-ui/src/modules/swap/soroswap.ts`); a 4-hop `swap_collateral`
hit `Budget, ExceededLimit` on testnet. If your own simulation fails with
`Budget, ExceededLimit`, re-quote with smaller limits. Request and program caps:
[api.md#get-apiv1quote](api.md#get-apiv1quote), [payload.md](payload.md#packed-program-ops--version-1).

## Re-simulate, then read the revert

Always run `prepareStellarBuiltTx` / `server.simulateTransaction` on the composed
transaction at the ledger you sign at. `prepareStellarBuiltTx(..., { invokedContractId })`
prefixes errors with `[xoxno-invoked:<contract>]` so a numeric code is mapped against
the right ABI. Router failures, including `BrokenTokenChain`, are
centralized in [payload.md#router-errors](payload.md#router-errors). Controller-only
guards are `RouterOverspend` (501), `NoSwapOutput` (502), `InvalidPayments` (16), and
`FlashLoanOngoing` (400); `Budget, ExceededLimit` is a host budget failure. Re-quote on
`SlippageExceeded`, reduce route limits on a budget failure, and report repeated
overspend/no-output failures.

Verify the encoded pair and `min_out` before building with `verifyRouteBytes`:
[payload.md#verify-routexdr-before-signing](payload.md#verify-routexdr-before-signing).

## How the controller uses the router

The controller never checks a minimum output. The only slippage floor is
`amounts[min_out]` inside `routeXdr`, enforced by the router before payout
(`execute/mod.rs`: `total_out < total_min_out → SlippageExceeded`); the controller then
credits its measured output-balance delta, and the verb's own risk checks
(`strategy_finalize`) apply the same LTV / health-factor gates as a manual borrow. The
`swap_tokens` auth and measurement sequence is in
[`strategies/swap.rs`](../../contracts/controller/src/strategies/swap.rs).

## Your own contract as `sender`

Use the contract-sender flow in
[payload.md#authorization-model](payload.md#authorization-model), then credit the
measured output-balance delta. Quote without `sender`; the bytes arrive as an argument.
Rust reference: `swap_tokens` in
[`strategies/swap.rs`](../../contracts/controller/src/strategies/swap.rs). Token-pull
rules:
[../xoxno-lending-contracts/composing.md#token-pull-ordering](../xoxno-lending-contracts/composing.md#token-pull-ordering).
