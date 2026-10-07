# Frontend integration

Companion to [SKILL.md](SKILL.md). Use the canonical build/prepare/sign/submit
state machine in [transactions.md](transactions.md); do not implement a second
retry policy in the React hook.

## Wallet boundary

Your wallet provider must receive the selected signing network. Browser wallets,
WalletConnect, mobile wallets, and webviews can share a callback shape:

```ts
export interface StellarSigner {
  signTransaction(xdr: string, networkPassphrase: string): Promise<string>
}
```

Pass only RPC-prepared XDR and the matching
`STELLAR_NETWORK_PASSPHRASE[network]`. Verify the signed transaction's hash
matches the prepared transaction before sending. Keep keys in the wallet.

Treat wallet rejection separately from ledger failure. Providers can throw plain
objects, including rejection codes `5000` or `-4`; inspect the provider's actual
error contract. Mobile app switching can be slow. Read the envelope's actual
max timebound rather than rebuilding because a prompt took time. The builder's
default `timeoutSeconds` is 300.

## Render a portfolio

Use plain HTTP or `createStellarLendingReadClient` from the SDK read entry.
`positions(owner)` returns one item per indexed owned NFT; `assets()` returns
collateral asset/spoke choices. Fetch them in parallel if the screen needs both.
No token metadata join or RAY balance conversion is required.

Render `supplied` and `borrow` lists directly. Keep `accountId` and `amountRaw`
as strings. APYs are fractions; `0.05` displays as 5%. Missing fields display as
“Unavailable”, not zero. Display “No debt” only when `hasDebt === false`.
Use `(network, nftContract, accountId)` as a position key and `(hubId, sac)` as a
leg key. Select accounts explicitly before actions.

Use a fallback icon for absent or failed logos. Browser HTML images can render
`nftImage` SVGs. React Native needs an SVG renderer, such as SvgUri from
`react-native-svg`; native `Image` is suitable for supported raster formats.
Use the public HTTPS API on mobile. A physical device's `localhost` is the
device, not the development computer.

Keep loading, empty, HTTP error, and incomplete-data states distinct. Abort or
discard an old wallet/network request after selection changes. See
[reads.md](reads.md) for nullable fields, pagination, and cache semantics.

## Prepared-XDR prefetch

Preparation can be prefetched when the final intent is known. A safe cache:

- keys by caller, network, complete intent, amount, coordinates, and route
  bytes;
- stores the in-flight promise so concurrent requests share one simulation;
- has a short TTL and bounded entry count;
- removes rejected preparations;
- invalidates all entries for the caller immediately after one prepared
  envelope is signed.

Sequence, timebounds, footprint, authorization, and resource fee are embedded
in prepared XDR. Never use cache freshness as proof that an envelope remains
admissible. Recheck the current position-NFT owner and account/reserve
coordinates before preparation.

Most applications can prepare after the user confirms the action. Add a shared
cache only when measured preparation latency justifies it; keep the policy above
if one is used. No cache removes the need to check sequence and timebounds.

## UI transaction states

Render lifecycle states by evidence:

- local build/validation, preparation simulation, or wallet rejection:
  deterministic pre-submit failure;
- `PENDING` / `DUPLICATE`: submitted, confirming;
- first-send `ERROR`: RPC rejected the envelope before inclusion; terminal;
- `TRY_AGAIN_LATER` or a thrown send transport error: network uncertain;
- polling `NOT_FOUND`, timeout, or a thrown polling transport error:
  `UNKNOWN`, not failed;
- `FAILED`: terminal ledger failure;
- `SUCCESS`: terminal product success.

Persist the original network, hash, and exact signed envelope durably before send. On an
uncertain state, query that hash and, if needed, resubmit the unchanged
envelope. A resubmission `ERROR` keeps the state uncertain: the original may
already have applied. Expiry alone does not prove failure. Reconcile the original
network's retained history and account state before deciding on a replacement;
keep an unproven outcome unresolved. The full policy and reference helper are in
[transactions.md#canonical-lifecycle](transactions.md#canonical-lifecycle).

Use one toast/activity record per transaction hash. `DUPLICATE` must not emit a
second success, and send acceptance must not refresh product state as though it
were confirmed.

## Error display

`invokedContractId` tags the top-level invocation only. A controller-tagged
strategy can fail in the pool, aggregator router, or a DEX. Their numeric error
codes overlap.

For nested errors:

1. Read the error and contract id from the same diagnostic entry.
2. Trace propagated errors and handled nested failures.
3. Map the error only when the emitting contract is established.
4. If the emitter is unknown, display the raw error with an unknown namespace.

Do not call `mapSorobanError` solely because the top-level tag is the lending
controller. Use the single SDK interpretation flow in
[transactions.md#error-interpretation](transactions.md#error-interpretation).

## Live state and reconciliation

Refresh v1 arrays when the wallet or network changes, after ledger `SUCCESS`,
and as the screen needs fresher data. Use one shared read query when several components need the same data.

After success:

1. Refresh the selected wallet's positions.
2. Refresh the affected asset data.
3. Compare the returned account id and expected positions with indexed state.
4. Keep a refreshing state while the indexer lags.
5. Keep the confirmed action complete; do not repeat it to refresh data.
6. Recheck `owner_of(accountId)` before the next owner-wallet mutation.

Float fields and formatted leverage are display estimates. For decisions, use
raw base-unit/RAY/WAD `bigint` inputs with directed rounding. API capacities,
flags, and successful preparation do not guarantee ledger admission.

## Spokes and trustlines

The UI may label a spoke as an “e-mode,” but the protocol has no separate
e-mode identifier. Parse values such as `STELLAR:3` into spoke `3`, reject
zero. Populate choices from v1 asset `spokes`. To use another spoke, select or
create a different lending account.

Before an action delivers a classic Stellar asset to a wallet, verify the
trustline through Horizon. This includes borrow, withdraw, close-position
withdrawals, and possible refunds. Native XLM and pure Soroban tokens do not
use classic trustlines. Resolve code/issuer from catalog data; a zero-balance
trustline is still present.

Apply the canonical [completion gates](SKILL.md#completion-gates). A frontend
must invalidate its prepared-XDR cache immediately after signing.
It must retain the original network, envelope, and hash through reloads and
mobile app switching.
