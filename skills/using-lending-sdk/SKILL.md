---
name: using-lending-sdk
description: Use when building off-chain TypeScript/JavaScript against XOXNO Lending — assembling supply/borrow/withdraw/repay/liquidate/flash-loan transactions with @xoxno/sdk-js, leverage and debt/collateral swaps via the aggregator quote server, Blend migration, or the REST read surface.
---

# Using the XOXNO Lending SDK (off-chain)

**REQUIRED BACKGROUND:** the `lending-protocol-fundamentals` skill.

## Setup

Everything lives under the `stellar-lending` subpath of `@xoxno/sdk-js`
(symbols are also re-exported from the package root):

```ts
import { buildStellarSupplyTx, mapSorobanError } from '@xoxno/sdk-js/stellar-lending'
```

Contract addresses and quote-server URLs are **env-sourced per network**
(`STELLAR_LENDING_CONTROLLER_<NETWORK>`, `STELLAR_AGGREGATOR_ROUTER_<NETWORK>`,
`STELLAR_QUOTE_URL_<NETWORK>`); resolution helpers like
`getStellarLendingController(network)` throw when unset. Keep `network`
(`'testnet' | 'mainnet'`) in your app config — never literal in code.

## Builder contract (applies to every builder)

Builders map 1:1 onto controller entrypoints and return an **unsigned** XDR
built from a synthetic source account:

```ts
const NETWORK = config.stellarNetwork // 'testnet' | 'mainnet'
const { xdr } = buildStellarSupplyTx(
  { network: NETWORK, caller, sourceSequence },
  { hubId, asset, amount: '100000000', accountNonce: 0, spokeId },
)
// then ALWAYS: rpc.Server.prepareTransaction (simulation adds footprint,
// auth entries, resource fees) -> sign -> send
```

- `amount` values are i128 decimal strings in native asset decimals.
- `accountNonce` is the account id; `0` opens a new account.
- **Always pass `spokeId` explicitly when creating an account.** Spoke ids
  start at 1; the builder defaults an omitted `spokeId` to `0`, which reverts
  `SpokeNotFound` on account creation.

## Core builders

Single-asset: `buildStellarSupplyTx`, `buildStellarBorrowTx`,
`buildStellarWithdrawTx`, `buildStellarRepayTx` — args extend
`{ hubId, asset, amount }` (borrow/withdraw accept optional `to`).

Batch variants take an args object, not a bare array:

```ts
buildStellarSupplyBatchTx(opts,  { accountNonce, spokeId, assets: [{ hubId, asset, amount }, ...] })
buildStellarBorrowBatchTx(opts,  { accountNonce, borrows: [...], to? })
buildStellarWithdrawBatchTx(opts,{ accountNonce, withdrawals: [...], to? })
buildStellarRepayBatchTx(opts,   { accountNonce, payments: [...] })
```

Also: `buildStellarLiquidateTx({ accountNonce, debtPayments })` (see
`building-lending-liquidation-bots`) and
`buildStellarFlashLoanTx({ hubId, asset, amount, receiver, data })` (receiver
side: `writing-flash-loan-receivers`).

## Strategy verbs (leverage, swaps, deleverage, migration)

`multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, and
`migrate_from_blend` combine an internal flash loan with a DEX-aggregator
route passed as opaque bytes. The route comes from the quote server — never
hand-encode it:

```ts
import {
  getStellarAggregatorQuote, mapQuoteResponseToStrategySwap,
  buildStellarMultiplyTx,
} from '@xoxno/sdk-js/stellar-lending'

// 1. Quote. Exactly one of amountIn/amountOut. slippage is a decimal
//    fraction (0.01 = 1%) and is required for the mappers' amountOutMin.
//    NOTE for multiply: the on-chain swap input is the flash-loaned amount
//    NET of the market's flash-loan fee (plus any same-token initial
//    payment) — quote for the net amount.
const quote = await getStellarAggregatorQuote(
  { from: debtAsset, to: collateralAsset, amountIn: netFlashLoanAmount, slippage: 0.005 },
  { network: NETWORK },
)

// 2. Map to the builders' steps input (uses server routeXdr when present)
const steps = mapQuoteResponseToStrategySwap(quote)

// 3. Build, prepare, sign
const { xdr } = buildStellarMultiplyTx(opts, {
  accountNonce: 0, spokeId,
  collateral: { hubId, asset: collateralAsset },
  debtToFlashLoan: flashLoanAmount,
  debt: { hubId, asset: debtAsset },
  mode, steps,
})
```

`buildStellarSwapDebtTx`, `buildStellarSwapCollateralTx`,
`buildStellarRepayDebtWithCollateralTx`, `buildStellarMigrateFromBlendTx`
follow the same quote→map→build pattern (migration: check
`is_blend_pool_approved` first; it runs at zero flash-loan fee). For a plain
user swap without a lending account, `buildStellarExecuteStrategyTx` targets
the aggregator router directly.

Strategy verbs are atomic — the post-state must pass the same LTV/HF gates as
a manual borrow or everything reverts; the router credits measured balance
deltas, so venues cannot fake output. Re-quote close to submission.

## Read surface

```ts
import { stellarLendingRead } from '@xoxno/sdk-js/stellar-lending'

const read = stellarLendingRead(client) // XOXNOClient
// read.assets / hubs / spokes / reserves / reserve / userPositions
// read.accountPositions / userActivity / assetMarkets / governanceProposals
```

Standalone equivalents carry a `getStellar*` prefix (`getStellarAssets`, …).
These return enriched, price-annotated REST views; for on-chain truth
simulate the contract views (`reading-lending-protocol-state`).

## Errors

Map simulation/submission failures to protocol error names with
`mapSorobanError` from the same subpath.

## Common mistakes

- **Importing from `@xoxno/sdk-js/stellar`** — the subpath is
  `stellar-lending`; `/stellar` does not resolve.
- **Skipping prepareTransaction** — builders return raw XDR without
  footprint, auth entries, or fees; submission fails without simulation.
- **Omitting `spokeId` on account creation** — defaults to 0 and reverts
  `SpokeNotFound` (spoke ids start at 1; see fundamentals + INVARIANTS §5.2).
- **Passing a bare array to batch builders** — they take
  `{ accountNonce, assets|borrows|withdrawals|payments: [...] }`.
- **Quoting `debt_to_flash_loan` gross for multiply** — the swapped amount is
  net of the flash-loan fee; a gross quote overstates output and can revert
  on `totalMinOut`.
