# Strategy verbs: quote bytes inside lending

Companion to [SKILL.md](SKILL.md). `multiply`, `swap_debt`,
`swap_collateral`, and `repay_debt_with_collateral` execute an aggregator route
inside one atomic controller call. The quote server's standalone transaction
is not a lending transaction and must never be signed for these verbs.

## Composition pipeline

1. Select the account and exact `(spokeId, hubId, asset)` reserves.
2. Compute the controller's actual swap input in base-unit `bigint`.
3. Request a quote with `slippage`, `platform: 'aggregator'`, `maxHops: 2`,
   `maxSplits: 1`, and `includePaths: true`.
4. Omit `sender` and `router`; composition needs only `routeXdr`.
5. Verify the quote pair, input amount, and accepted output floor.
6. Map the route with caller-owned expectations using `mapQuoteResponseToStrategySwap`.
7. Build the lending operation.
8. Follow the [transaction lifecycle](transactions.md#canonical-lifecycle).

The verified mapping call is in the
[composition guide](../xoxno-swap-aggregator/composition.md#quote-for-composition).

Prepare the complete controller transaction before signing. A quote-server simulation does not include lending accrual,
transfers, pool guards, or post-action risk gates.

## Sizing rule per verb (what `amountIn` must equal)

For `multiply` and `swap_debt`, the builder's borrowed amount is **gross** but
the router receives the amount net of the flash-loan fee. The fee is BPS,
rounded half up, with a minimum of one base unit when the rate is positive:

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

Sizing by verb:

- `multiply`: quote debt → collateral with
  `netAfterFlashFee(debtToFlashLoan, fee)`, plus an initial payment when that
  payment is the debt token; pass gross `debtToFlashLoan` to the builder.
- `swap_debt`: quote new debt → existing debt with net input; pass gross
  `newDebtAmount` to the builder.
- `swap_collateral`: quote current → new collateral with `fromAmount`
  unchanged.
- `repay_debt_with_collateral`: quote collateral → debt with
  `collateralAmount` unchanged.

When the desired output is known, request reverse mode with `amountOut`.
For repay-with-collateral use returned `amountIn` as `collateralAmount`; for
swap-debt gross up returned `amountIn` before building.

## Same-token routes

When input and output token addresses are equal, no quote is required. Pass
the zero-length bytes returned by `buildSameTokenRepaySwapSteps()`. A
non-empty route between equal tokens, including a placeholder or self-swap,
reverts with `InvalidPayments`.

`swap_debt` and `swap_collateral` reject identical full `HubAssetKey` values
with `AssetsAreTheSame`.
Different hubs with the same token can use the pass-through empty-byte path,
but the account and reserve coordinates must still be valid.

## `mode`, `initialPayment`, `convertSwap`

Stellar position modes are `0` normal, `1` multiply, `2` long, and `3` short.
`multiply` accepts modes `1..=3`. Mode `1` requires distinct `HubAssetKey`
values; modes `2` and `3` require distinct token addresses. Otherwise the call
reverts with `AssetsAreTheSame`. Reusing an account requires its stored mode
and spoke; read both from that account rather than deriving them from the
selected token.

`initialPayment` behavior:

- collateral token: add directly to supplied collateral;
- debt token: add to the quoted swap input;
- third token: provide a second executable `convertSwap` route into
  collateral, or the call reverts with `ConvertStepsRequired`.

## Leverage display is not admission

A float calculation such as `(leverage - 1) * deposit` is a UI estimate only.
It must not determine `debtToFlashLoan`, quote input, or whether the action is
offered as guaranteed.

For builder and risk decisions:

- parse token amounts into base-unit `bigint`;
- use raw WAD price strings and BPS weights;
- apply protocol rounding and the flash fee in integer arithmetic;
- value expected collateral at `amountOutMin`;
- enforce an application safety margin in raw WAD/BPS;
- still require successful preparation of the composed transaction.

Application formulas are pseudocode in
[../xoxno-lending/math.md](../xoxno-lending/math.md).
Use current raw contract state for a risk preview. Treat the result as an
estimate. Simulate the complete transaction before signing.

Even a careful client estimate or earlier simulation can fail later because
of:

- price/index movement or oracle stale/deviation checks;
- minimum collateral, LTV, health-factor, cap, pause/freeze, cash,
  liquidation-buffer, or utilization guards;
- account ownership, delegate, mode, or spoke authorization;
- missing classic-asset trustlines;
- stale route bytes, slippage, venue failure, or no swap output;
- Soroban instruction/memory/resource or footprint limits;
- sequence consumption, fee conditions, or timebounds expiry.

### Complete example: 3× long on XLM funded by USDC debt

1. Set the user's XLM as `initialPayment`.
2. Calculate the additional XLM in token base units.
3. Convert its WAD value to a net USDC amount.
4. Include the USDC strategy fee in the gross debt amount.
5. Quote the net USDC → XLM input.
6. Value `initialPayment + amountOutMin` against gross debt.
7. Build with `mode: 1`.
8. Prepare the complete controller transaction.

This produces a Multiply account. The leverage calculation is a preview.

## Repay and close

`closePosition: true` requires the account to hold no debt position after the
repayment, or the call reverts with `CannotCloseWithRemainingDebt`. It then
withdraws every supply position to the caller. Size reverse quotes from the
ceiled live debt and include a small accrual buffer; the controller refunds
surplus debt tokens to the caller. `accountNonce: '0'` is never valid for
repay-with-collateral.

### Complete example: repay USDC debt with XLM collateral, same-token aware

If collateral and debt token addresses match, use
`buildSameTokenRepaySwapSteps()` without a quote. Otherwise:

1. Reverse-quote the ceiling-rounded debt with an accrual buffer.
2. Use `quote.amountIn` as `collateralAmount`.
3. Verify and map the executable route into `steps`.

Set `closePosition` explicitly. Prepare the complete controller transaction.

Re-quote whenever amount, direction, slippage, or reserve state changes, and
again after meaningful user idle time. Route bytes are part of the prepared
envelope, so a re-quote requires a fresh build and preparation.

Nested strategy failures may originate in the controller, pool, router, or
DEX. Do not map the numeric code from the top-level controller tag. Follow
[transactions.md#error-interpretation](transactions.md#error-interpretation)
and leave the failure unmapped unless diagnostics identify the emitter.
