# Quote Server HTTP Reference

Every quote server route, with parameters, response fields, units, and error bodies. Payload format: [payload.md](payload.md); lending composition: [composition.md](composition.md); base URLs and deployment check: [SKILL.md](SKILL.md#base-urls-and-the-deployment-check).

Sources: the deployed quote server and its OpenAPI document; base URLs from
`STELLAR_NETWORKS[*].quoteUrl` in the [pinned SDK](../xoxno-lending/SKILL.md#verified-against).
The server is off-chain: this repository's contracts and tests do not enforce
the routes, status bodies, retry behavior, exact-out handling, or envelope
placeholders below. Re-check the live OpenAPI document after a server release.

## Conventions

| Rule | Detail |
|---|---|
| Method | All `GET`; no auth; permissive CORS (`Any` origin, method, header) |
| Amounts | Base-unit integers as decimal **strings** (`u128` on the wire, `1..=i128::MAX` accepted); `*Short` fields are `f64` whole-token display values, never for transaction arithmetic |
| Token ids | 56-char `C…` contract strkeys only; `XLM`, `CODE:GISSUER`, `G…` → `400 invalid_request`. Resolve from `GET /api/v1/tokens` (XLM under its Stellar Asset Contract id); lending market ids in [addresses](../xoxno-lending/addresses.md) |
| Naming | `/health`, `/ready` bodies **snake_case** (`last_applied_ledger`, `seconds_since_last_apply`); `/api/v1/*` bodies **camelCase**, except `/api/v1/prices` `depth_usd` |
| Query names | `/api/v1/quote` names are snake_case; camelCase aliases for `amount_in`, `amount_out`, `max_hops`, `max_splits`, `include_paths`, `referral_id` (plus `referral`). `from`, `to`, `slippage`, `sender`, `simulate`, `platform`, `fresh` have one spelling. Unknown keys (e.g. `router`) are ignored |
| Limits | 30 s timeout (`408`, empty body), 1 MiB body limit, `x-request-id` propagated or generated |
| Errors | JSON `{ "code": string, "error": string }` on `/api/v1/quote` and `/api/v1/referrals/{id}`; branch on stable `code`, not free-text `error` |
| OpenAPI | `GET /api-docs/openapi.json`; Swagger UI at `/` |

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

Process liveness; a 200 does not mean quotes are fresh (use `/ready`).

| Field | Type | Meaning |
|---|---|---|
| `status` | string | Always `"healthy"` |
| `pools` | integer | Indexed swap edges (multi-token pools count several) |
| `tokens` | integer | Token ids in the snapshot interner |
| `decimals_entries` | integer | Tokens with a resolved decimal precision |
| `last_applied_ledger` | integer (u32) | Ledger the snapshot is applied through |
| `last_applied_cursor` | string | Opaque event-stream cursor |
| `seconds_since_last_apply` | integer (u64) | Seconds since snapshot last advanced |
| `static_fee_bps` | integer (u32) | Cached router admin fee (bps), excluding referral fees |
| `usd_prices_cached` | integer | Oracle price cache entries |

### GET /ready

`200` only when the serving epoch is active, coverage age is at most 30
seconds, and observed ledger drift is at most 2 ledgers; otherwise `503`.
Preserve the body for diagnosis.

| Field | Type | Meaning |
| --- | --- | --- |
| `ready` | boolean | All readiness conditions hold |
| `seconds_since_last_apply` | u64 | Coverage age; u64 max when unknown |
| `staleness_threshold_secs` | u64 | `30` |
| `last_applied_ledger` | u32 | Snapshot ledger |
| `pools` | integer | Indexed swap edges |

### GET /api/v1/config

| Field | Type | Meaning |
|---|---|---|
| `network` | `"mainnet"` \| `"testnet"` | Deployment network |
| `networkPassphrase` | string | Signing domain of every returned envelope |
| `router` | string \| null | Deployment router `C…`, from `AGGREGATOR_ROUTER`, then the seed file, then the network default; `null` when none resolves. Must equal `configs/networks.json` `aggregator` for that network |
| `apiVersion` | string | `"v1"` |
| `protocolVersion` | integer (u8) | Packed program version: `1` ([payload.md](payload.md)) |
| `defaultMaxHops` | integer | `4` |
| `defaultMaxSplits` | integer | `4` |
| `maxHops` | integer | `6` |
| `maxSplits` | integer | `8` |
| `operations` | string[] | `["swap","addLiquidity","removeLiquidity","convertLiquidity"]` |
| `venues` | string[] | Sorted DEX labels in the snapshot; not a route promise |
| `lastAppliedLedger` | integer (u32) | Snapshot ledger |

### GET /api/v1/tokens

Array of `TokenEntry`, sorted by pool count descending, then `id`. Resolve `from`, `to`, and `decimals` here.

| Field | Type | Meaning |
|---|---|---|
| `id` | string | Token contract `C…`; pass as `from`/`to` |
| `decimals` | integer (u8) | Display divisor `10^decimals` |
| `lp` | boolean | Pool share token; quoting from/to it is add/remove liquidity |
| `dexes` | string[] | Sorted DEX labels; for LP, the issuing DEX |
| `pool` | string | LP only: issuing pool contract |
| `assets` | string[] | LP only: constituent token ids in pool order |

Type: `StellarQuoteToken` in the [pinned SDK](../xoxno-lending/SKILL.md#verified-against).

### GET /api/v1/prices

`Record<tokenId, PriceEntry>`. A token is absent (not zero) when no usable path of at most 4 hops reaches it from an oracle-priced token. An LP share is priced from its constituents and absent unless all are priced.

| Field | Type | Meaning |
|---|---|---|
| `usd` | number | USD per whole token |
| `source` | `"oracle"` \| `"pool"` \| `"lp"` | Reflector price or network USD anchor (USDC on mainnet, fixed `1.0`); depth-weighted pool average; LP share valued from its pool |
| `depth_usd` | number | Summed source-pool depth, or LP pool TVL; absent for oracle. Threshold it to drop untradeable-pool prices |
| `hops` | integer (u8) | Hops from nearest oracle-priced token; `0` = oracle |

### GET /api/v1/pools

| Field | Type | Meaning |
|---|---|---|
| `lastAppliedLedger` | integer (u32) | Snapshot ledger |
| `pools[].id` | string | Pool contract; multi-token edges grouped |
| `pools[].dex` | string | Venue label |
| `pools[].assets` | string[] | Sorted, deduplicated constituent ids |
| `pools[].feeBps` | integer (u32) | Pool swap fee (bps), excluding aggregator/referral fees |
| `pools[].shareToken` | string \| null | Non-null only when the snapshot supports this pool's liquidity |

### GET /api/v1/referrals/{id}

Path `id`: u32 decimal; `0` = no referral. Served from a 60 s cache; a miss simulates router `referral(id)`.

| Field | Type | Meaning |
|---|---|---|
| `id` | string | Echo, decimal |
| `active` | boolean | `false` for missing, inactive, or id `0` |
| `referralFeeBps` | integer \| null | Active referral fee, else `null` |
| `staticFeeBps` | integer (u32) | Cached admin fee, charged only with an active nonzero referral |
| `totalFeeBps` | integer \| null | `0` for id `0`; `staticFeeBps + referralFeeBps` when active; else `null` |
| `feePolicy` | string | `"input unless output is whitelisted and input is not"` |
| `whitelistedTokens` | string[] | Cached router fee whitelist, sorted |

Errors: `400 invalid_referral_id` (not a u32), `503 router_unavailable` (no router configured), `502 upstream_unavailable` (RPC lookup failed; retry shortly).

### GET /api/v1/quote

Exactly one of `amount_in` (forward: maximize net output) or `amount_out`
(reverse: minimize gross input). Every quote checks readiness before route
search and before response delivery; `fresh=true` adds a live RPC ledger
check. Do not depend on internal validation order.

#### Freshness and slippage (canonical)

There is no quote expiry field and a quote reserves no liquidity. `snapshot`
describes the search input; the route's absolute `amountOutMin` is the
on-chain protection. Use `/ready`, request `fresh=true` when ledger drift must
reject the quote, and re-quote immediately before building. Do not widen
slippage to make an obsolete route execute. `snapshot_stale` can occur without
`fresh=true`.

| Query param | Aliases | Type | Default | Constraint / meaning |
|---|---|---|---|---|
| `from` | — | string | required | Input token `C…` from `/api/v1/tokens` |
| `to` | — | string | required | Output token `C…`; an LP share selects a liquidity operation |
| `amount_in` | `amountIn` | string | — | Gross input **including any input-side fee**; `1..=i128::MAX`; whitespace trimmed |
| `amount_out` | `amountOut` | string | — | Required **net** output; `1..=i128::MAX`. Exact-out is off-chain server behavior ([status](composition.md#exact-out-status)) |
| `slippage` | — | number | none | Decimal fraction (`0.005` = 0.5%), finite, in `[0, 1)`, floored to ppm. Required for `routeXdr`, `transaction`, and forward `amountOutMin`; omit for indicative amounts |
| `max_hops` | `maxHops` | integer | `4` | `1..=6` |
| `max_splits` | `maxSplits` | integer | `4` | `0..=8`; `0` is treated as `1` (single path) |
| `include_paths` | `includePaths` | boolean | `false` | Emit `paths[]` (`hops[]` always present) |
| `sender` | — | string | none | Account `G…`; requires `slippage`; adds `transaction` (`transactions[]` for `convertLiquidity`). `routeXdr` needs only `slippage` |
| `referral_id` | `referralId`, `referral` | integer | `0` | `0..=4294967295` (the payload stores a `u32`). `0` = no referral **and no protocol fee**. A nonzero id must be an **active** referral on the router, else `400 invalid_request` (`unknown referral N`). The id is baked into `routeXdr` |
| `simulate` | — | boolean | `true` | `true`: prepared envelope (`transaction.simulated = true`) with budget fallback ladder; `false`: unprepared, no ladder. Ignored without `sender` |
| `platform` | — | string | `"aggregator"` | Only accepted value |
| `fresh` | — | boolean | `false` | One RPC call compares live ledger with snapshot; `409` when drift > 2 |

`router` is not a parameter: the deployment fixes it, `/api/v1/config` echoes it, and a `router` query key is ignored.

For reverse quotes, slippage also raises the output used to size input:
`retainedPpm = 1_000_000 - floor(slippage × 1_000_000)` and the sizing target
is `ceil(requestedOut × 1_000_000 / retainedPpm)`. The returned `amountIn`
includes this buffer and any input-side fee. Verify that the encoded floor
still meets the caller's requested output.

#### Response: `QuoteResponse`

`?` = omitted when empty.

| Field | Type | Meaning |
|---|---|---|
| `snapshot` | object | Every 200: `ledger` (u32), `ageSeconds` (u64), `checkedLiveLedger?` (u32, `fresh=true` only). Search input; **no quote TTL** |
| `mode` | string | `"forward"`, `"reverse"`, `"addLiquidity"`, `"removeLiquidity"`, `"convertLiquidity"` |
| `from` / `to` | string | Token ids as resolved |
| `tokenInKind` / `tokenOutKind` | `"soroban"` \| `"lp"` | Share-token flag |
| `amountIn` | string | **Gross** input pulled from the user (fee-inclusive when `feeOnInput=true`); `total_in` for `execute_strategy`; reverse-mode cost |
| `amountOut` | string | Expected **net** output after aggregator/referral fees; simulated delivered output when server simulation succeeds |
| `amountInShort` / `amountOutShort` | number | Display values |
| `amountOutMin?` | string | Absolute net-output floor in `routeXdr`. Present with `slippage`, or with `amount_out` outside `convertLiquidity` ([exact-out](composition.md#exact-out-status)) |
| `amountOutMinShort?` | number | Display value |
| `slippage?` | number | Echo of the request |
| `priceImpact?` | number | Modelled impact (decimal), excluding aggregator/referral fees, vs oracle-anchored prices when deep enough, else route spot rates; absent without a reference; not recomputed from simulation |
| `rate` | number | Net output per whole unit of gross input (approximate) |
| `rateInverse` | number | Gross input per whole unit of net output (approximate) |
| `decimalsIn` / `decimalsOut` | integer (u8) | Token decimals |
| `hops` | `QuoteSwap[]` | All paths' hops concatenated; LP mint/burn legs excluded |
| `paths?` | `QuotePath[]` | Per-path breakdown; `includePaths=true` only |
| `routeXdr?` | string | Base64 XDR of the `StrategyPayload` ScVal. Present with `slippage` (always for swaps and add/remove liquidity; optional for `convertLiquidity`). Decoded bytes go to `execute_strategy` `swap_xdr` or the controller's `swap`; SDK builders take the base64 as `steps: { routeXdr }`. Not proof of budget feasibility |
| `transaction?` | `TransactionPayload` | Unsigned envelope; needs `sender` + `slippage`; excludes `transactions` |
| `transactions?` | `TransactionPayload[]` | Ordered `convertLiquidity` steps; confirm each before preparing the next |
| `platform` | string | `"aggregator"` |
| `feeBps?` | integer (u32) | `staticFeeBps + referralFeeBps`; absent with no fee or a `convertLiquidity` quote charging fees in several tokens |
| `feeAmount?` | string | Router fee in fee-side token units; `amountOut`/`amountOutMin` already net of it |
| `feeAmountShort?` | number | Display value |
| `feeOnInput?` | boolean | Fee from input (`true`) or output; router rule `fee_on_input = !out_whitelisted \|\| in_whitelisted`. Absent with no fee |
| `amountInUsd?` / `amountOutUsd?` / `amountOutMinUsd?` | number | USD values when priced |
| `lp?` | `QuoteLp` | Liquidity breakdown for `addLiquidity`/`removeLiquidity` |
| `degraded?` | `DegradedQuote` | Server retried with reduced limits after `Budget, ExceededLimit`. **Quote is valid**; show a "fewer paths" hint |

`QuoteSwap` (each `hops[]` / `paths[].swaps[]` entry): `dex` (`"Soroswap" | "Aquarius" | "AquariusClmm" | "Phoenix" | "Sushi" | "Comet"`), `kind` (`"ConstantProduct" | "Stable" | "Weighted" | "Concentrated"`), `address` (pool `C…`), `feeBps` (pool fee, excludes aggregator fees), `from`, `tokenInKind`, `to`, `tokenOutKind`, `amountIn`, `amountOut` (modelled hop amounts, atomic, excluding the top-level fee), `amountInShort`, `amountOutShort`.

`QuotePath`: `amountIn` (path input after any input-side fee), `amountOut` (path output before any output-side fee), `amountInShort`, `amountOutShort`, `splitPpm` (u32, share of routed input; `1000000` = 100%), `swaps[]`.

`QuoteLp`: `pool`, `dex`, `kind` (`"ConstantProduct" | "Stable"`), `feeBps`, `shareToken`, `amounts[]` (`{token, tokenKind, amount, amountShort}` per constituent; zero entries omitted), `preSwap?` (mint-only balancing swap, a `QuoteSwap`), `refunded[]` (mint-only dust returned; omitted when empty).

`DegradedQuote`: `requestedMaxSplits`, `effectiveMaxSplits`, `requestedMaxHops`, `effectiveMaxHops`, `fallbackAttempts` (1 = first fallback), `reason` (`"budget_exceeded"`). Fallback ladder: original → `(max(splits/2, 1), max(hops/2, min(hops, 2)))` → `(1, min(hops, 2))`, deduplicated. Only `simulation_failed` bodies containing `Budget, ExceededLimit` advance the ladder.

`TransactionPayload`:

| Field | Type | Meaning |
|---|---|---|
| `envelopeXdr` | string | Base64 `TransactionEnvelope` calling `execute_strategy(sender, total_in, swap_xdr)`, `seqNum` placeholder `0`. Verify operation, source, contract, args, and bytes before replacing it or signing |
| `sourceAccount` | string | Echo of `sender` |
| `routerContract` | string | Deployment router `C…` |
| `baseFee` | integer (u64) | Simulated total fee (`100` + `minResourceFee`), or unprepared placeholder `100` (not a usable resource fee) |
| `networkPassphrase` | string | Signing domain |
| `notes` | string | Human checklist |
| `simulated` | boolean | `true`: server attached footprint, auth, resource fee (re-prepare and inspect before signing); `false`: simulate and assemble |
| `minResourceFee?` | string | Stroops; simulated only |
| `latestLedger?` | integer (u32) | Server simulation ledger, not `snapshot.ledger` |

#### Error bodies (`ErrorResponse` `{code, error}`)

| HTTP | `code` | Trigger | Client action |
|---|---|---|---|
| 400 | `invalid_request` | Malformed request ([triggers](#invalid_request-triggers)) | Fix; never resend unchanged |
| 404 | `no_route` | No path for the amount and limits | Change tokens or amount, or raise `maxHops` |
| 409 | `snapshot_stale` | Inactive epoch, coverage age > 30 s, or ledger drift > 2 (`fresh=true` also checks live RPC) | Re-quote, optionally after `/ready` is 200; never submit the stale quote |
| 422 | `simulation_failed` | Server simulation failed; only `Budget, ExceededLimit` means route too large. Also output `0`/below `amountOutMin` (`below execution minimum`) or undecodable (`no valid delivered output`) | Missing/zero/below-min output: re-quote unchanged. Reduce `maxSplits`/`maxHops` only on `Budget, ExceededLimit`. Surface other diagnostics |
| 422 | `minimum_output_unreachable` | Net output below `max(slippage floor, amount_out target, 1)`; fallbacks keep the first attempt's floor | Re-quote |
| 500 | `internal_error` | Payload encoding failed; unparsable routed output; nonzero `referral_id` with no fee router configured | Retry once after a bounded delay, then report and fail |
| 502 | `upstream_error` | RPC unreachable during `fresh` check, referral lookup, or simulation | Retry with bounded jittered backoff |
| 503 | `router_unavailable` | `sender` with no router configured | Wait; check `/api/v1/config.router` and `/ready` |

#### `invalid_request` triggers

`400 invalid_request` when:

- the query string fails to parse;
- `slippage` is not finite or is outside `[0,1)`;
- `sender` is sent without `slippage`;
- `sender` is not a valid strkey;
- `referral_id > u32::MAX`;
- `from` or `to` is not a `C…` id, or is `token … not tracked in snapshot`;
- both or neither of `amount_in`/`amount_out` are sent;
- an amount is not an integer or is outside `1..=i128::MAX`;
- `maxHops` is outside `1..=6`;
- `maxSplits > 8`;
- `amountOut` exceeds the liquidity route capacity;
- a nonzero referral id is not active on the router (`unknown referral N`);
- the combined static + referral fee exceeds 1000 bps;
- the fee-adjusted amount is outside the executable range;
- a `simulate=false` envelope build fails.

## Request recipes

Standalone signed swap, forward:

```
GET /api/v1/quote?from=<C…>&to=<C…>&amountIn=1000000000&slippage=0.01&sender=<G…>&includePaths=true
```

Before signing, follow [SKILL.md#completion-checks](SKILL.md#completion-checks).
Exact-out usage: [composition.md#exact-out-status](composition.md#exact-out-status).

## Client error handling

Follow the error-action table, with bounded backoff retries only for
transient failures. Use one spelling per query parameter: never add a
camelCase alias when the URL already has its snake_case form.
