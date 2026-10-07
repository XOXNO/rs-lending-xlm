---
name: xoxno-swap-aggregator
description: "Use when swapping tokens on Stellar through the XOXNO swap aggregator (arb-algo, stellar-swap.xoxno.com, GET /api/v1/quote), reading a quote's output, minimum received, price impact, route or fee, executing execute_strategy on the router, or obtaining a routeXdr to embed in a lending strategy (multiply, swap collateral, swap debt, repay debt with collateral) or in your own Soroban contract."
user-invocable: true
argument-hint: "[swap or route task]"
---

# XOXNO Swap Aggregator

The aggregator has a quote server and a router contract. The server searches
Soroban DEX liquidity and returns a route. The router executes the route
atomically through `execute_strategy(sender, total_in, swap_xdr)`.
Standalone swaps and lending strategies use the same route format.

Sources: `arb-algo` commit `54de209a` (verified source), `@xoxno/sdk-js` 1.0.228 with
`@stellar/stellar-sdk` ^16, `rs-lending-xlm` at this skill's commit.

## Related skills
- Shared model, addresses, hub/spoke ids → [../xoxno-lending/SKILL.md](../xoxno-lending/SKILL.md)
- Strategy transaction builders in TypeScript → [../xoxno-lending-sdk/SKILL.md](../xoxno-lending-sdk/SKILL.md)
- Calling the controller or the router from a Soroban contract → [../xoxno-lending-contracts/SKILL.md](../xoxno-lending-contracts/SKILL.md)
- Decoding a failed simulation → [../xoxno-lending-troubleshooting/SKILL.md](../xoxno-lending-troubleshooting/SKILL.md)
- Signing, submission, and recovery → [SDK transactions](../xoxno-lending-sdk/transactions.md)

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

The quote server has no submission endpoint. A request with `sender` can
return an unsigned envelope. Before quoting, fetch `/api/v1/config`.
Compare its network passphrase and router with trusted configuration.
Read the router from `configs/networks.json` or `STELLAR_NETWORKS`.
Use the server response only to check the deployment.

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

export async function quoteXlmToUsdc(usdcContractId: string, xlmBaseUnits: bigint) {
  const tokens = await getStellarQuoteTokens({ baseUrl: NET.quoteUrl })
  const xlm = Asset.native().contractId(NET.passphrase)
  for (const id of [xlm, usdcContractId]) {
    const entry = tokens.find((t) => t.id === id)
    if (!entry || entry.lp) throw new Error(`${id} is not a routable token`)
  }

  const quote = await getStellarAggregatorQuote(
    { from: xlm, to: usdcContractId, amountIn: xlmBaseUnits.toString(), slippage: 0.005, includePaths: true },
    { baseUrl: NET.quoteUrl },
  )
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

## Example 2: prepare a standalone swap

Build the transaction locally. Take the pair, input, minimum output, caller,
and router from the application's request and trusted configuration.

```ts
import { rpc } from '@stellar/stellar-sdk'
import {
  STELLAR_NETWORKS,
  assertStellarPreparedTxAuth,
  buildStellarExecuteStrategyTx,
  getStellarAggregatorQuote,
  mapQuoteResponseToStrategySwap,
  prepareStellarBuiltTx,
} from '@xoxno/sdk-js/stellar-lending'

const NET = STELLAR_NETWORKS.stellarMainnet
const server = new rpc.Server(NET.sorobanRpcUrl)

export async function prepareStandaloneSwap(
  caller: string,
  trustedRouter: string,
  tokenIn: string,
  tokenOut: string,
  amountIn: string,
  acceptedMinOut: string,
) {
  const response = await fetch(new URL('/api/v1/config', NET.quoteUrl))
  if (!response.ok) throw new Error('deployment lookup failed')
  const config = await response.json()
  if (config.networkPassphrase !== NET.passphrase || config.router !== trustedRouter) {
    throw new Error('quote deployment mismatch')
  }
  const quote = await getStellarAggregatorQuote(
    { from: tokenIn, to: tokenOut, amountIn, slippage: 0.005, fresh: true },
    { baseUrl: NET.quoteUrl },
  )
  if (!quote.routeXdr || !quote.amountOutMin || quote.from !== tokenIn ||
      quote.to !== tokenOut || quote.amountIn !== amountIn) {
    throw new Error('quote mismatch or missing executable route')
  }
  const steps = mapQuoteResponseToStrategySwap(quote, {
    expected: { tokenIn, tokenOut, amountIn, minOut: acceptedMinOut },
  })
  const account = await server.getAccount(caller)
  const built = buildStellarExecuteStrategyTx({
    network: 'mainnet', caller, sourceSequence: account.sequenceNumber(),
    routerAddress: trustedRouter, totalIn: amountIn,
  }, steps)
  const preparedXdr = await prepareStellarBuiltTx(server, built, {
    network: 'mainnet', invokedContractId: trustedRouter,
  })
  assertStellarPreparedTxAuth(preparedXdr, {
    network: 'mainnet', caller, builtXdr: built.xdr,
    deployment: { aggregatorRouter: trustedRouter },
  })
  return { preparedXdr, quote }
}
```

`acceptedMinOut` is the output floor accepted by the user. Do not replace it
with a response value. Sign only the checked `preparedXdr`. Follow
[SDK transactions](../xoxno-lending-sdk/transactions.md) for signing,
persistence, submission, confirmation, and recovery. On success, read the
actual delivered return value and reconcile token balances.

## LP conversion: `transactions[]`

A `convertLiquidity` quote with `sender` returns ordered `transactions[]` instead of
`transaction`; confirm each on-chain before preparing the next
([api.md#response-quoteresponse](api.md#response-quoteresponse)).

## Completion checks

1. Compare `/api/v1/config` with trusted network and router configuration.
2. Fetch a current quote.
3. Check the encoded route against caller-owned pair, input, and output floor.
4. Build the transaction locally with the current source sequence.
5. Prepare the complete transaction through the selected network's RPC.
6. Check authorization against the built invocation and expected caller.
7. Sign the checked prepared XDR.
8. Persist the original network, signed XDR, and hash before sending.
9. Confirm the original hash and reconcile delivered tokens.

Keep an unresolved transaction record after expiry. Do not submit a replacement
until the original outcome is resolved. Follow the
[submission lifecycle](../xoxno-lending-troubleshooting/SKILL.md#submission-lifecycle).
