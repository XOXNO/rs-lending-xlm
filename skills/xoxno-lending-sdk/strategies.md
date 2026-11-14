# Strategy verbs: quote bytes inside lending

Companion to [SKILL.md](SKILL.md). `multiply`, `swap_debt`,
`swap_collateral`, and `repay_debt_with_collateral` execute an aggregator route
in one atomic controller call. Never sign the quote server's standalone
transaction for them.

## Composition pipeline

1. Select the account and exact `(spokeId, hubId, asset)` reserves.
2. Compute the controller's actual swap input as base-unit `bigint`.
3. Request a quote with `slippage`, `platform: 'aggregator'`, `maxHops: 2`,
   `maxSplits: 1`, and `includePaths: true`.
4. Omit `sender` and `router`; composition needs only `routeXdr`.
5. Verify the quote pair, input amount, and accepted output floor.
6. Map the route with caller-owned expectations via `mapQuoteResponseToStrategySwap`.
7. Build the lending operation.
8. Follow the [transaction lifecycle](transactions.md#canonical-lifecycle).

Verified mapping call:
[composition guide](../xoxno-swap-aggregator/composition.md#quote-for-composition).
Quote-server simulation omits lending accrual, transfers, pool guards, and
post-action risk gates; prepare the complete controller transaction before
signing.

## Sizing rule per verb (what `amountIn` must equal)

For `multiply` and `swap_debt`, the builder takes the **gross** borrow; the
router receives it net of the flash-loan fee
([formula](../xoxno-lending/math.md#flash-loan-and-strategy-fees): BPS,
half up, minimum one base unit when the rate is positive):

```ts
export function netAfterFlashFee(
  gross: bigint,
  flashloanFeeBps: number,
): bigint {
  const bps = BigInt(flashloanFeeBps)
  let fee = (gross * bps + 5_000n) / 10_000n
  if (bps > 0n && fee === 0n) fee = 1n
  return gross - fee
}

export function grossForNet(
  net: bigint,
  flashloanFeeBps: number,
): bigint {
  let gross =
    (net * 10_000n) / (10_000n - BigInt(flashloanFeeBps))
  while (netAfterFlashFee(gross, flashloanFeeBps) < net) gross += 1n
  return gross
}
```

Quote input by verb:

- `multiply`: debt → collateral, `netAfterFlashFee(debtToFlashLoan, fee)` plus
  any debt-token initial payment; builder gets gross `debtToFlashLoan`.
- `swap_debt`: new debt → existing debt, net input; builder gets gross
  `newDebtAmount`.
- `swap_collateral`: current → new collateral, `fromAmount` unchanged.
- `repay_debt_with_collateral`: collateral → debt, `collateralAmount`
  unchanged.

For a known output, reverse-quote with `amountOut`. The returned `amountIn` is
`collateralAmount` for repay-with-collateral; for swap-debt, gross it up
before building.

## Same-token routes

Equal input and output token addresses: no quote; pass the zero-length bytes
from `buildSameTokenRepaySwapSteps()`. A non-empty route between equal tokens,
placeholder or self-swap included, reverts `InvalidPayments`.

`swap_debt` and `swap_collateral` reject identical full `HubAssetKey` values
with `AssetsAreTheSame`. Different hubs with one token take the empty-byte
pass-through; coordinates must still be valid. Contract rules:
[composing.md](../xoxno-lending-contracts/composing.md#swap-route-rules).

## `mode`, `initialPayment`, `convertSwap`

Modes: `0` normal, `1` multiply, `2` long, `3` short; `multiply` accepts
`1..=3`. Mode `1` needs distinct `HubAssetKey` values, modes `2` and `3`
distinct token addresses, else `AssetsAreTheSame`. Reusing an account needs
its stored mode and spoke; read them from the account, not the token.

`initialPayment` by token:

- collateral: added to supplied collateral;
- debt: added to the quoted swap input;
- third token: needs a second executable `convertSwap` route into collateral,
  else `ConvertStepsRequired`.

## Leverage display is not admission

A float like `(leverage - 1) * deposit` is a UI estimate; it never sets
`debtToFlashLoan` or quote input, or marks the action guaranteed.

Builder and risk decisions:

- token amounts as base-unit `bigint`;
- raw WAD price strings and BPS weights;
- protocol rounding and flash fee in integer arithmetic;
- expected collateral valued at `amountOutMin`;
- an application safety margin in raw WAD/BPS;
- successful preparation of the composed transaction.

Formulas: [../xoxno-lending/math.md](../xoxno-lending/math.md). A risk preview
uses current raw contract state and is an estimate.

A careful estimate or earlier simulation can still fail on:

- price/index movement or oracle stale/deviation checks;
- minimum collateral, LTV, health-factor, cap, pause/freeze, cash,
  liquidation-buffer, or utilization guards;
- account ownership, delegate, mode, or spoke authorization;
- missing classic-asset trustlines;
- stale route bytes, slippage, venue failure, or no swap output;
- Soroban instruction/memory/resource or footprint limits;
- sequence consumption, fee conditions, or timebounds expiry.

### Complete example: 3× XLM in Multiply mode, funded by USDC debt

1. Set the user's XLM as `initialPayment`.
2. Calculate the additional XLM in token base units.
3. Convert its WAD value to a net USDC amount.
4. Include the USDC strategy fee in the gross debt amount.
5. Quote the net USDC → XLM input.
6. Value `initialPayment + amountOutMin` against gross debt.
7. Build with `mode: 1`.
8. Prepare the complete controller transaction.

For a Long account, use `mode: 2`.

## Repay and close

`closePosition: true` requires zero debt positions after repayment (else
`CannotCloseWithRemainingDebt`), then withdraws every supply position to the
caller. Size reverse quotes from ceiled live debt plus a small accrual buffer;
surplus debt tokens are refunded to the caller. `accountNonce: '0'` is never
valid here.

### Complete example: repay USDC debt with XLM collateral, same-token aware

If collateral and debt token addresses match, use
`buildSameTokenRepaySwapSteps()` without a quote. Otherwise:

1. Reverse-quote the ceiling-rounded debt with an accrual buffer.
2. Use `quote.amountIn` as `collateralAmount`.
3. Verify and map the executable route into `steps`.

Set `closePosition` explicitly; prepare the complete controller transaction.

Re-quote when amount, direction, slippage, or reserve state changes, and
after meaningful idle time. Route bytes live in the prepared envelope, so a
re-quote means a fresh build and preparation.

Nested failures (controller, pool, router, or DEX): see
[transactions.md#error-interpretation](transactions.md#error-interpretation).
