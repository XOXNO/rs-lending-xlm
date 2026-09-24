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
5. Require an executable `routeXdr` and map it with
   `mapQuoteResponseToStrategySwap(quote)`.
6. Build the lending verb, then follow
   [transactions.md#canonical-lifecycle](transactions.md#canonical-lifecycle).

Preparation of the complete controller transaction is the only useful
simulation. A quote-server simulation does not include lending accrual,
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
`@xoxno/sdk-js@1.0.214` exports no `projectAccountRisk` or `maxBorrow`. Later
SDK versions export both; do not present their output as an admission
guarantee.

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

Use `initialPayment` for the user's XLM, derive the desired extra XLM in raw
base units, convert its WAD value to a net USDC amount, gross that amount up
for the USDC flash fee, quote the net USDC → XLM route, and value
`initialPayment + amountOutMin` against gross debt. This is a preview only;
build `mode: 1` and require composed preparation before signing.

## Repay and close

`closePosition: true` requires the account to hold no debt position after the
repayment, or the call reverts with `CannotCloseWithRemainingDebt`. It then
withdraws every supply position to the caller. Size reverse quotes from the
ceiled live debt and include a small accrual buffer; the controller refunds
surplus debt tokens to the caller. `accountNonce: '0'` is never valid for
repay-with-collateral.

### Complete example: repay USDC debt with XLM collateral, same-token aware

For XLM → USDC, reverse-quote the buffered ceiled debt and use `quote.amountIn`
as `collateralAmount`; map its executable route into `steps`. If collateral
and debt token addresses match, skip the quote and pass
`buildSameTokenRepaySwapSteps()`. Set `closePosition` explicitly and prepare
the complete controller transaction.

Re-quote whenever amount, direction, slippage, or reserve state changes, and
again after meaningful user idle time. Route bytes are part of the prepared
envelope, so a re-quote requires a fresh build and preparation.

Nested strategy failures may originate in the controller, pool, router, or
DEX. Do not map the numeric code from the top-level controller tag. Follow
[transactions.md#error-interpretation](transactions.md#error-interpretation)
and leave the failure unmapped unless diagnostics identify the emitter.
