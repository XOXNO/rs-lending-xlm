---
name: xoxno-lending-sdk
description: Use when integrating XOXNO Lending from TypeScript with @xoxno/sdk-js/stellar-lending: reads, positions, transaction builders, strategy swaps, preparation, signing, submission, and frontend lifecycle.
user-invocable: true
argument-hint: "[read | positions | transaction | strategy | frontend]"
---

# XOXNO Lending SDK

This skill targets the published `@xoxno/sdk-js@1.0.214` surface and
`@stellar/stellar-sdk@16.0.0`. Builders return unsigned, unprepared XDR. The host
application owns RPC preparation, signing, submission, confirmation, and
state reconciliation.

## Route by task

- API reads, DTO semantics, polling, and the one view-simulation helper:
  [reads.md](reads.md)
- Account discovery, ownership, rounding, and application-side risk math:
  [positions.md](positions.md)
- Builder/ABI mapping and the canonical transaction lifecycle:
  [transactions.md](transactions.md)
- `multiply`, debt/collateral swaps, repay-with-collateral, and quote bytes:
  [strategies.md](strategies.md)
- Wallet UX, prepared-XDR caching, trustlines, and reconciliation:
  [frontend.md](frontend.md)

Canonical sources used by every page:

- Units and formulas: [../xoxno-lending/math.md](../xoxno-lending/math.md)
- Identifiers and deployed addresses:
  [../xoxno-lending/addresses.md](../xoxno-lending/addresses.md)
- ABI and contract-side composition:
  [../xoxno-lending-contracts/abi.md](../xoxno-lending-contracts/abi.md)
- Error namespaces and diagnosis:
  [../xoxno-lending-troubleshooting/SKILL.md](../xoxno-lending-troubleshooting/SKILL.md)
- REST endpoint inventory:
  [../xoxno-lending-data/api.md](../xoxno-lending-data/api.md)

## Install and initialize

```sh
npm install @xoxno/sdk-js@1.0.214 @stellar/stellar-sdk@16.0.0
```

Install one copy of `@stellar/stellar-sdk`; duplicate package realms can make
`prepareTransaction` reject a structurally valid transaction at its
`instanceof` boundary.

```ts
import { rpc } from '@stellar/stellar-sdk'
import {
  XOXNOClient,
  getStellarDeployment,
  stellarLendingRead,
} from '@xoxno/sdk-js/stellar-lending'

const network = 'mainnet' as const
const deployment = getStellarDeployment(network)
const server = new rpc.Server(deployment.sorobanRpcUrl)
const read = stellarLendingRead(
  new XOXNOClient({ apiUrl: 'https://api.xoxno.com' }),
)
```

Use only the published `@xoxno/sdk-js/stellar-lending` subpath or package root.
Do not deep-import `dist/` or unpublished source modules.

## Shared invariants

- Resolve controller, router, governance, and position-NFT addresses from
  `getStellarDeployment(network)`.
- Resolve `(spokeId, hubId, asset)` and token decimals from the selected API
  reserve. These coordinates are independent; none can be inferred from
  another.
- `sourceSequence` is a Stellar account sequence. `accountNonce` is a lending
  account id / position-NFT token id. Never interchange them.
- Amounts passed to builders are decimal `i128` strings in token base units.
  Use `BigInt` and the raw RAY/WAD strings for decisions; floating-point fields
  are display values only.
- An account id does not encode a spoke. A wallet may own multiple accounts on
  the same spoke. Select an account explicitly, then verify its current spoke
  and position-NFT owner.
- A position-NFT transfer transfers control of the lending account. Recheck
  ownership before preparing any mutation.
- Full supply withdrawal uses `amount: '0'`. Full debt repayment sends a
  ceiled, buffered amount, and the pool refunds the excess. There is no
  repay-all sentinel.

## Completion gates

Before calling an integration complete:

1. Every documented import compiles against the pinned published package.
2. The selected account, spoke, hub, asset, and network coordinates match.
3. Only RPC-prepared XDR reaches the signer.
4. The original signed envelope and hash are retained until its timebounds
   expire or it reaches a terminal ledger status.
5. `SUCCESS`, not send acceptance or a polling timeout, is confirmed.
6. API and live-state caches are reconciled after confirmation.
7. The position-NFT owner is rechecked before the next account mutation.
