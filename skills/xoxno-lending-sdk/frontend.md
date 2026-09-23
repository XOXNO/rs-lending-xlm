# Frontend integration

Companion to [SKILL.md](SKILL.md). Use the canonical build/prepare/sign/submit
state machine in [transactions.md](transactions.md); do not implement a second
retry policy in the React hook.

## Wallet boundary

Hide browser wallets, WalletConnect, and mobile webviews behind one interface:

```ts
export interface StellarSigner {
  signTransactions(xdrs: string[]): Promise<string[]>
}
```

Pass only RPC-prepared XDR to this interface and use
`STELLAR_NETWORK_PASSPHRASE[network]`. Treat wallet rejection separately from
contract failure: wallet providers commonly throw plain objects, with rejection
codes such as `5000` or `-4`, rather than `Error` instances.

Mobile app switching can exceed 30 seconds. The builder default timebound is
300 seconds (`timeoutSeconds`). Do not rebuild only because the wallet prompt
was slow; read the envelope's actual max timebound.

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

```ts
type Entry = { createdAt: number; promise: Promise<string> }
const prepared = new Map<string, Entry>()
const TTL_MS = 15_000

export function cachedPreparation(
  key: string,
  prepare: () => Promise<string>,
): Promise<string> {
  const hit = prepared.get(key)
  if (hit && Date.now() - hit.createdAt < TTL_MS) return hit.promise
  const promise = prepare()
  prepared.set(key, { createdAt: Date.now(), promise })
  promise.catch(() => {
    if (prepared.get(key)?.promise === promise) prepared.delete(key)
  })
  return promise
}
```

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

Persist the original hash and exact signed envelope before send. On an
uncertain state, query that hash and, if needed, resubmit the unchanged
envelope. A resubmission `ERROR` keeps the state uncertain: the original may
already have applied. Do not build a replacement until the retained envelope's
timebounds expire. The full policy and reference helper are in
[transactions.md#canonical-lifecycle](transactions.md#canonical-lifecycle).

Use one toast/activity record per transaction hash. `DUPLICATE` must not emit a
second success, and send acceptance must not refresh product state as though it
were confirmed.

## Error display

`invokedContractId` tags the top-level invocation only. A controller-tagged
strategy can fail in the pool, aggregator router, or a DEX. Their numeric error
codes overlap.

For nested errors:

1. inspect diagnostic events for the emitting contract id;
2. map only against that contract's namespace;
3. if diagnostics do not identify the emitter, display an unmapped nested
   contract error and retain the raw diagnostic.

Do not call `mapSorobanError` solely because the top-level tag is the lending
controller. Use the single SDK interpretation flow in
[transactions.md#error-interpretation](transactions.md#error-interpretation).

## Live state and reconciliation

Keep one shared `liveState()` query, normally polled every 10 seconds. Fetch
`context()` on page load/navigation and refetch `userPositions(owner)` only
after ledger `SUCCESS`.

After success:

1. refresh live state and positions together;
2. compare the expected account id and legs with the returned/indexed state;
3. tolerate indexer lag without claiming the old state is final;
4. recheck `owner_of(accountId)` before enabling another mutation.

Float fields (`*Short`, `*Usd`, `*Native`, APY percentages, formatted
leverage) are UI estimates. Builder inputs and risk decisions remain raw
base-unit/RAY/WAD `BigInt` values, and successful preparation is still not a
guarantee of ledger admission.

## Spokes and trustlines

The UI may label a spoke as an “e-mode,” but the protocol has no separate
e-mode identifier. Parse values such as `STELLAR:3` into spoke `3`, reject
zero, and populate choices from `context()`; changing spoke means selecting or
creating a different lending account.

Before an action delivers a classic Stellar asset to a wallet, verify the
trustline through Horizon. This includes borrow, withdraw, close-position
withdrawals, and possible refunds. Native XLM and pure Soroban tokens do not
use classic trustlines. Resolve code/issuer from catalog data; a zero-balance
trustline is still present.

Apply the canonical [completion gates](SKILL.md#completion-gates). A frontend
must also pass two checks: the prepared-XDR cache is invalidated immediately
after signing, and the retained envelope and hash survive a page reload.
