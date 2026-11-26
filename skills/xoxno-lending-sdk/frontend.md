# Frontend integration

This file is a companion to [SKILL.md](SKILL.md). Use the state machine in
[transactions.md](transactions.md). Do not add a second retry policy in the
React hook.

## Wallet boundary

Give the wallet the selected signing network. Browser wallets,
WalletConnect, mobile wallets, and webviews can share one callback:

```ts
export interface StellarSigner {
  signTransaction(xdr: string, networkPassphrase: string): Promise<string>
}
```

Pass only RPC-prepared XDR with the matching
`STELLAR_NETWORK_PASSPHRASE[network]`. Check that the signed hash equals the
prepared hash before you send. Keys stay in the wallet.

Wallet rejection is not ledger failure. Providers can throw plain objects,
including rejection codes `5000` or `-4`. Check the provider's error contract.
App switching can be slow. Read the envelope's max timebound (builder default
`timeoutSeconds` 300) rather than rebuilding after a slow prompt.

## React Native runtime

The optional SDK reader needs standards-compatible `URL`, `URLSearchParams`,
`fetch`, and cancellation. Without compatible URL support, install
`react-native-url-polyfill@2.0.0` and put
`import 'react-native-url-polyfill/auto'` first in the entry point, before SDK
and app imports. Plain HTTP reads skip the reader.

Initialize `TextEncoder` and `TextDecoder` before transaction SDK imports. Use
the wallet integration's Stellar SDK and Metro setup. Verify preparation,
signing, submission, and recovery on the target device.

## Render a portfolio

Fetch as in [reads.md](reads.md) and render `supplied` and `borrow` directly.
Keep `accountId` and `amountRaw` as strings. Show missing fields as
"Unavailable", not zero. For units and keys, see
[reads.md](reads.md#units-and-missing-data) and
[positions.md](positions.md#discover-and-select-an-account).

Fall back to an icon for absent or failed logos. Browser HTML images render
`nftImage` SVGs. React Native needs an SVG renderer such as SvgUri from
`react-native-svg`. Native `Image` handles supported raster formats. Mobile
uses the public HTTPS API, and a device's `localhost` is the device itself.

Keep loading, empty, HTTP error, and incomplete-data states distinct. Abort or
discard stale wallet and network requests after selection changes.

## Prepared-XDR prefetch

Prefetch preparation only after the final intent is known. A safe cache has
these properties:

- keys on caller, network, full intent, amount, coordinates, and route bytes
- stores the in-flight promise so concurrent requests share one simulation
- has a short TTL and bounded size
- drops rejected preparations
- clears all the caller's entries as soon as one envelope is signed.

Prepared XDR embeds sequence, timebounds, footprint, authorization, and
resource fee. Cache freshness never proves admissibility. Recheck NFT owner
and coordinates before preparation.

Most apps can prepare after confirmation. Add a shared cache only when
measured latency justifies it.

## UI transaction states

Render states by evidence:

- build, validation, preparation simulation, or wallet rejection:
  deterministic pre-submit failure
- `PENDING` or `DUPLICATE`: submitted, confirming
- first-send `ERROR`: rejected before inclusion, terminal
- `TRY_AGAIN_LATER` or thrown send transport error: uncertain
- polling `NOT_FOUND`, timeout, or thrown polling error: `UNKNOWN`, not
  failed
- `FAILED`: terminal ledger failure
- `SUCCESS`: terminal success

For persistence, resubmission, and replacement, see
[transactions.md#canonical-lifecycle](transactions.md#canonical-lifecycle). A
resubmission `ERROR` or expiry stays uncertain.

Show one toast or activity record per hash. `DUPLICATE` never emits a second success,
and send acceptance never refreshes product state as if confirmed.

## Error display

`invokedContractId` tags only the top-level invocation, and nested codes overlap.
Follow [transactions.md#error-interpretation](transactions.md#error-interpretation).
With an unknown emitter, show the raw error under an unknown namespace.

## Live state and reconciliation

Refresh v1 arrays on wallet or network change, after ledger `SUCCESS`, and
as the screen needs. Share one read query across components.

After success, follow these steps:

1. Refresh the wallet's positions.
2. Refresh affected asset data.
3. Compare returned account id and expected positions with indexed state.
4. Show refreshing while the indexer lags.
5. Keep the action complete. Never repeat it to refresh data.
6. Recheck `owner_of(accountId)` before the next owner-wallet mutation.

Float fields and formatted leverage are display estimates. Decide with raw
base-unit, RAY, and WAD `bigint` values and directed rounding. API capacities, flags, and
successful preparation do not guarantee admission.

## Spokes and trustlines

A UI may call a spoke an "e-mode". The protocol has no e-mode identifier.
Parse `STELLAR:3` into spoke `3`. Reject zero. Populate choices from v1 asset
`spokes`. Another spoke means another lending account.

Before an action delivers a classic Stellar asset to a wallet (borrow,
withdraw, close-position withdrawals, possible refunds), verify the trustline
via Horizon. Native XLM and pure Soroban tokens need none. Resolve
the code and issuer from catalog data. A zero-balance trustline still counts.

Apply the [completion gates](SKILL.md#completion-gates), and invalidate the
prepared-XDR cache immediately after signing.
