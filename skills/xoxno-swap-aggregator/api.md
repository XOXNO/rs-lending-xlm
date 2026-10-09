# Quote Server HTTP Reference

Every route of the XOXNO Stellar swap quote server (`arb-algo/stellar-indexer`), with parameters, response fields, units, and error bodies. The payload a quote carries is in [payload.md](payload.md); using it inside a lending transaction is in [composition.md](composition.md); base URLs and the deployment check are in [SKILL.md](SKILL.md#base-urls-and-the-deployment-check).

Sources: `arb-algo` commit `54de209a` (verified source), `stellar-indexer/src/server/{http,pipeline,discovery,bootstrap,liquidity}.rs`, `src/quote/{types,attach,fee}.rs`, `src/transaction/builder/{swap,envelope}.rs`; `@xoxno/sdk-js` 1.0.228 `STELLAR_NETWORKS[*].quoteUrl`.

**Evidence boundary:** all HTTP routes, status bodies, retry behavior, exact-out handling,
and envelope placeholders below are external behavior pinned to those revisions. They
are not enforced by this repository's contracts or local tests. Re-pin and re-check the
server/SDK before treating them as current production behavior.

## Conventions

| Rule | Detail |
|---|---|
| Method | Every route is `GET`; no authentication; permissive CORS (`Any` origin, method, header) |
| Amounts | Atomic base-unit integers as decimal **strings** (`u128` on the wire, `1..=i128::MAX` accepted); `*Short` fields are `f64` whole-token display values, never transaction arithmetic |
| Token ids | 56-char `C…` Soroban contract strkeys only. `AssetKey::parse_id` rejects `XLM`, `CODE:GISSUER`, and `G…` with `400 invalid_request`. Resolve `from`/`to` from `GET /api/v1/tokens`; XLM is listed under its Stellar Asset Contract id, and lending market ids are in [../xoxno-lending/addresses.md](../xoxno-lending/addresses.md) |
| Naming | `/health` and `/ready` bodies are **snake_case** (`last_applied_ledger`, `seconds_since_last_apply`). Every `/api/v1/*` body is **camelCase** (`#[serde(rename_all = "camelCase")]`) with one exception: `/api/v1/prices` entries carry `depth_usd` (`PriceEntry` has no rename) |
| Query names | `/api/v1/quote` canonical names are snake_case; camelCase aliases exist for `amount_in`, `amount_out`, `max_hops`, `max_splits`, `include_paths`, `referral_id` (plus `referral`). `from`, `to`, `slippage`, `sender`, `simulate`, `platform`, `fresh` have one spelling. Unknown query keys (for example `router`) are ignored, not rejected (`QuoteParams` has no `deny_unknown_fields`) |
| Limits | 30 s request timeout, 1 MiB request-body limit, `x-request-id` header propagated or generated (`bootstrap.rs`). A timed-out request gets `408` with an empty body (tower-http 0.5 `TimeoutLayer`) |
| Errors | JSON `{ "code": string, "error": string }` (`ErrorResponse` on `/api/v1/quote`, `DiscoveryError` on `/api/v1/referrals/{id}`). Branch on `code` (stable); `error` is free text and may change |
| OpenAPI | `GET /api-docs/openapi.json`; Swagger UI at `/`. Regenerate offline with `DUMP_OPENAPI=1 RUSTC_WRAPPER= cargo run -q -p stellar-indexer --features server --bin quote_server > openapi.json` (`arb-algo/scripts/sync_stellar_openapi.py` docstring): `server::run()` prints `ApiDoc::openapi().to_pretty_json()` to stdout and exits before any network bootstrap (`bootstrap.rs`) |

## Routes

| Method | Path | Purpose | Status codes |
|---|---|---|---|
| GET | `/health` | Liveness + snapshot stats | 200 |
| GET | `/ready` | Snapshot freshness gate | 200, 503 |
| GET | `/api/v1/config` | Deployment identity, router, limits, venues | 200 |
| GET | `/api/v1/tokens` | Routable token catalog | 200 |
| GET | `/api/v1/prices` | USD price per token id | 200 |
| GET | `/api/v1/pools` | Indexed pool catalog | 200 |
| GET | `/api/v1/referrals/{id}` | Referral fee lookup | 200, 400, 502, 503 |
| GET | `/api/v1/quote` | Forward / reverse / LP quote, optional envelope | 200, 400, 404, 409, 422, 500, 502, 503 |

### GET /health

Process liveness. A 200 does not mean quotes are fresh; use `/ready` for that.

| Field | Type | Meaning |
|---|---|---|
| `status` | string | Always `"healthy"` when the process answers |
| `pools` | integer | Indexed swap edges (a multi-token pool contributes several) |
| `tokens` | integer | Token ids in the snapshot interner |
| `decimals_entries` | integer | Tokens with a resolved decimal precision |
| `last_applied_ledger` | integer (u32) | Ledger the snapshot is applied through |
| `last_applied_cursor` | string | Opaque event-stream resume cursor |
| `seconds_since_last_apply` | integer (u64) | Seconds since the snapshot last advanced |
| `static_fee_bps` | integer (u32) | Cached router admin fee (bps), before referral fees |
| `usd_prices_cached` | integer | Entries in the oracle price cache |

### GET /ready

Return status is `200` only when all readiness conditions hold:

1. The serving epoch is active.
2. Coverage age is at most 30 seconds.
3. Observed ledger drift is at most 2 ledgers.

Otherwise, the server returns `503`. Poll `/ready` for readiness.
Use `/health` for process liveness. Preserve the readiness body for diagnosis.

| Field | Type | Meaning |
| --- | --- | --- |
| `ready` | boolean | All readiness conditions hold |
| `seconds_since_last_apply` | u64 | Coverage age; maximum u64 when unknown |
| `staleness_threshold_secs` | u64 | `30` |
| `last_applied_ledger` | u32 | Ledger applied through the snapshot |
| `pools` | integer | Indexed swap edges |

### GET /api/v1/config

| Field | Type | Meaning |
|---|---|---|
| `network` | `"mainnet"` \| `"testnet"` | Deployment network |
| `networkPassphrase` | string | Signing domain every returned envelope is bound to |
| `router` | string \| null | Router contract `C…` the deployment executes through. Resolved from `AGGREGATOR_ROUTER`, then the seed file, then the built-in network default; `null` only when none resolves. Must equal the `aggregator` entry of `configs/networks.json` for the same network |
| `apiVersion` | string | `"v1"` |
| `protocolVersion` | integer (u8) | Packed program version the router expects: `1` (`PROGRAM_VERSION`, see [payload.md](payload.md)) |
| `defaultMaxHops` | integer | `4` (`SplitConfig::default().max_hops`) |
| `defaultMaxSplits` | integer | `4` (`SplitConfig::default().max_splits`) |
| `maxHops` | integer | `6` (`MAX_REQUEST_HOPS`) |
| `maxSplits` | integer | `8` (`MAX_REQUEST_SPLITS`) |
| `operations` | string[] | `["swap","addLiquidity","removeLiquidity","convertLiquidity"]` |
| `venues` | string[] | DEX labels present in this snapshot, sorted (`BTreeSet`); not a route promise |
| `lastAppliedLedger` | integer (u32) | Snapshot ledger |

### GET /api/v1/tokens

Array of `TokenEntry`, sorted by pool count descending, then `id` ascending. Resolve `from`, `to`, and `decimals` here before quoting.

| Field | Type | Meaning |
|---|---|---|
| `id` | string | Canonical Soroban token contract `C…` — the value to pass as `from`/`to` |
| `decimals` | integer (u8) | Divide atomic amounts by `10^decimals` for display |
| `lp` | boolean | `true` for a pool share token; quoting from/to it is an add/remove-liquidity route |
| `dexes` | string[] | DEX labels the token appears on, sorted; for an LP token, the issuing DEX |
| `pool` | string | LP only: pool contract that issued the shares (omitted otherwise) |
| `assets` | string[] | LP only: constituent token ids in pool order (omitted otherwise) |

Use the published `StellarQuoteToken` type from SDK `1.0.228`.

### GET /api/v1/prices

`Record<tokenId, PriceEntry>`. A token is absent, not zero, when no usable pool path of 4 hops or fewer (`PRICE_MAX_HOPS`) reaches it from an oracle-priced token. An LP share is priced from its constituents instead, and is absent unless all of them have a price.

| Field | Type | Meaning |
|---|---|---|
| `usd` | number | USD per whole token |
| `source` | `"oracle"` \| `"pool"` \| `"lp"` | Reflector price or the network USD anchor (USDC on mainnet) fixed at `1.0`, depth-weighted pool average, or LP share valued from its pool |
| `depth_usd` | number | Summed depth of the pools that produced the price, or the LP pool's TVL; absent for oracle prices. Threshold on it to drop prices implied by untradeable pools |
| `hops` | integer (u8) | Pool hops from the nearest oracle-priced token; `0` = oracle |

### GET /api/v1/pools

| Field | Type | Meaning |
|---|---|---|
| `lastAppliedLedger` | integer (u32) | Snapshot ledger |
| `pools[].id` | string | Pool contract; multi-token edges grouped into one entry |
| `pools[].dex` | string | Venue label |
| `pools[].assets` | string[] | Constituent token ids, sorted, deduplicated |
| `pools[].feeBps` | integer (u32) | Pool swap fee (bps); excludes aggregator/referral fees |
| `pools[].shareToken` | string \| null | Non-null only when the snapshot supports this pool's liquidity position |

### GET /api/v1/referrals/{id}

Path `id`: unsigned u32 decimal; `0` = no referral. Served from the quote cache (`REFERRAL_TTL` = 60 s); a cache miss simulates `referral(id)` on the router.

| Field | Type | Meaning |
|---|---|---|
| `id` | string | Echo, decimal |
| `active` | boolean | An active referral was found; `false` for missing, inactive, and id `0` |
| `referralFeeBps` | integer \| null | Active referral fee; `null` when none |
| `staticFeeBps` | integer (u32) | Cached router admin fee, charged only alongside an active nonzero referral |
| `totalFeeBps` | integer \| null | `0` for id `0`; `staticFeeBps + referralFeeBps` when active; `null` when missing or inactive |
| `feePolicy` | string | `"input unless output is whitelisted and input is not"` |
| `whitelistedTokens` | string[] | Cached router fee whitelist, sorted |

Errors (`DiscoveryError` `{code, error}`): `400 invalid_referral_id` (not a u32), `503 router_unavailable` (router not configured), `502 upstream_unavailable` (RPC lookup failed; retry shortly).

### GET /api/v1/quote

Exactly one of `amount_in` (forward: maximize net output) or `amount_out` (reverse:
minimize gross input). Every quote checks readiness before route search and
again before response delivery. `fresh=true` adds a live RPC ledger check.
Do not depend on internal validation order.

#### Freshness and slippage (canonical)

There is no quote expiry field. `snapshot` describes the search input, while the route's
absolute `amountOutMin` is the on-chain protection. Use `/ready`, request `fresh=true`
when ledger drift must reject the quote, and re-quote immediately before building.
Do not widen slippage to make an obsolete route execute. `snapshot_stale`
can occur without `fresh=true`. A quote does not reserve liquidity.

| Query param | Aliases | Type | Default | Constraint / meaning |
|---|---|---|---|---|
| `from` | — | string | required | Input token `C…` (56 chars), from `/api/v1/tokens` |
| `to` | — | string | required | Output token `C…`; an LP share token selects a liquidity operation |
| `amount_in` | `amountIn` | string | — | Gross input in atomic units **including any input-side fee**; `1..=i128::MAX`; surrounding whitespace trimmed |
| `amount_out` | `amountOut` | string | — | Required **net** output after fees; `1..=i128::MAX`. Exact-out preservation is external pinned behavior; see [composition.md#exact-out-status](composition.md#exact-out-status) |
| `slippage` | — | number | none | Output tolerance as a decimal fraction: `0.005` = 0.5%, `0.5` = 50%. Finite, in `[0, 1)`; rounded down to ppm. Required for `routeXdr`, `transaction`, and a forward `amountOutMin`; omit for indicative amounts. See the [canonical policy](#freshness-and-slippage-canonical) |
| `max_hops` | `maxHops` | integer | `4` | `1..=6` |
| `max_splits` | `maxSplits` | integer | `4` | `0..=8`; `0` is treated as `1` (single path) |
| `include_paths` | `includePaths` | boolean | `false` | Emit `paths[]`; `hops[]` is always present |
| `sender` | — | string | none | Account `G…` (56 chars). Requires `slippage`. Adds `transaction` (or `transactions[]` for `convertLiquidity`). Without `sender`, `slippage` alone still yields `routeXdr` |
| `referral_id` | `referralId`, `referral` | integer | `0` | `0..=4294967295` (the payload stores a `u32`). `0` = no referral **and no protocol fee**. A nonzero id must be an **active** referral on the router: `lp_fee_for` looks it up and answers `400 invalid_request` (`unknown referral N`) when it is missing or inactive; the id is baked into `routeXdr` |
| `simulate` | — | boolean | `true` | `true`: the server simulates and returns a prepared envelope (`transaction.simulated = true`) with the budget fallback ladder. `false`: unprepared envelope, no ladder. Ignored without `sender` |
| `platform` | — | string | `"aggregator"` | Only value accepted by the `Platform` enum |
| `fresh` | — | boolean | `false` | Compare the live RPC ledger with the snapshot; `409` when drift > `MAX_FRESH_LEDGER_DRIFT = 2`. Costs one RPC call |

Not a parameter: `router`. The router is fixed by the deployment (`AppState::router`, echoed by `/api/v1/config`); a `router` query key is silently ignored. Read the router from trusted configuration and check `/api/v1/config`.

For reverse quotes, slippage also increases the requested output used to size
input. Let `retainedPpm = 1_000_000 - floor(slippage × 1_000_000)`.
The sizing target is `ceil(requestedOut × 1_000_000 / retainedPpm)`.
The returned `amountIn` includes this buffer and any input-side fee.
Verify that the encoded floor still meets the caller's requested output.

#### Response: `QuoteResponse`

`?` marks `skip_serializing_if = Option::is_none`.

| Field | Type | Meaning |
|---|---|---|
| `snapshot` | object | Set on every 200 (`QuoteSnapshot`): `ledger` (u32), `ageSeconds` (u64), `checkedLiveLedger?` (u32, only with `fresh=true`). Describes the search input — **there is no quote TTL**; re-quote before execution |
| `mode` | string | `"forward"`, `"reverse"`, `"addLiquidity"`, `"removeLiquidity"`, `"convertLiquidity"` |
| `from` / `to` | string | Token ids as resolved |
| `tokenInKind` / `tokenOutKind` | `"soroban"` \| `"lp"` | Share-token flag per side |
| `amountIn` | string | **Gross** input the router pulls from the user, fee-inclusive when `feeOnInput=true`. This is `total_in` for `execute_strategy` and the cost to report in reverse mode |
| `amountOut` | string | Expected **net** output after aggregator/referral fees. A successful server-side simulation replaces the model estimate with the simulated delivered output (`apply_simulated_amount_out`) |
| `amountInShort` / `amountOutShort` | number | Display values (decimals applied) |
| `amountOutMin?` | string | Absolute net-output floor encoded in `routeXdr`. Present when `slippage` was supplied, or when `amount_out` was set on a quote other than `convertLiquidity`. Exact-out: [composition.md#exact-out-status](composition.md#exact-out-status) |
| `amountOutMinShort?` | number | Display value |
| `slippage?` | number | Echo of the request |
| `priceImpact?` | number | Modelled execution impact as a decimal, excluding aggregator/referral fees, against oracle-anchored prices when deep enough, else route spot rates; absent when no reference exists; not recomputed from simulation |
| `rate` | number | Net output per whole unit of gross input (approximate) |
| `rateInverse` | number | Gross input per whole unit of net output (approximate) |
| `decimalsIn` / `decimalsOut` | integer (u8) | Token decimals |
| `hops` | `QuoteSwap[]` | Flat hop list (all paths concatenated); LP mint/burn legs are not hops |
| `paths?` | `QuotePath[]` | Per-path breakdown; only with `includePaths=true` (`pipeline.rs` strips it otherwise) |
| `routeXdr?` | string | Base64 XDR of the `StrategyPayload` ScVal. Present when `slippage` was sent (always for swaps and add/remove liquidity; optional for `convertLiquidity`). Pass the decoded bytes as `swap_xdr` to `execute_strategy` or as the controller's `swap` argument; SDK lending builders take the base64 string as `steps: { routeXdr }`. Presence does not prove budget feasibility; simulate the full call |
| `transaction?` | `TransactionPayload` | Unsigned standalone envelope; requires `sender` + `slippage`. Mutually exclusive with `transactions` |
| `transactions?` | `TransactionPayload[]` | Ordered `convertLiquidity` steps; confirm each before preparing the next |
| `platform` | string | `"aggregator"` |
| `feeBps?` | integer (u32) | `staticFeeBps + referralFeeBps`; absent when no fee applies or when a `convertLiquidity` quote charges fees in different tokens |
| `feeAmount?` | string | Fee the router deducts, atomic units of the fee-side token; `amountOut`/`amountOutMin` are already net of it |
| `feeAmountShort?` | number | Display value |
| `feeOnInput?` | boolean | `true`: fee taken from `amountIn`; `false`: from the output. Mirrors the router rule `fee_on_input = !out_whitelisted \|\| in_whitelisted`. Absent when no fee applies |
| `amountInUsd?` / `amountOutUsd?` / `amountOutMinUsd?` | number | USD values when the token has a price |
| `lp?` | `QuoteLp` | Liquidity breakdown for `addLiquidity`/`removeLiquidity` |
| `degraded?` | `DegradedQuote` | Present when the server retried with reduced limits after `Budget, ExceededLimit`. **The quote is valid**; render a "routed through fewer paths" hint |

`QuoteSwap` (each `hops[]` / `paths[].swaps[]` entry): `dex` (`"Soroswap" | "Aquarius" | "AquariusClmm" | "Phoenix" | "Sushi" | "Comet"`), `kind` (`"ConstantProduct" | "Stable" | "Weighted" | "Concentrated"`), `address` (pool `C…`), `feeBps` (pool fee, excludes aggregator fees), `from`, `tokenInKind`, `to`, `tokenOutKind`, `amountIn`, `amountOut` (modelled hop amounts, atomic, excluding the top-level fee), `amountInShort`, `amountOutShort`.

`QuotePath`: `amountIn` (path input after any input-side fee), `amountOut` (path output before any output-side fee), `amountInShort`, `amountOutShort`, `splitPpm` (u32, share of routed input; `1000000` = 100%), `swaps[]`.

`QuoteLp`: `pool`, `dex`, `kind` (`"ConstantProduct" | "Stable"`), `feeBps`, `shareToken`, `amounts[]` (`{token, tokenKind, amount, amountShort}` per constituent; zero entries omitted), `preSwap?` (mint-only balancing swap, a `QuoteSwap`), `refunded[]` (mint-only dust returned; omitted when empty).

`DegradedQuote`: `requestedMaxSplits`, `effectiveMaxSplits`, `requestedMaxHops`, `effectiveMaxHops`, `fallbackAttempts` (1 = first fallback), `reason` (`"budget_exceeded"`). Ladder (`build_attempt_ladder`): original → `(max(splits/2, 1), max(hops/2, min(hops, 2)))` → `(1, min(hops, 2))`, deduplicated. Only `simulation_failed` bodies containing `Budget, ExceededLimit` advance the ladder.

`TransactionPayload` (`quote/types.rs`, built by `transaction/builder/{swap,envelope}.rs`):

| Field | Type | Meaning |
|---|---|---|
| `envelopeXdr` | string | Base64 `TransactionEnvelope` intended to call `execute_strategy(sender, total_in, swap_xdr)`. At the pinned external revision `seqNum` is placeholder `0`; verify the actual operation, source, invoked contract, args, and bytes before replacing it or signing |
| `sourceAccount` | string | Echo of `sender` |
| `routerContract` | string | Router `C…` selected by the deployment |
| `baseFee` | integer (u64) | At the pinned external revision: simulated total envelope fee (`100` base + `minResourceFee`), or unprepared placeholder `100`; never treat the placeholder as a usable resource fee |
| `networkPassphrase` | string | Signing domain |
| `notes` | string | Human checklist |
| `simulated` | boolean | `true`: footprint, auth, and resource fee attached by the server. Re-prepare and inspect before signing. `false`: simulation and assembly are required |
| `minResourceFee?` | string | Stroops, stringified; only when simulated |
| `latestLedger?` | integer (u32) | Ledger observed by the server's simulation; distinct from `snapshot.ledger` |

#### Error bodies (`ErrorResponse` `{code, error}`)

| HTTP | `code` | Trigger | Client action |
|---|---|---|---|
| 400 | `invalid_request` | Query deserialization failure; `slippage` not finite or outside `[0,1)`; `sender` without `slippage`; bad `sender` strkey; `referral_id > u32::MAX`; `from`/`to` not a `C…` id or `token … not tracked in snapshot`; both or neither of `amount_in`/`amount_out`; amount not an integer or outside `1..=i128::MAX`; `maxHops` outside `1..=6`; `maxSplits > 8`; `amountOut` above the liquidity route capacity; `unknown referral N` (nonzero id not active on the router); combined static + referral fee above 1000 bps; fee-adjusted amount outside the executable range; `simulate=false` envelope build failure | Fix the request. Do not resend unchanged |
| 404 | `no_route` | Both tokens resolved, no path between them for the amount and limits | Do not retry as-is; change tokens or amount, or raise `maxHops` |
| 409 | `snapshot_stale` | Inactive serving epoch, coverage age above 30 seconds, or observed ledger drift above 2; `fresh=true` also checks the live RPC ledger | Re-quote (optionally after `/ready` returns 200). Never submit the stale quote |
| 422 | `simulation_failed` | Server simulation failed. Only `Budget, ExceededLimit` means the route is too large; other diagnostics (auth, state) do not. A successful simulation may also report output `0`/below `amountOutMin` (`below execution minimum`) or no decodable output (`no valid delivered output`) | Re-quote unchanged first for missing/zero/below-min output. Reduce `maxSplits`/`maxHops` only for explicit `Budget, ExceededLimit`. Fix or surface other diagnostics |
| 422 | `minimum_output_unreachable` | Net output below the enforced floor `max(slippage floor, amount_out target, 1)`; on a fallback attempt the floor is the one fixed by the first attempt (`preserve_output_minimum`) | Re-quote |
| 500 | `internal_error` | Payload encoding failed; routed output unparsable; nonzero `referral_id` on a deployment with no router configured for fees | Retry unchanged once after a bounded delay; then report and fail |
| 502 | `upstream_error` | RPC unreachable during the `fresh` check, the referral lookup, or simulation | Transient: bounded backoff with jitter, then retry |
| 503 | `router_unavailable` | `sender` supplied but no router configured for the deployment | Wait for the service; check `/api/v1/config.router` and `/ready` |

## Request recipes

Standalone signed swap, forward:

```
GET /api/v1/quote?from=<C…>&to=<C…>&amountIn=1000000000&slippage=0.01&sender=<G…>&includePaths=true
```

Before signing, follow [SKILL.md#completion-checks](SKILL.md#completion-checks).
Exact-out usage: [composition.md#exact-out-status](composition.md#exact-out-status).
Freshness: [the canonical policy above](#freshness-and-slippage-canonical).

## Client error handling

Use the error-action table above. Apply bounded retries with backoff for
transient service failures. Do not retry an invalid request unchanged.
Keep one spelling for each query parameter. Do not add a camelCase alias
when the URL already contains its snake_case equivalent.
