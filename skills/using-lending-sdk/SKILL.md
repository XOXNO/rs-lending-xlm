---
name: using-lending-sdk
description: Use when building off-chain TypeScript/JavaScript against XOXNO Lending — assemble, prepare, sign, and submit SDK transactions; obtain aggregator routes for strategy swaps; migrate Blend positions; or query the REST read surface. Do not use for Soroban-to-Soroban integrations, contract receivers, or direct on-chain views.
---

# Using the XOXNO Lending SDK (off-chain)

Read `lending-protocol-fundamentals` first. It defines hubs, spokes, account
ids, asset units, and the health-factor constraints this SDK cannot bypass.

## When to use

- Build a wallet-facing XDR for a lending action with `@xoxno/sdk-js`.
- Quote and encode an aggregator route for multiply, debt swap, collateral
  swap, or collateral-funded repayment.
- Read indexed lending data through `XOXNOClient`.
- Move an approved Blend position into XOXNO Lending.

## When not to use

- Calling the controller from a Soroban contract — use
  `integrating-lending-from-soroban-contracts`.
- Implementing a flash-loan receiver — use `writing-flash-loan-receivers`.
- Requiring an authoritative current-ledger value before a state-changing
  decision — use `reading-lending-protocol-state` and simulate the relevant
  controller view.
- Building or operating liquidations — use `building-lending-liquidation-bots`.

## Setup

Everything lives under the `stellar-lending` subpath of `@xoxno/sdk-js`
(symbols are also re-exported from the package root):

```ts
import {
  buildStellarSupplyTx,
  mapSorobanError,
  prepareStellarTxXdr,
} from '@xoxno/sdk-js/stellar-lending'
```

Controller and router addresses are **env-sourced per network**
(`STELLAR_LENDING_CONTROLLER_<NETWORK>`, `STELLAR_AGGREGATOR_ROUTER_<NETWORK>`)
and their resolution helpers throw when unset. Quote-server URLs accept
`STELLAR_QUOTE_URL_<NETWORK>` overrides but have SDK defaults. Keep `network`
(`'testnet' | 'mainnet'`) in one application configuration boundary; do not
mix it with addresses, RPC servers, or signatures for another network.

## Transaction lifecycle (all builders)

Controller-backed lending builders return an **unsigned** XDR built from a
synthetic source account. Single-asset supply, borrow, withdraw, and repay
helpers wrap their batch controller entrypoints. Fetch the sequence immediately
before building, then prepare the exact returned XDR before signing:

```ts
const network = config.stellarNetwork // 'testnet' | 'mainnet'
const account = await server.getAccount(caller)
const built = buildStellarSupplyTx(
  { network, caller, sourceSequence: account.sequenceNumber() },
  { hubId, asset, amount: '100000000', accountNonce: 0, spokeId },
)
const preparedXdr = await prepareStellarTxXdr(server, built.xdr)
// then sign preparedXdr -> send
```

`prepareStellarTxXdr` adds the simulated Soroban footprint, authorization
entries, and resource fee. If the host application owns a different
`@stellar/stellar-sdk` instance than `@xoxno/sdk-js`, parse the XDR and call
`server.prepareTransaction` with the host instance instead; the SDK classes
must come from the same package instance.

- `amount` values are i128 decimal strings in native asset decimals.
- `accountNonce` is the controller account id (the legacy SDK field name);
  `0` opens a new account.
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

Also: `buildStellarLiquidateTx(opts, { accountNonce, debtPayments })` (see
`building-lending-liquidation-bots`) and
`buildStellarFlashLoanTx(opts, { hubId, asset, amount, receiver, data })`
(receiver side: `writing-flash-loan-receivers`).

## Strategy verbs (leverage and swaps)

`multiply`, `swap_debt`, `swap_collateral`, and
`repay_debt_with_collateral` accept a DEX-aggregator route as opaque bytes.
`multiply` and `swap_debt` borrow inside the atomic strategy; collateral swap
and collateral-funded repayment withdraw existing collateral instead. Obtain
the route from the quote server — do not hand-encode it:

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

`buildStellarSwapDebtTx`, `buildStellarSwapCollateralTx`, and
`buildStellarRepayDebtWithCollateralTx` follow the same quote → map → build
pattern. For a plain user swap without a lending account,
`buildStellarExecuteStrategyTx` targets the aggregator router directly.

### Blend migration

`buildStellarMigrateFromBlendTx(opts, args)` is a separate flow: it takes
`blendPool`, `collateralTokens`, `supplyTokens`, and `debtCaps`, not a swap
route. Confirm that the pool is currently approved before building (for the
indexed preflight: `const approved = (await read.blendPools()).some((p) =>
p.pool === blendPool && p.approved)`). The controller remains authoritative at
execution and rejects an unapproved pool. Migration uses the zero-fee migration
borrow path; each `debtCaps` value must be positive and slightly exceed the
live Blend debt so Blend can refund any excess on-chain.

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
  `SpokeNotFound` (spoke ids start at 1; see fundamentals).
- **Passing a bare array to batch builders** — they take
  `{ accountNonce, assets|borrows|withdrawals|payments: [...] }`.
- **Quoting `debt_to_flash_loan` gross for multiply** — the swapped amount is
  net of the flash-loan fee; a gross quote overstates output and can revert
  on `totalMinOut`.
