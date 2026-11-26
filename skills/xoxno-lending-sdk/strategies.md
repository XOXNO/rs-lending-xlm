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

The [composition guide](../xoxno-swap-aggregator/composition.md#quote-for-composition)
shows the verified mapping call. Quote-server simulation omits lending accrual,
transfers, pool guards, and post-action risk gates. Prepare the complete
controller transaction before signing.

## Sizing rule per verb (what `amountIn` must equal)

For `multiply` and `swap_debt`, the builder takes the **gross** borrow. The
router receives it net of the flash-loan fee. The
[formula](../xoxno-lending/math.md#flash-loan-and-strategy-fees) uses BPS,
rounds half up, and has a minimum of one base unit when the rate is positive:

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

Quote this input for each verb:

- `multiply`: debt to collateral. The input is
  `netAfterFlashFee(debtToFlashLoan, fee)` plus any debt-token initial payment.
  The builder gets gross `debtToFlashLoan`.
- `swap_debt`: new debt to existing debt. The input is the net amount. The
  builder gets gross `newDebtAmount`.
- `swap_collateral`: current to new collateral. The input is `fromAmount`,
  unchanged.
- `repay_debt_with_collateral`: collateral to debt. The input is
  `collateralAmount`, unchanged.

For a known output, reverse-quote with `amountOut`. The returned `amountIn` is
`collateralAmount` for repay-with-collateral. For swap-debt, gross it up
before you build.

## Same-token routes

When the input and output token addresses are equal, request no quote. Pass the
zero-length bytes from `buildSameTokenRepaySwapSteps()`. A non-empty route between equal tokens,
placeholder or self-swap included, reverts `InvalidPayments`.

`swap_debt` and `swap_collateral` reject identical full `HubAssetKey` values
with `AssetsAreTheSame`. Different hubs with one token take the empty-byte
pass-through. The coordinates must still be valid. The contract rules are in
[composing.md](../xoxno-lending-contracts/composing.md#swap-route-rules).

## `mode`, `initialPayment`, `convertSwap`

The modes are `0` normal, `1` multiply, `2` long, and `3` short. `multiply`
accepts `1..=3`. Mode `1` needs distinct `HubAssetKey` values. Modes `2` and `3`
need distinct token addresses. Otherwise the call fails with
`AssetsAreTheSame`. When you reuse an account, use its stored mode and spoke.
Read them from the account, not the token.

Treat `initialPayment` by its token:

- collateral: the payment is added to supplied collateral.
- debt: the payment is added to the quoted swap input.
- third token: the call needs a second executable `convertSwap` route into
  collateral. Otherwise it fails with `ConvertStepsRequired`.

## Leverage display is not admission

A float like `(leverage - 1) * deposit` is a UI estimate. It never sets
`debtToFlashLoan` or quote input, and it never marks the action guaranteed.

Base builder and risk decisions on these values:

- token amounts as base-unit `bigint`.
- raw WAD price strings and BPS weights.
- protocol rounding and flash fee in integer arithmetic.
- expected collateral valued at `amountOutMin`.
- an application safety margin in raw WAD and BPS values.
- successful preparation of the composed transaction.

The [math reference](../xoxno-lending/math.md) has the formulas. A risk preview
uses current raw contract state and is an estimate.

A careful estimate or earlier simulation can still fail because of these
conditions:

- price or index movement, or oracle staleness or deviation checks.
- minimum collateral, LTV, health-factor, cap, pause, freeze, cash,
  liquidation-buffer, or utilization guards.
- account ownership, delegate, mode, or spoke authorization.
- missing classic-asset trustlines.
- stale route bytes, slippage, venue failure, or no swap output.
- Soroban instruction, memory, resource, or footprint limits.
- sequence consumption, fee conditions, or timebounds expiry.

### Complete example: 3× XLM in Multiply mode, funded by USDC debt

1. Set the user's XLM as `initialPayment`.
2. Calculate the additional XLM in token base units.
3. Convert its WAD value to a net USDC amount.
4. Include the USDC strategy fee in the gross debt amount.
5. Quote the net USDC to XLM input.
6. Value `initialPayment + amountOutMin` against gross debt.
7. Build with `mode: 1`.
8. Prepare the complete controller transaction.

For a Long account, use `mode: 2`.

## Repay and close

`closePosition: true` requires zero debt positions after repayment, or the call
fails with `CannotCloseWithRemainingDebt`. After repayment, it withdraws every
supply position to the caller. Size reverse quotes from ceiled live debt plus a
small accrual buffer. The caller receives a refund of surplus debt tokens. `accountNonce: '0'` is never
valid here.

### Complete example: repay USDC debt with XLM collateral, same-token aware

If collateral and debt token addresses match, use
`buildSameTokenRepaySwapSteps()` without a quote. Otherwise:

1. Reverse-quote the ceiling-rounded debt with an accrual buffer.
2. Use `quote.amountIn` as `collateralAmount`.
3. Verify and map the executable route into `steps`.

Set `closePosition` explicitly. Prepare the complete controller transaction.

Re-quote when amount, direction, slippage, or reserve state changes, and
after meaningful idle time. Route bytes live in the prepared envelope, so a
re-quote means a fresh build and preparation.

For nested failures in the controller, pool, router, or DEX, see
[Error interpretation](transactions.md#error-interpretation).
