# A087 — Prefetch prices / market indexes batching

- Agent: A087
- Theme: T6/T7
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/context/mod.rs:70-74` (`Cache::load_markets`)
  - `contracts/controller/src/context/oracle.rs:19-38` (`fetch_prices`, `cached_price`)
  - `contracts/controller/src/context/market_index.rs:12-42` (`put_market_index`, `fetch_market_indexes`, `cached_market_index`)
  - `contracts/controller/src/strategies/mod.rs:57-66` (`prefetch_strategy_prices`)
  - `contracts/controller/src/risk/totals.rs:24-40,69-75,81-86,169-178` (`account_price_assets`, `load_markets` call sites)
  - `common/src/collections.rs:15-45` (`unique_hub_tokens`, `collect_uncached_keys`)
  - `contracts/controller/src/external/price_aggregator.rs:20-30`, `external/pool.rs:155-162`
  - Strategy call sites: `multiply.rs:147`, `swap_debt.rs:57`, `swap_collateral.rs:56`, `repay_debt_with_collateral.rs:59`, `migrate_blend.rs:88`, `flash_position.rs:120`
  - `contracts/controller/src/views.rs:150-162` (detailed index view)
  - `contracts/controller/src/spec_hooks.rs:15-19` (Certora no-op index fetch)
- Defense: Prices and simulated market indexes are batched through `Cache` with uncached-key dedup before risk valuation. Strategies take an early fail-closed price snapshot (`prefetch_strategy_prices` → aggregator `prices`) covering open positions plus declared extras; finalize reuses the cache and only cross-contracts for still-missing keys. Touched markets refresh indexes from pool mutations via `put_market_index` (A077/A094). Liquidation warms the full portfolio once in `build_liquidation_plan` via `calculate_account_risk_totals` → `load_markets` before any `cached_price` use in math.
- Gap: (1) Strategies prefetch **prices only**, not market indexes — by design; untouched hubs batch at finalize/`load_markets`, touched hubs arrive via mutation. (2) `cached_price` never lazy-fetches (fail-closed panic) while `cached_market_index` lazy-fetches one hub at a time — asymmetry is intentional (ADR-0005) but keepers’ per-asset event path can N+1 pool reads before the HF `load_markets`. (3) `portfolio_hub_keys` concatenates without hub dedup (benign: `collect_uncached_keys` dedups). (4) `migrate_blend` appends debt assets to `price_assets` without `push_unique_address` (benign: `account_price_assets` dedups). (5) Views’ detailed path uses soft `fetch_prices_status`, not the hard Cache feed — correct for observability, not a money path. (6) No unit test first-pass for `fetch_market_indexes` analogous to `tests/context/oracle.rs`.
- Impact: No fund-theft or solvency-bypass vector from batching itself. Incomplete prefetch fails closed (`OracleNotConfigured` / missing aggregator key). Stale mid-tx prices are the ADR-0020 / ADR-0005 coherent-snapshot policy, not accidental reuse. Budget waste from keeper N+1 index reads or a forgotten strategy extra is DoS/revert or late `load_markets` fetch — not silent mis-valuation of a missing asset (missing price panics). Future footgun: any new path that calls `cached_price` without a prior `fetch_prices`/`load_markets` panics; any new pool leg that skips `put_market_index` leaves a simulated index for that hub (A094).
- Evidence: ADR-0005, ADR-0020 (“Prices used at finalize are the pre-callback snapshot”); SEED cache facts; unit `contracts/controller/tests/context/oracle.rs`; harness multiply unlisted-payment-before-transfer (A046); peer A086, A094, A077, A032, A045, A099. Pool `get_bulk_indexes` is simulate-only (`contracts/pool/src/lib.rs:307-316`).
- Opinion: Batching layer is defended for correctness. Keep the split — early hard price snapshot for strategies, mutation-backed index refresh, bulk `load_markets` at risk gates. Optionally prefetch indexes in strategies or keepers for budget only; do not refresh oracle mid-tx after legs.

## Method

1. Read `COORDINATION.md`, `SEED.md`, README finding format, ADR-0005, ADR-0020.
2. Inventoried Cache price/index APIs and `collect_uncached_keys` / `unique_hub_tokens`.
3. Traced every `prefetch_strategy_prices`, `load_markets`, `fetch_prices`, `fetch_market_indexes`, `cached_price`, `cached_market_index` call site under `contracts/controller/src`.
4. Cross-checked liquidation plan→math warm-up order, strategy finalize risk gates, keepers threshold sync, views detailed indexes.
5. Cross-linked peers A086 (inventory), A094 (post-pool staleness), A077 (mutation indexes), A032 (finalize batch), A045/A046 (flash/multiply prefetch), A099 (opt skip checks).
6. No novel critical gap: batching does not skip solvency or soften oracle failure.

---

## 1. Cache batching primitives

### 1.1 `load_markets` — joint warm-up

```70:74:contracts/controller/src/context/mod.rs
    pub(crate) fn load_markets(&mut self, hub_assets: &Vec<HubAssetKey>) {
        let assets = unique_hub_tokens(&self.env, hub_assets);
        self.fetch_prices(&assets);
        self.fetch_market_indexes(hub_assets);
    }
```

- Tokens: one Address per distinct underlying across hubs (same token on two hubs → one oracle key).
- Indexes: requested as `HubAssetKey`s; uncached subset bulk-fetched from the pool.
- Idempotent within a `Cache` lifetime: second call finds empty `missing` and returns.

### 1.2 Prices — hard fail-closed batch

| Step | Behavior |
|---|---|
| `fetch_prices` | `collect_uncached_keys` → single aggregator `prices` call → map into `token_prices` |
| Missing aggregator entry | Panic `OracleNotConfigured` (per asset in the request) |
| `cached_price` | **No** lazy fetch; panic if key absent |

Aggregator hard path rejects stale/invalid feeds (price-aggregator audit tests: hard `prices` reverts while soft `quotes`/`status` only flags). Controller money paths use hard `prices` only (ADR-0005).

### 1.3 Market indexes — bulk + lazy + overwrite

| API | Behavior |
|---|---|
| `fetch_market_indexes` | Uncached hubs → one `get_bulk_indexes` → zip by request order into `market_indexes` |
| `cached_market_index` | Hit cache, else single-element bulk fetch + insert |
| `put_market_index` | Overwrite after pool mutation (accrued write path) |

Pool `get_bulk_indexes` is **simulate-only** (no storage write): loads sync data and runs `simulate_update_indexes` to `now_ms`. Post-mutation truth must come from the mutation return + `put_market_index` (A077/A094).

Certora: `spec_hooks.rs` compiles `fetch_market_indexes` as a no-op; harnesses seed via `put_market_index`.

### 1.4 Dedup helper contract

`collect_uncached_keys` (common): first-seen order, skips keys already in the cache `Map`, O(n²) scan documented as safe only for position-limit-bounded inputs. Callers today are capped by `PositionLimits` / declared strategy lists — not attacker-growable unbounded vecs on these paths.

---

## 2. Who warms what

### 2.1 Risk totals (canonical money valuation)

| Function | Prefetch |
|---|---|
| `calculate_account_risk_totals_body` | `load_markets(portfolio_hub_keys(supply, borrow))` then iterate with `cached_*` |
| `sum_debt_usd` | `load_markets(borrow keys)` |
| `calculate_ltv_collateral_wad` | `load_markets(supply keys)` |
| `sum_debt_usd_loaded` | Assumes already loaded (used after portfolio warm-up) |

`require_post_pool_risk_gates` and liquidation plan/post-totals all go through `calculate_account_risk_totals` → full portfolio `load_markets`. One coherent price+simulated-index snapshot for remaining positions, then legs that already mutated have overwritten indexes.

`portfolio_hub_keys` is a plain append (no hub dedup). Duplicate hubs across supply and borrow are collapsed by `collect_uncached_keys` before the pool call.

### 2.2 Strategies — price prefetch only

```57:66:contracts/controller/src/strategies/mod.rs
pub(crate) fn prefetch_strategy_prices(
    cache: &mut Cache,
    account: &Account,
    extra_assets: &Vec<Address>,
) {
    let assets = account_price_assets(cache.env(), account, extra_assets);
    cache.fetch_prices(&assets);
}
```

`account_price_assets`: unique Addresses from supply keys + borrow keys + extras.

| Strategy | Extras passed | Notes |
|---|---|---|
| `multiply` | collateral, debt, optional payment asset | Fail-fast unlisted payment (A046) |
| `swap_debt` | existing + new debt assets | |
| `swap_collateral` | current + new collateral | After `require_can_supply(new)` |
| `repay_debt_with_collateral` | collateral + debt | |
| `migrate_blend` | withdraw (coll+supply) + debt-cap assets | Debt append not pre-deduped; OK |
| `flash_position` | debt + each collateral asset | **Not** `refund_assets` (refunds are not valued) |
| `flash_loan` | none | Pool-settled; no account / no oracle |

Intentional properties:

1. **Early fail-closed** before token movement / callback (unconfigured oracle aborts).
2. **Frozen price snapshot** through finalize (ADR-0020 for flash_position; ADR-0005 “one transaction observes one coherent valuation snapshot”).
3. **Indexes not prefetched** — strategy legs source indexes from pool mutations (`put_market_index`); remaining hubs batch inside finalize’s `require_post_pool_risk_gates` → `load_markets`. Mid-leg paths do not walk `cached_price`/`cached_market_index` for HF.

`strategy_finalize`: restamp LTV (config only) → post-pool risk gates → `finalize_position_flow` (A032). Second `fetch_prices` inside `load_markets` is a no-op for already-cached assets.

### 2.3 Liquidation — warm once, then reuse

```39:44:contracts/controller/src/positions/liquidation/plan.rs
    let totals = risk::calculate_account_risk_totals(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
```

Order inside `build_liquidation_plan`:

1. `load_markets` for entire book (prices + simulated indexes).
2. Seizure proportions / bonus / repayment / seize math all call `cached_price` + `cached_market_index` — cache already warm; no second aggregator round-trip.
3. Apply repay/seize uses **plan-embedded** `market_index` on entries for share math / events; pool mutations still `put_market_index` via merge helpers.
4. Post-liq `calculate_account_risk_totals` for bad-debt gate: reuses cached prices; indexes for touched hubs are post-mutation; untouched hubs still simulated from step 1 (same ledger time within the tx).

No path calls liquidation math `cached_price` before plan warm-up on the production entrypoints.

### 2.4 Keepers — lazy index N+1 then bulk risk

`sync_account_thresholds`: per changed supply hub calls `cached_market_index` (possible one pool call each) for `ParamUpd` events, then on `FullTuple` runs `calculate_account_risk_totals` → `load_markets` (prices + any still-missing indexes). Correctness OK; budget suboptimal vs an upfront `fetch_market_indexes(&supply_keys)`.

### 2.5 Views

| View helper | Batching |
|---|---|
| `get_market_index` | `Cache::new_view` + `cached_market_index` (lazy single) |
| `get_all_market_indexes_detailed` | `fetch_market_indexes` bulk + soft `fetch_prices_status` (does **not** fill `token_prices`) |
| Risk views (`sum_debt_usd`, LTV, HF) | `load_markets` / risk totals |

Soft status for detailed views is deliberate observability; money paths never read `PriceStatus.valid` as a permissive fallback.

---

## 3. Correctness properties of batching

### 3.1 No partial hard price set for risk

If any requested asset lacks a hard quote, `fetch_prices` panics before maps are partially relied on for a completed risk decision in that call’s success path. A panic rolls back the Soroban transaction. Strategies that prefetch before movement therefore cannot partially execute with a missing extra’s price.

### 3.2 Price vs index freshness (same invocation)

| Data | Source mid-tx | At risk gate |
|---|---|---|
| Token price | First successful `fetch_prices` / strategy prefetch | Same cached `PriceFeedRaw` (not re-queried if present) |
| Market index (untouched hub) | Simulated bulk / lazy | Same simulated cache entry unless overwritten |
| Market index (mutated hub) | `put_market_index` from pool return | Post-accrual mutation index |

Using a pre-callback price with post-callback positions is required for flash_position (ADR-0020). Using simulated indexes for pre-pool planning then mutation indexes after legs is required for caps/usage (A077). Cross-ledger races between simulate and another tx’s accrual are normal concurrency (A094), not a Cache bug.

### 3.3 Token vs hub keying

Prices keyed by `Address`; indexes by `HubAssetKey`. Multi-hub same underlying shares one oracle snapshot and keeps per-market indexes — correct for USD valuation and RAY share math.

### 3.4 What batching does **not** skip

- Solvency / HF / min-borrow gates still run after legs (`require_post_pool_risk_gates`).
- Hub-active / spoke flags / flash guards are independent of price/index maps (A099).
- `flash_loan` correctly skips oracle (no controller valuation).

---

## 4. Call-site matrix (prefetch coverage)

| Entrypoint family | Price batch | Index batch | Notes |
|---|---|---|---|
| Ordinary supply (debt-free) | Often none | Mutation put only | Gates skipped when debt-free (A025/A099) |
| Borrow / withdraw / risky supply | Via post-pool `load_markets` | Mutation put + load residual | |
| Strategies (six above) | Early `prefetch_strategy_prices` | Mutation put + finalize `load_markets` | |
| Liquidation | Plan `load_markets` | Plan simulate + apply put | Math assumes warm cache |
| Bad-debt cleanup | `load_markets` in totals | Same | |
| Keepers threshold | Via HF `load_markets` | Lazy per event + load | Opt gap |
| Views detailed | Soft status (not Cache) | Bulk fetch | |
| Certora | Harness-dependent | `fetch_market_indexes` no-op | |

---

## 5. Residuals and non-findings

### 5.1 Residuals (optimization / hygiene)

1. **Strategy index prefetch absent** — could add `fetch_market_indexes` on portfolio+declared hubs before legs to collapse finalize’s residual bulk call; not required for safety.
2. **Keeper N+1** — prefetch supply hub indexes once before the ParamUpd loop.
3. **`portfolio_hub_keys` / migrate debt list dedup** — cosmetic; helpers already dedup.
4. **Test gap** — add a first-pass killer for `fetch_market_indexes` empty/missing/dedup like `tests/context/oracle.rs`.
5. **Future API discipline** — document: never call `cached_price` without prior warm-up; always `put_market_index` after pool mutations (checklist with A094).

### 5.2 Non-findings (checked, not gaps)

- Prefetch does not freeze a wrong index for cap enforcement on the mutating leg (caps use mutation index).
- Missing strategy extra for an asset that later enters the book still fails at finalize `load_markets` if unpriced (fail-closed), or hits cache if extras covered it.
- `refund_assets` omitted from flash_position extras: refunds are not risk-valued.
- Soft view quotes cannot be confused with hard Cache feeds on money paths.
- Certora no-op fetch is harness-scoped, not production WASM.

---

## 6. Cross-links

| Peer | Relation |
|---|---|
| A086 | Field inventory; this finding is the price/index warm-up slice |
| A094 | Post-pool index overwrite vs simulated bulk |
| A077 | Cap/usage use mutation indexes, not prefetch |
| A032 | Finalize batches storage after risk (which reuses Cache) |
| A045 / A046 | Flash/multiply rely on early price snapshot |
| A099 | Memo short-circuits do not skip failed oracle checks |
| A008 | View bounds; views still cross-contract read indexes/oracles |

---

## Verdict

Prefetch/batching of prices and market indexes in `Cache` and strategies is **defended**. Hard oracle batching + uncached-key dedup + mutation index overwrite + risk-gate `load_markets` form a coherent T6/T7 design aligned with ADR-0005/0020. Remaining issues are budget/hygiene (strategy/keeper index prefetch, tests), not undefended valuation holes.
