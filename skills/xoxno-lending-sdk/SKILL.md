---
name: xoxno-lending-sdk
description: "Use when integrating XOXNO Lending from TypeScript with the public SDK: wallet/web/mobile reads, positions, transaction builders, strategy swaps, preparation, signing, submission, and frontend lifecycle."
user-invocable: true
argument-hint: "[read | positions | transaction | strategy | frontend]"
---

# XOXNO Lending SDK

This skill targets the published `@xoxno/sdk-js@1.0.228` surface and
`@stellar/stellar-sdk@16.3.0`. Builders return unsigned, unprepared XDR. The host
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

## Install for the task

For a positions or asset screen, use plain HTTP or the optional read entry:

```sh
npm install @xoxno/sdk-js@1.0.228
```

```ts
import { createStellarLendingReadClient } from '@xoxno/sdk-js/stellar-lending/read'

const network = 'mainnet' as const
const baseUrl = {
  mainnet: 'https://api.xoxno.com',
  testnet: 'https://testnet-api.xoxno.com',
}[network]
const read = createStellarLendingReadClient({ baseUrl })
```

The read entry exports the public response types and schemas. It has no Stellar
SDK runtime dependency. `read.positions(owner)` collects all pages; `read.assets()`
lists collateral assets. Use `read.assets({ usage: 'borrow' })` for borrow assets.

For transactions and advanced math, also install the Stellar SDK:

```sh
npm install @stellar/stellar-sdk@16.3.0
```

Install one copy so RPC and builders use the same transaction classes.
Use published package entries; do not deep-import `dist/` or source files.
See the [wallet guide](https://xoxno.com/docs/stellar-lending/dev/wallet-integration)
and [API reference](https://xoxno.com/docs/stellar-lending/dev/integrator-api).

## Convert a user amount

```ts
import { parseStellarTokenAmount } from '@xoxno/sdk-js/stellar-lending'

const amount = parseStellarTokenAmount('1.25', 7) // '12500000'
```

Pass the selected token's `decimals` directly. The helper rejects missing
metadata, invalid decimal text, excess precision, non-positive amounts, and
positive i128 overflow. It uses Stellar's string parser; no floating-point
conversion or integrator-owned parser is needed. Use raw `'0'` only for the
withdraw-all sentinel, not as a positive amount to this helper.

## Shared invariants

- Resolve controller, router, governance, and position-NFT addresses from
  `getStellarDeployment(network)`.
- Resolve `(spokeId, hubId, sac)` and token decimals from the selected v1 asset
  and spoke. Pass `sac` as the builder's `asset`. These coordinates are
  independent. Do not infer one from another.
- `sourceSequence` is a Stellar account sequence. `accountNonce` is a lending
  account id / position-NFT token id. Never interchange them.
- Amounts passed to builders are decimal `i128` strings in token base units.
  Keep v1 `amountRaw` as a string; it is already in token base units. Advanced
  math uses `BigInt` and raw RAY/WAD inputs. Display numbers are estimates.
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
4. The original network, signed envelope, and hash survive reloads.
5. Unresolved records remain stored after expiry.
6. The original outcome is established before any replacement submission.
7. Ledger `SUCCESS` is confirmed for the original hash.
8. API caches are refreshed after confirmation.
9. Current NFT ownership is verified before the next owner or delegate action.
