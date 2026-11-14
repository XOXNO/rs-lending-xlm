# Composing aggregator routes into lending strategies and your own contract

One quote yields one `routeXdr`; where it ends up decides how you quote. Wire
format: [payload.md](payload.md); fields: [api.md](api.md); `controller`,
`pool`, and `aggregator` addresses: [addresses](../xoxno-lending/addresses.md).

Sources: `rs-lending-xlm/contracts/controller/src/strategies/{swap,legs,multiply,swap_debt,swap_collateral,repay_debt_with_collateral}.rs`,
`contracts/swap-aggregator/src/{execute/mod,program}.rs`, `common/src/token.rs`,
and the strategy builders in the [pinned SDK](../xoxno-lending/SKILL.md#verified-against).

## Standalone or composed

| | Standalone swap | Composed (controller verb or your contract) |
|---|---|---|
| Who calls `execute_strategy` | The user's account (`sender = G…`) | The controller (`sender = controller`) or your contract (`sender = your contract`) |
| Quote request | `slippage`; no `sender` when building locally | `slippage`, **no `sender`** |
| What you take from the response | Checked `routeXdr` | `routeXdr` only |
| `amountIn` you quote | User's payment | What the calling contract hands the router (table below) |
| Who simulates | You, preparing the locally built transaction | **You**, on the composed transaction (server never sees it) |
| Auth | See [payload.md#authorization-model](payload.md#authorization-model) | Same |
| Min-out enforcement | Router `SlippageExceeded` | Router `SlippageExceeded`; controller adds `RouterOverspend` / `NoSwapOutput`, no min-out |

## Quote for composition

```ts
import { STELLAR_NETWORKS, getStellarAggregatorQuote, mapQuoteResponseToStrategySwap } from '@xoxno/sdk-js/stellar-lending'

const NET = STELLAR_NETWORKS.stellarMainnet

export async function quoteForComposition(
  tokenIn: string,
  tokenOut: string,
  amountTheContractWillSend: string,
  acceptedMinOut: string,
) {
  const quote = await getStellarAggregatorQuote(
    { from: tokenIn, to: tokenOut, amountIn: amountTheContractWillSend, slippage: 0.005, maxHops: 2, maxSplits: 1 },
    { baseUrl: NET.quoteUrl },
  )
  if (!quote.routeXdr || !quote.amountOutMin || quote.amountIn !== amountTheContractWillSend) {
    throw new Error('quote mismatch or missing executable route')
  }
  const steps = mapQuoteResponseToStrategySwap(quote, {
    expected: { tokenIn, tokenOut, amountIn: amountTheContractWillSend, minOut: acceptedMinOut },
  })
  return { quote, steps }
}
```

Pass the checked `steps`; the builders convert the route to Soroban `Bytes`,
which the controller forwards undecoded. Set `referralId` on the quote
request: the server encodes it into `routeXdr` and builders cannot add it
later.

### Exact-out status

Reverse quotes keep the requested output as the minimum and size input with
a slippage-raised output ([formula](api.md#get-apiv1quote)). Check the encoded
floor against the caller's requested output and withdraw the returned
`amountIn`. For `multiply`, subtract the measured debt-asset initial payment
before grossing up the remaining borrow; for `swap_debt`, gross up the full
router input. The router has no maximum-input parameter. This is source
evidence; check hosted deployment behavior separately.

## `amountIn` per lending verb

The controller hands the router exactly what it measured receiving
(`swap_tokens(env, refund_to, token_in, amount_in, token_out, swap)` in
`strategies/swap.rs`); quote that amount.

| Controller verb (SDK builder) | Router `token_in → token_out` | `amountIn` to quote | Where the amount comes from |
|---|---|---|---|
| `multiply` (`buildStellarMultiplyTx`) | `debt.asset → collateral.asset` | `net = debtToFlashLoan − fee` (+ any debt-asset `initialPayment`) | `borrow_into_controller(…, charge_fee = true)` returns the receipt net of flash-loan fee; `swap_amount_in = amount_received + debt_extra` (`multiply.rs`) |
| `multiply` with third-asset `initialPayment` (`convertSwap`) | `payment.asset → collateral.asset` | Measured receipt (can differ from `initialPayment.amount`) | `collect_initial_multiply_payment` swaps the whole receipt |
| `swap_debt` (`buildStellarSwapDebtTx`) | `newDebt.asset → existingDebt.asset` | `newDebtAmount − fee` | Same flash-fee borrow path (`swap_debt.rs`, `borrow_into_controller`) |
| `swap_collateral` (`buildStellarSwapCollateralTx`) | `current.asset → newCollateral.asset` | `fromAmount` (measured withdrawal) | `withdraw_and_swap_from_supply` swaps the withdrawal's balance delta (`legs.rs`) |
| `repay_debt_with_collateral` (`buildStellarRepayDebtWithCollateralTx`) | `collateral.asset → debt.asset` | `collateralAmount` (measured withdrawal) | Same withdrawal leg, then repays with measured output |

`fee` is the [flash-loan fee](../xoxno-lending/math.md#flash-loan-and-strategy-fees)
on the gross amount, with `flashloan_fee` (BPS) from
`pool.get_sync_data(HubAssetKey).params`; it applies even when
`is_flashloanable` is false. Helpers `netAfterFlashFee` and `grossForNet`:
[SDK sizing rule](../xoxno-lending-sdk/strategies.md#sizing-rule-per-verb-what-amountin-must-equal).

A withdraw-all sentinel (`i128::MAX`, `WITHDRAW_ALL_SENTINEL`) hands the
router an amount you cannot know in advance. Quote the current balance. The
first hop may use `Mode::All` or `Mode::Ppm` to consume the measured
withdrawal, but `amountOutMin` stays the **absolute floor for the quoted
amount**, so a smaller actual input can revert with `SlippageExceeded`.
Re-read the position immediately before building, quote slightly less than
the balance, or avoid withdraw-all when the amount can change materially.

`accountNonce`, `mode`, `initialPayment` and `convertSwap` rules:
[../xoxno-lending-sdk/strategies.md#mode-initialpayment-convertswap](../xoxno-lending-sdk/strategies.md#mode-initialpayment-convertswap).

## Same market: empty bytes, not a placeholder

`repay_debt_with_collateral` with `collateral == debt` (same `hub_id` and
`asset`) nets supply against debt in the pool without token movement and
asserts `swap.is_empty()`; any bytes revert `GenericError::InvalidPayments`
(16). `swap_tokens_or_passthrough` applies the same rule to every strategy
whose input and output asset coincide (for example `swap_collateral` between
two hubs of the same asset). Do not quote: pass
`buildSameTokenRepaySwapSteps()` or `new Uint8Array(0)` as `steps` (mapped to
empty `Bytes`). A non-empty same-token route would also fail in the router
with `SameToken` (25).

## Builder walkthroughs

- Repay debt with collateral (same-token aware, exact-out variant):
  [../xoxno-lending-sdk/strategies.md#complete-example-repay-usdc-debt-with-xlm-collateral-same-token-aware](../xoxno-lending-sdk/strategies.md#complete-example-repay-usdc-debt-with-xlm-collateral-same-token-aware)
- Multiply quoted net of the flash-loan fee, built with the gross amount:
  [the SDK Multiply example](../xoxno-lending-sdk/strategies.md#complete-example-3-xlm-in-multiply-mode-funded-by-usdc-debt)

## Budget: why `maxSplits: 1`, `maxHops: 2`

The server's budget ladder (`build_attempt_ladder`) runs only when it
simulates, which requires `sender`, so it never budget-checks a composed
quote. The controller verb adds pool calls, oracle reads and account updates
around the swap. Keep `maxSplits` at 1 and `maxHops` at 2
(`STELLAR_LENDING_QUOTE_MAX_HOPS` / `_MAX_SPLITS` in
`xoxno-ui/src/modules/swap/soroswap.ts`); a 4-hop `swap_collateral` hit
`Budget, ExceededLimit` on testnet. Re-quote with smaller limits when your
simulation fails that way. Caps: [api.md](api.md#get-apiv1quote),
[payload.md](payload.md#packed-program-ops--version-1).

## Re-simulate, then read the revert

Always run `prepareStellarBuiltTx` / `server.simulateTransaction` on the
composed transaction at the ledger you sign at. The
`[xoxno-invoked:<contract>]` label from `prepareStellarBuiltTx(..., { invokedContractId })`
marks only the top-level invocation, not a nested error emitter: pair code and
contract address from the same diagnostic and trace propagated errors before
mapping. Router failures, including `BrokenTokenChain`:
[payload.md#router-errors](payload.md#router-errors). Controller-only guards
are `RouterOverspend` (501), `NoSwapOutput` (502), `InvalidPayments` (16), and
`FlashLoanOngoing` (400); `Budget, ExceededLimit` is a host budget failure.
Re-quote on `SlippageExceeded`, reduce route limits on a budget failure, and
report repeated overspend/no-output failures.

Before building, verify the encoded pair and `min_out` with
[`verifyStellarRouteBytes`](payload.md#verify-routexdr-before-signing).

## How the controller uses the router

The controller never checks a minimum output: the router enforces
`amounts[min_out]` inside `routeXdr` before payout (`SlippageExceeded`), and
the controller credits its measured output balance increase.
`strategy_finalize` applies the manual-borrow LTV and health-factor checks plus
the applicable collateral minimums. The `swap_tokens` auth and measurement
sequence is in
[`strategies/swap.rs`](https://github.com/XOXNO/rs-lending-xlm/blob/main/contracts/controller/src/strategies/swap.rs).

## Your own contract as `sender`

Quote without `sender` (the bytes arrive as an argument), follow the
contract-sender flow in [payload.md](payload.md#authorization-model), and
credit the measured output-balance delta. Rust reference: `swap_tokens` above.
Token-pull rules:
[contracts composing](../xoxno-lending-contracts/composing.md#token-pull-ordering).
