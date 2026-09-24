---
name: xoxno-swap-aggregator
description: Use when swapping tokens on Stellar through the XOXNO swap aggregator (arb-algo, stellar-swap.xoxno.com, GET /api/v1/quote), reading a quote's output, minimum received, price impact, route or fee, executing execute_strategy on the router, or obtaining a routeXdr to embed in a lending strategy (multiply, swap collateral, swap debt, repay debt with collateral) or in your own Soroban contract.
user-invocable: true
argument-hint: "[swap or route task]"
---

# XOXNO Swap Aggregator

The XOXNO swap aggregator is two pieces: an off-chain quote server (`arb-algo`,
`stellar-indexer`) that searches Soroban DEX liquidity and returns a route, and an
on-chain router contract (`rs-lending-xlm/contracts/swap-aggregator`) that executes that
route atomically in one `execute_strategy(sender, total_in, swap_xdr)` call. The same
`routeXdr` bytes serve a standalone swap and the lending controller's strategy verbs.

Sources: `arb-algo` commit `f2d5fe9` (2026-09-13), `@xoxno/sdk-js` 1.0.214 with
`@stellar/stellar-sdk` ^16, `rs-lending-xlm` at this skill's commit.

## Related skills
- Shared model, addresses, hub/spoke ids → [../xoxno-lending/SKILL.md](../xoxno-lending/SKILL.md)
- Strategy transaction builders in TypeScript → [../xoxno-lending-sdk/SKILL.md](../xoxno-lending-sdk/SKILL.md)
- Calling the controller or the router from a Soroban contract → [../xoxno-lending-contracts/SKILL.md](../xoxno-lending-contracts/SKILL.md)
- Decoding a failed simulation → [../xoxno-lending-troubleshooting/SKILL.md](../xoxno-lending-troubleshooting/SKILL.md)
- Generic wallet signing and submission → the `stellar-dev` `dapp` skill

## Read the file that matches the task

| Task | File |
|------|------|
| Every route, query parameter alias, response field and error body | [api.md](api.md) |
| `StrategyPayload` wire format, packed program, `execute_strategy` semantics, fees, referrals, custody and auth | [payload.md](payload.md) |
| Standalone vs composed decision, `amountIn` sizing per lending verb, same-token rule, own-contract usage, re-simulation, verifying `min_out` | [composition.md](composition.md) |
| Quote, render, execute a standalone swap | below |

## Routing boundary

The on-chain venue/opcode mapping is centralized in
[payload.md#packed-program-ops--version-1](payload.md#packed-program-ops--version-1).
The external quote server's venue discovery and production compatibility remain outside
the contract proof boundary.

## Base URLs and the deployment check

| Network | Quote server | Source |
|---|---|---|
| mainnet | `https://stellar-swap.xoxno.com` | `STELLAR_NETWORKS.stellarMainnet.quoteUrl` |
| testnet | `https://testnet-stellar-swap.xoxno.com` | `STELLAR_NETWORKS.stellarTestnet.quoteUrl` |

There is no submit endpoint; the quote builds the envelope when `sender` is present.
Before the first quote, `GET /api/v1/config` and compare `networkPassphrase` and `router`
with `configs/networks.json` (`aggregator`, listed in
[../xoxno-lending/addresses.md](../xoxno-lending/addresses.md)). Never paste a `C…`
router address into code. Read it from trusted configuration (`configs/networks.json`
`aggregator` or the SDK's `STELLAR_NETWORKS[*].aggregatorRouter`). Use
`/api/v1/config.router` only as a cross-check.

## Token identifiers

`from` and `to` are 56-character `C…` contract ids only; XLM is
`Asset.native().contractId(passphrase)`. Catalog and entry shape:
[api.md#get-apiv1tokens](api.md#get-apiv1tokens).

## Quote parameters

Parameters: [api.md#get-apiv1quote](api.md#get-apiv1quote); exactly one amount,
`slippage` required for `routeXdr`. Response fields:
[api.md#response-quoteresponse](api.md#response-quoteresponse). Never render `routeXdr`
or `transaction`; they are execution inputs. Error codes and reactions:
[api.md#error-bodies-errorresponse-code-error](api.md#error-bodies-errorresponse-code-error).

Freshness and slippage rules are centralized in
[api.md#freshness-and-slippage-canonical](api.md#freshness-and-slippage-canonical).

## Example 1: quote and render

```ts
import { Asset } from '@stellar/stellar-sdk'
import {
  STELLAR_NETWORKS,
  getStellarAggregatorQuote,
  getStellarQuoteTokens,
} from '@xoxno/sdk-js/stellar-lending'

const NET = STELLAR_NETWORKS.stellarMainnet

/** Shape actually served by GET /api/v1/tokens (the SDK's StellarQuoteToken is stale). */
type QuoteToken = { id: string; decimals: number; lp: boolean; dexes: string[] }
/** Wire fields the SDK 1.0.214 response DTO does not declare yet. */
type QuoteExtras = {
  snapshot?: { ledger: number; ageSeconds: number; checkedLiveLedger?: number }
  degraded?: { requestedMaxSplits: number; effectiveMaxSplits: number; requestedMaxHops: number; effectiveMaxHops: number; fallbackAttempts: number; reason: string }
}

export async function quoteXlmToUsdc(usdcContractId: string, xlmBaseUnits: bigint) {
  const tokens = (await getStellarQuoteTokens({ baseUrl: NET.quoteUrl })) as unknown as QuoteToken[]
  const xlm = Asset.native().contractId(NET.passphrase)
  for (const id of [xlm, usdcContractId]) {
    const entry = tokens.find((t) => t.id === id)
    if (!entry || entry.lp) throw new Error(`${id} is not a routable token`)
  }

  const raw = await getStellarAggregatorQuote(
    { from: xlm, to: usdcContractId, amountIn: xlmBaseUnits.toString(), slippage: 0.005, includePaths: true },
    { baseUrl: NET.quoteUrl },
  )
  const quote = raw as typeof raw & QuoteExtras
  if (!quote.amountOutMin) throw new Error('amountOutMin requires slippage')

  return {
    youPay: `${quote.amountInShort} (${quote.amountIn} base units)`,
    expected: `${quote.amountOutShort} (${quote.amountOut})`,
    minimumReceived: `${quote.amountOutMinShort} (${quote.amountOutMin})`,
    priceImpactPct: quote.priceImpact == null ? 'n/a' : (quote.priceImpact * 100).toFixed(3),
    rate: quote.rate,
    route: quote.hops.map((h) => `${h.dex}:${h.address.slice(0, 6)} ${h.from.slice(0, 6)}→${h.to.slice(0, 6)}`),
    splits: quote.paths?.map((p) => `${(p.splitPpm / 10_000).toFixed(2)}% via ${p.swaps.length} hop(s)`) ?? [],
    feeBps: quote.feeBps ?? 0,
    feeOn: quote.feeOnInput === undefined ? 'none' : quote.feeOnInput ? 'input' : 'output',
    degraded: quote.degraded ? `routed with ${quote.degraded.effectiveMaxSplits} split(s)` : null,
    snapshotLedger: quote.snapshot?.ledger,
  }
}
```

## Example 2: standalone swap, sign and submit

Calls `router.execute_strategy(sender, total_in = quote.amountIn, routeXdr)`. The sender's
signature is the only authorization
([payload.md#authorization-model](payload.md#authorization-model)). Copy
`verifyRoutePayload` from
[payload.md#verify-routexdr-before-signing](payload.md#verify-routexdr-before-signing)
into `verify-route-payload.ts`.

```ts
import { Keypair, TransactionBuilder, rpc, scValToNative, xdr } from '@stellar/stellar-sdk'
import {
  STELLAR_NETWORKS,
  getStellarAggregatorQuote,
  prepareStellarBuiltTx,
} from '@xoxno/sdk-js/stellar-lending'
import { verifyRoutePayload } from './verify-route-payload.js'

const NET = STELLAR_NETWORKS.stellarMainnet
const server = new rpc.Server(NET.sorobanRpcUrl)

type SwapOutcome =
  | { status: 'SUCCESS'; hash: string; quoted: string; minimum: string; received: string }
  | { status: 'PENDING'; hash: string } // still in flight after polling; look the hash up later

export async function swapStandalone(
  kp: Keypair,
  expectedRouter: string,
  from: string,
  to: string,
  amountIn: string,
): Promise<SwapOutcome> {
  const config = (await (await fetch(new URL('/api/v1/config', NET.quoteUrl))).json()) as {
    networkPassphrase: string; router: string | null
  }
  if (config.networkPassphrase !== NET.passphrase || config.router !== expectedRouter) {
    throw new Error('quote server deployment does not match trusted configuration')
  }

  // Let the server build the envelope. sender requires slippage.
  const quote = await getStellarAggregatorQuote(
    { from, to, amountIn, slippage: 0.005, sender: kp.publicKey() },
    { baseUrl: NET.quoteUrl },
  )
  if (!quote.transaction || !quote.routeXdr || !quote.amountOutMin) throw new Error('expected transaction + routeXdr')
  verifyRoutePayload(
    {
      from: quote.from,
      to: quote.to,
      amountIn: quote.amountIn,
      amountOutMin: quote.amountOutMin,
      routeXdr: quote.routeXdr,
      transaction: {
        envelopeXdr: quote.transaction.envelopeXdr,
        simulated: quote.transaction.simulated,
      },
    },
    {
      router: expectedRouter,
      signer: kp.publicKey(),
      tokenIn: from,
      tokenOut: to,
      totalIn: amountIn,
    },
  )

  const account = await server.getAccount(kp.publicKey())
  const envelope = xdr.TransactionEnvelope.fromXDR(quote.transaction.envelopeXdr, 'base64')
  // The server emits seqNum 0 as a placeholder: set current + 1 on the raw envelope.
  envelope.v1().tx().seqNum(xdr.Int64.fromString((BigInt(account.sequenceNumber()) + 1n).toString()))
  const unsigned = TransactionBuilder.fromXDR(envelope.toXDR('base64'), NET.passphrase)

  // Simulate at the ledger you sign at (also required when transaction.simulated === false).
  const preparedXdr = await prepareStellarBuiltTx(server, { xdr: unsigned.toXDR() },
    { network: 'mainnet', invokedContractId: expectedRouter })
  const tx = TransactionBuilder.fromXDR(preparedXdr, NET.passphrase)
  tx.sign(kp)

  const sent = await server.sendTransaction(tx)
  // PENDING = accepted. DUPLICATE = already submitted; poll the hash. TRY_AGAIN_LATER = resubmit after a delay. ERROR = rejected.
  if (sent.status === 'ERROR') throw new Error(`send rejected: ${sent.errorResult?.toXDR('base64') ?? ''}`)
  if (sent.status === 'TRY_AGAIN_LATER') throw new Error('rpc busy; resubmit the same signed tx later')

  // NOT_FOUND after polling means not yet included, not an on-chain failure. Only FAILED is a failure.
  const result = await server.pollTransaction(sent.hash, { attempts: 20 })
  if (result.status === rpc.Api.GetTransactionStatus.FAILED) throw new Error(`failed on-chain: ${sent.hash}`)
  if (result.status === rpc.Api.GetTransactionStatus.NOT_FOUND) return { status: 'PENDING', hash: sent.hash }

  // execute_strategy returns the i128 actually delivered after fees: this is what the user got.
  const received = scValToNative(result.returnValue as xdr.ScVal) as bigint
  return { status: 'SUCCESS', hash: sent.hash, quoted: quote.amountOut, minimum: quote.amountOutMin, received: received.toString() }
}
```

Building locally from `routeXdr` instead (`buildStellarExecuteStrategyTx`) is in the
encoder table of [payload.md](payload.md#sdk-encoder-surface-xoxnosdk-js-10214-srcsdkstellar).

## LP conversion: `transactions[]`

A `convertLiquidity` quote with `sender` returns ordered `transactions[]` instead of
`transaction`; confirm each on-chain before preparing the next
([api.md#response-quoteresponse](api.md#response-quoteresponse)).

## Completion checks

Before signing: match `/api/v1/config` to trusted network/router configuration; request
a fresh quote; call `verifyRoutePayload` with the originally requested pair, total input,
and expected wallet; replace the external sequence placeholder; prepare/simulate at the
current ledger; reject a delivered result below `amountOutMin`.

Before submitting: confirm no quote input, route byte, signer, network, or router changed
after verification; sign the prepared XDR only; then submit once and poll its hash.
`PENDING`, `DUPLICATE`, and polling `NOT_FOUND` are not execution failures. HTTP and
envelope-placeholder details are external pinned behavior documented in [api.md](api.md).
Submission statuses are in
[../xoxno-lending-troubleshooting/SKILL.md#submission-lifecycle](../xoxno-lending-troubleshooting/SKILL.md#submission-lifecycle).
