# A100 — Dead cache paths / unused memo maps

- Agent: A100
- Theme: T7 (also T6 budget; T8 hygiene)
- Severity: info
- Status: optimization-note
- Paths:
  - `contracts/controller/src/context/mod.rs:25-83` (`Cache` fields, constructors, `load_markets`, `require_hub_active`)
  - `contracts/controller/src/context/{pool,oracle,market_index,spoke,events}.rs` (every `pub(crate)` Cache method)
  - `contracts/controller/src/spec_hooks.rs:10-20` (Certora `fetch_market_indexes` no-op)
  - `contracts/controller/src/views.rs:30-42,53-98,148-186` (`health_factor` eager Cache; detailed indexes bypass `token_prices`)
  - `contracts/controller/src/strategies/flash_loan.rs:32-33` (full Cache for `pool_address` only)
  - `contracts/controller/src/strategies/flash_position.rs:80-94` (sole mutator `cached_pool_sync_data` — one shot)
  - `contracts/controller/src/keepers.rs:17-91,207-236` (address-only keepers; `ParamUpd` lazy indexes; empty emit)
  - `contracts/controller/src/markets.rs:89-97` (`upgrade_liquidity_pool_params` address-only)
  - `contracts/controller/src/risk/{totals,validation}.rs` (`load_markets`; debt-free gate skip)
  - `contracts/controller/src/strategies/mod.rs:59-66` (`prefetch_strategy_prices`)
  - `contracts/controller/src/external/price_aggregator.rs:20-45` (hard `prices` vs soft `quotes`)
  - `common/src/types/{pool,oracle}.rs` (`PoolSyncData` / `PriceFeedRaw` payloads)
  - Unit `contracts/controller/tests/context/oracle.rs` (price hit / skip; **no** sync-hit or index first-pass)
- Defense: Every `Cache` field is constructed, and every field has at least one production **writer or reader** (A086 inventory holds at the type level). No map is vestigial in the “never touched, can delete today” sense except the **stored `bool` in `verified_hubs`** (presence-only; A090). Fail-closed `cached_price` miss, success-only hub memo, spoke pin, and index overwrite are live controls, not dead code.
- Gap: (1) **`pool_sync_data` hit path is production-dead** — every reader is first-and-only; the map is write-only overhead and the latent A088 stale-hit trap has no current trigger. (2) **Most of each `PoolSyncData` blob is unused** at Cache consumers (`is_flashloanable` or `asset_decimals` only; `state.*` never read from Cache). (3) **`PriceFeed.timestamp` is unused** after `cached_price`. (4) **Invocation-class empty maps** — `flash_loan` / `update_indexes` / `claim_revenue` / `recapitalize` / `upgrade_liquidity_pool_params` allocate the full struct and use only `pool_address`. (5) **`health_factor` / `can_be_liquidated` allocate `new_view` before the debt-free/missing-account early return.** (6) **`get_all_market_indexes_detailed` never fills `token_prices`** (soft `fetch_prices_status` off-Cache). (7) **Certora `fetch_market_indexes` no-op** makes the bulk-fill path dead under that feature; lazy `cached_market_index` carries the load. (8) **Keeper FullTuple** N+1-lazy-fills supply indexes so the subsequent bulk fetch is a no-op for those keys (A087). (9) **`set_prices` is `#[cfg(test)]` only** (correct; not in deployable WASM). (10) **`cached_price` takes `&mut self` but does not mutate.** No unit asserts sync-map second get, hub-memo hit, or bulk-index first-pass (A087 residual).
- Impact: **No fund-theft, share-mint, undercollateralized exit, or skipped security check** from dead paths. Unused maps/hits are CPU/RAM/instance-rent budget (Soroban Map alloc on every `Cache::new*`) and reviewer confusion (fill-once sync looks load-bearing). Blast radius of deleting a “dead” hit without checking future readers: A088 stale-sync if a post-leg `cached_pool_sync_data` is added; A090 `contains_key` if someone stores `false`. Practical impact today ≈ **negligible** for safety; small for budget on address-only mutators. Severity **info**.
- Evidence: Exhaustive grep of every Cache method and field under `contracts/controller/src`; constructors vs entrypoint matrix in §3; hit/miss classification in §2; peers **A086** (inventory), **A087** (batching / N+1 / Certora no-op), **A088** (sync fill-once), **A089–A091** (spoke maps live), **A090/A099** (`verified_hubs` presence), **A092** (event Vecs live append), **A093** (`new` TTL on address-only paths), **A094** (index overwrite is the live memo), **A095** (savings vs correctness), **A104** (listed A100 as hole: “A086 all fields appear used”). Unit `fetch_prices_skips_already_cached_assets` proves **price** hit, not sync hit. SEED Cache facts.
- Opinion: **Confirm A086: there is no wholly unused memo map.** Refine A104’s hole-closure: “all fields appear used” ≠ “every fill earns a hit.” Treat `pool_sync_data` as a **preflight fetch disguised as a map** until a second reader exists; do not add invalidation for a hit that never happens. Do not delete fields to “fix” empty invocation-class maps without a slimmer `Cache` newtype (optional, budget-only). Keep `set_prices` test-gated. Optional: construct `health_factor` Cache only on the indebted branch; document sync as write-through of one FFI.

---

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, README finding format, `AGENT_MANIFEST` Wave 6 (**A100**), peers **A086–A099** (filed), **A104** §7 A100 hole, adjacency A008/A033/A034.
2. Listed every `Cache` field and every `pub(crate) fn` on `impl Cache` across `context/{mod,pool,oracle,market_index,spoke,events}.rs` plus Certora `spec_hooks.rs`.
3. Grepped production `contracts/controller/src` for each method. Classified **miss / hit / never-called / cfg-gated**.
4. Built an entrypoint × field occupancy matrix (`Cache::new` / `new_view` sites).
5. Classified payload fields inside memoized blobs (`PoolSyncData`, `PriceFeedRaw`, `verified_hubs` bool).
6. Cross-checked tests/Certora for hit-path coverage.
7. No production Rust edited. No git operations (COORDINATION).

No novel Critical/High. Closes A104’s A100 hole: **dead-code proof is “no unused maps; several unused hits/payloads/allocations.”**

---

## 0. What “dead” means here

| Class | Meaning | Security? |
|---|---|---|
| **D0 Unused field** | Field never read and never written except constructor | Would be delete-safe |
| **D1 Dead hit** | Miss fills the map; **no production second get** | Budget + latent A088 if a second get is added |
| **D2 Dead payload** | Blob stored; **consumer reads a subset of fields** | Budget; cannot shrink FFI without a new pool view |
| **D3 Invocation-empty** | Field live on *some* entrypoints; **empty for this entrypoint’s whole Cache life** | Budget (full struct still allocated) |
| **D4 Dead branch** | Hit or miss arm unreachable **on a given path** but live elsewhere | Fine if documented |
| **D5 Feature-dead** | Compiled out or no-op under `certora` / `cfg(test)` | Harness / ADR-0017 |
| **D6 Dead fill** | Map populated then **never `cached_*`** on that invocation (debt-free skip, etc.) | Budget; fail-closed if a later reader is added without warm-up |

A100’s job is this classification. **D0 is empty** for maps. D1–D6 are the residuals.

---

## 1. Field inventory vs A086 (type-level: all live)

```25:37:contracts/controller/src/context/mod.rs
pub(crate) struct Cache {
    env: Env,
    token_prices: Map<Address, PriceFeedRaw>,
    market_indexes: Map<HubAssetKey, MarketIndexRaw>,
    pool_address: Option<Address>,
    pool_sync_data: Map<HubAssetKey, PoolSyncData>,
    spoke_usage: Option<SpokeUsageContext>,
    spoke_config: Option<SpokeConfig>,
    spoke_assets: Map<HubAssetKey, SpokeAssetConfig>,
    verified_hubs: Map<u32, bool>,
    supply_updates: Vec<EventDepositDelta>,
    debt_updates: Vec<EventBorrowDelta>,
}
```

| Field | Production writer | Production reader | D0 unused map? |
|---|---|---|---|
| `env` | constructor | `env()`, all helpers | No |
| `token_prices` | `fetch_prices` (`load_markets` / prefetch) | `cached_price` | No |
| `market_indexes` | `fetch_market_indexes`, `cached_market_index` miss, `put_market_index` | `cached_market_index` hit | No |
| `pool_address` | `cached_pool_address` miss | same fn hit; all FFI | No |
| `pool_sync_data` | `cached_pool_sync_data` miss | **intended** same fn hit — **never in prod** | **No map, yes D1** |
| `spoke_usage` | `ensure_spoke_context` | apply / persist | No (A091) |
| `spoke_config` | `spoke_config` miss | hit + `active_spoke` | No (A089) |
| `spoke_assets` | `cached_spoke_asset` listed miss | hit / flags / caps / restamp | No (A089) |
| `verified_hubs` | success `set(..., true)` | **`contains_key` only** | Map live; **bool D2** (A090) |
| `supply_updates` / `debt_updates` | `record_*` | `emit_position_batch` | No (A092) |

Constructors always `Map::new` / `None` / empty `Vec` (`new_view`; `new` delegates after TTL renew). No field is left uninitialized. **A086 “all maps live” is true at this granularity.**

---

## 2. Method liveness (no orphan APIs in production WASM)

Every `pub(crate)` Cache method under `context/` has a production caller except `set_prices`.

| Method | Callers (production `src`) | Dead? |
|---|---|---|
| `new` / `new_view` | All mutators / views listed in §3 | Live |
| `env` | `prefetch_strategy_prices` only | Live (thin) |
| `load_markets` | `risk/totals.rs` only (`sum_debt_usd`, `calculate_ltv_collateral_wad`, `calculate_account_risk_totals_body`) | Live |
| `require_hub_active` | `require_listed_unhalted_config` only | Live (A090) |
| `cached_pool_address` | Widespread FFI / recipient ban | Live; **hit is the memo that actually pays** |
| `cached_pool_sync_data` | `flash_position` (once); views collateral/borrow amount (once each) | **Miss live; hit D1** |
| `ensure_spoke_context` | Internal spoke accessors | Live |
| `reset_spoke_context` | Keeper threshold batch only | Live (A091) |
| `require_spoke_usage_context` | `apply_spoke_{entry,exit}` only | Live wrapper |
| `cached_spoke_asset` / `require_spoke_asset*` / `require_listed_active_config` / `spoke_config` / `active_spoke` | Positions, strategies, liq, keepers, account create | Live (A089) |
| `apply_spoke_{entry,exit}` / `persist_spoke_usage` | Legs / finalize / bad-debt | Live (A091) |
| `fetch_prices` / `cached_price` | `load_markets`, prefetch; risk + liq math | Live |
| `set_prices` | Unit tests only | **D5** `#[cfg(test)]` |
| `put_market_index` | `merge_*_leg` supply/debt | Live (A094) |
| `fetch_market_indexes` | `load_markets`; `get_all_market_indexes_detailed` | Live in WASM; **D5 no-op under `certora`** |
| `cached_market_index` | Risk, liq math, views, keeper ParamUpd, `get_market_index` | Live (hit + lazy miss) |
| `record_*` / `emit_position_batch` | Merges, liq, keeper; finalize / keeper emit | Live; empty emit D4 no-op |

**No dead production function** to delete. `set_prices` must stay test-gated (ADR-0017): it is the only way `fetch_prices_skips_already_cached_assets` avoids a real aggregator.

`cached_price(&mut self)` never writes — **D6 hygiene** (`&self` would match the contract).

---

## 3. Entrypoint occupancy matrix (D3)

`Y` = field can become non-empty on a successful call. `—` = stays constructor-empty for that Cache’s life. `P` = possible on a subpath (e.g. only if the account has debt).

| Entrypoint | prices | indexes | pool addr | sync | spoke_* | verified | events |
|---|---|---|---|---|---|---|---|
| `process_supply` (debt-free) | — / `P` restamp HF | `put` | Y | — | Y | Y | Y |
| `process_borrow` / withdraw w/ debt | Y (`load_markets`) | `put` + load | Y | — | Y | borrow: Y | Y |
| `process_repay` | — | `put` only | Y | — | flags/usage | — | Y |
| `process_liquidation` / socialize | Y | Y | Y | — | Y | — (exit) | Y |
| Strategies except `flash_loan` | Y prefetch | `put` + finalize load | Y | — (`flash_position`: **Y once**) | Y | Y | Y |
| **`process_flash_loan`** | — | — | **Y only** | — | — | — | — |
| **`update_indexes` / `claim_revenue` / `recapitalize`** | — | — | **Y only** | — | — | — | — |
| **`upgrade_liquidity_pool_params`** | — | — | **Y only** | — | — | — | — |
| `update_account_threshold` LtvOnly | — | lazy if ParamUpd | Y? via index fetch | — | assets | — | `P` / empty emit |
| FullTuple threshold | Y at HF | lazy then bulk no-op | Y | — | assets | — | `P` |
| `health_factor` indebted | Y | Y | via indexes | — | — | — | — |
| **`health_factor` missing / debt-free** | **— (Cache still allocated)** | — | — | — | — | — | — |
| collateral / borrow amount views | — | lazy 1 | via index+sync | **Y once** | — | — | — |
| **`get_all_market_indexes_detailed`** | **— (soft quotes off-Cache)** | bulk then hit | via bulk | — | — | — | — |
| liq estimate view | Y | Y | Y | — | Y | — | — |
| `ltv_collateral_in_usd` | Y | Y | Y | — | assets (restamp) | — | — |

**Address-only mutators** (`flash_loan`, three keepers, param upgrade) are the sharp D3 cluster: `Cache::new` still builds seven maps + two Vecs + three Options to memoize one `Address`. TTL renew is owned by **A093**; A100 notes the **allocation shape** is oversized for those verbs.

`process_repay` **writes** `market_indexes` via `put_market_index` and **never** `cached_market_index` (no post-pool HF). That is **D6 dead fill** of the index map: the mutation index is consumed from `LegOutcome` / the event payload, not from a later Cache get. The `put` still matters on paths that *do* load markets afterward (borrow, withdraw, strategies, liq).

---

## 4. Dead hit paths (D1) — the main unused-memo result

### 4.1 `pool_sync_data` — fill-once map that never hits

```20:28:contracts/controller/src/context/pool.rs
pub(crate) fn cached_pool_sync_data(&mut self, hub_asset: &HubAssetKey) -> PoolSyncData {
    if let Some(data) = self.pool_sync_data.get(hub_asset.clone()) {
        return data;
    }
    let pool_addr = self.cached_pool_address();
    let data = fetch_pool_sync_data(&self.env, &pool_addr, hub_asset);
    self.pool_sync_data.set(hub_asset.clone(), data.clone());
    data
}
```

Production `cached_pool_sync_data` sites under `contracts/controller/src`:

| Site | Keys | Calls per Cache | Hit reachable? |
|---|---|---|---|
| `flash_position` `is_flashloanable` | debt hub | **1** | No |
| `collateral_amount_for_hub_asset` | one hub | **1** | No |
| `borrow_amount_for_hub_asset` | one hub | **1** | No |

Certora rules (`strategy_rules`, `flash_loan_rules`) also take a **single** sync read.

Therefore:

- The `get` arm is **production-dead**.
- `.set` stores a value **nobody fetches again**.
- A088’s “stale after mutation if you re-read” residual is **counterfactual** on today’s graph (A088 already: check is pre-leg). A100 adds: even a *same-value* second read does not exist.

**Implication:** today’s behavior equals `fetch_pool_sync_data` without a Map. The Map is a **latent API** for a second reader that was never added. Do not treat it as proven batching.

Contrast **`pool_address`**: many FFI sites per invocation → **hit is live** and is the memo that saves instance-storage re-reads.

### 4.2 `cached_market_index` miss vs bulk (D4, path-local)

| Path | Miss (lazy 1-key bulk)? | Hit? |
|---|---|---|
| After `load_markets` / `fetch_market_indexes` | **Dead on that path** (already filled) | Live |
| `get_market_index` view; amount views | **Live** (primary) | N/A second |
| Keeper ParamUpd before HF | **Live N+1** (A087) | later HF hits |
| After `put_market_index` then risk totals | Miss dead for that hub | Live (A094) |
| `get_all_market_indexes_detailed` | Miss dead after bulk | Live (loop) |

Lazy miss is **not globally dead**. Bulk fill is **not globally dead**. Each is dead on the other’s happy path — intended overlap (A087).

### 4.3 `token_prices` hit is live; miss-fetch does not exist

`cached_price` never lazy-fills (fail-closed panic). Hit is live on multi-position HF, prefetch-then-finalize, and liq plan→math. The “skip aggregator” arm of `fetch_prices` (`missing.is_empty()`) is live on a **second** `load_markets` (e.g. post-liq totals) and is unit-tested (`fetch_prices_skips_already_cached_assets`).

### 4.4 Spoke / hub / events hits are live

- `spoke_config` / `spoke_assets`: second asset / flags-after-listing / restamp — hits live (A089).
- `verified_hubs`: bulk entry + `process_deposit` re-gate — **`contains_key` hit live** (A090). Stored `true` never read (D2).
- Event Vecs: append then emit clone — live; **empty emit** (`is_empty && is_empty`) is a live no-op (keeper unchanged accounts always call `emit_position_batch`).

---

## 5. Dead payloads (D2)

### 5.1 `PoolSyncData` — almost all fields unused at Cache consumers

`get_sync_data` returns `{ params: MarketParamsRaw, state: PoolStateRaw }`. Cache stores the whole blob.

| Consumer | Fields read from Cache |
|---|---|
| `flash_position` | `params.is_flashloanable` |
| Amount views | `params.asset_decimals` |

**Never read from Cache:** rate curve, `reserve_factor`, `flashloan_fee`, `asset_id`, **entire `state`** (`supplied` / `borrowed` / `revenue` / indexes / `cash` / `last_timestamp`).

Index/HF truth is `market_indexes` (simulate + `put`), not `sync.state.*_index` (A088/A094/A077). Storing raw unaccrued state in Cache is **dead weight** plus a footgun if a future reader uses it for valuation.

Admin listing uses **direct** `fetch_pool_sync_data` (bypasses Cache) — also not a Cache-hit.

### 5.2 `PriceFeed.timestamp`

`cached_price` converts `PriceFeedRaw` → `PriceFeed { price, asset_decimals, timestamp }`. Controller Cache consumers use `price` (risk totals, bonus weights) and `asset_decimals` (liq unscale). **`timestamp` is never read** under `contracts/controller/src` after Cache. Freshness is enforced inside the aggregator hard `prices` path (A065/A087), not by Cache.

### 5.3 `verified_hubs: Map<u32, bool>`

Only `contains_key` / `set(..., true)`. Value is vestigial (A090 `contains_key` footgun). **D2**, not D0 — the map keys *are* the memo.

### 5.4 `SpokeUsageContext` Map empty-but-Some

First `cached_spoke_asset` / `spoke_config` pins `spoke_usage = Some(empty Map)` (A091). Views (`ltv_collateral_in_usd`, liq estimate) pin and **never persist**. Not a leak (persist not called). Occupancy buffer is empty — **D3 for usage rows**, pin is live.

---

## 6. Feature-dead and test-only (D5)

### 6.1 Certora `fetch_market_indexes` no-op

```10:20:contracts/controller/src/spec_hooks.rs
#[cfg(feature = "certora")]
impl crate::context::Cache {
    pub(crate) fn fetch_market_indexes(
        &mut self,
        _hub_assets: &soroban_sdk::Vec<crate::types::HubAssetKey>,
    ) {
    }
}
```

Under `certora`, `load_markets` still `fetch_prices` then a **no-op** index bulk. Subsequent `cached_market_index` **must** take the lazy miss (or harness `put_market_index`). Production WASM keeps the bulk path. A087 already: harness concern, not deployable dead code.

### 6.2 Soft quotes vs Cache prices (views)

```155:162:contracts/controller/src/views.rs
    let mut cache = Cache::new_view(env);
    cache.fetch_market_indexes(hub_assets);
    let assets = unique_hub_tokens(env, hub_assets);
    let statuses = if assets.is_empty() {
        Map::new(env)
    } else {
        fetch_prices_status(env, &assets)
    };
```

`token_prices` is allocated and **never filled** on this view. Observability correctly uses soft `quotes` (A087/A095). Not a money-path dual Cache. **D3** for `token_prices` on this entrypoint only.

### 6.3 `health_factor` allocates then discards

```30:42:contracts/controller/src/views.rs
pub(crate) fn health_factor(env: &Env, account_id: u64) -> i128 {
    let mut cache = Cache::new_view(env);
    match storage::try_get_account(env, account_id) {
        Some(account) if !account.debt_free() => risk::calculate_account_risk_totals(...)
            .health_factor
            .raw(),
        _ => i128::MAX,
    }
}
```

Missing account and debt-free accounts pay **full empty-Cache construction** for `i128::MAX`. `can_be_liquidated` inherits this. **D3 + wasted ctor**; not a correctness bug (A008 views still write-free).

### 6.4 Prefetch then debt-free skip (D6)

`prefetch_strategy_prices` always fills `token_prices`. `require_post_pool_risk_gates` returns immediately if `account.debt_free()` — **no `cached_price`**. Closing all debt via `repay_debt_with_collateral` (or a collateral swap that leaves no borrow) can leave a populated price map unread. Fail-closed if a future gate reads `cached_price` without prefetch; today it is **unused fill**, not a skip of a failed oracle check (A099).

---

## 7. Overlapping warm-up paths (not dead code)

These look redundant; both arms are occupied on some graph:

| Pair | Why both exist |
|---|---|
| `fetch_market_indexes` vs `cached_market_index` miss | Bulk at HF / detailed view; lazy at single-asset view and keeper ParamUpd |
| `fetch_prices` vs `cached_price` | Batch + fail-closed read; no lazy price (ADR-0005) |
| `put_market_index` vs simulate fill | Mutation overwrite vs untouched-hub simulate (A094) |
| `config::require_hub_active` vs `Cache::require_hub_active` | Strategies pre-check before `Cache::new`; memo only after Cache exists (A090) |
| `fetch_pool_sync_data` (config) vs `cached_pool_sync_data` | Admin live; Cache preflight — **Cache side never hits** |
| `storage::get_pool` vs `cached_pool_address` | Some admin (`create_liquidity_pool`, `get_pool_address` view) skip Cache |

Do **not** delete the lazy index path: Certora and `get_market_index` depend on it.

---

## 8. What is **not** dead (anti-findings)

| Claim | Why rejected |
|---|---|
| Delete `pool_sync_data` field as unused | Miss path is live; three readers depend on the fetch. Only the **hit** is dead |
| Delete `verified_hubs` | Hit skips repeat storage reads on multi-asset entry (A090/A099) |
| Delete event buffers | Append + emit is the observational SoT drain (A092/A033) |
| `spoke_assets` unused on flash_loan ⇒ unused type | Live on every position/strategy/liq path |
| `reset_spoke_context` on first keeper account is redundant | Required for account 2+ mixed spokes (A083/A091); first reset of empty is cheap |
| Certora no-op means production bulk fetch is dead | Opposite feature flags |
| Empty `load_markets([])` is a bug | Cheap no-op; some callers pass empty sibling maps by design (`total_collateral_in_usd`) |

---

## 9. Tests and formal coverage

| Claim | Coverage | Gap |
|---|---|---|
| Price miss → fetch → `cached_price` | Unit `fetch_prices_populates_cache` | — |
| Price hit skips FFI | Unit `fetch_prices_skips_already_cached_assets` | — |
| Index bulk first-pass | **Missing** (A087) | P4 |
| `cached_pool_sync_data` second get returns memo | **Missing** (would document D1) | Optional PIN |
| `verified_hubs` second call skips storage | **Missing** (A090) | P4 |
| `health_factor` debt-free does not FFI | Implied by early return; **no** assert Cache unused | Optional |
| Certora index bulk | Intentionally no-op; lazy / `put` | Documented |
| Sync payload field subset | None | Docs only |

A108’s high-severity backlog does **not** need A100 artifacts; these are hygiene PINs.

---

## 10. Threat / misuse (none live)

| # | Attempt | Outcome |
|---|---|---|
| 1 | Rely on `pool_sync_data` hit after a leg | Unreachable today; would be A088 stale if added |
| 2 | Read `sync.state.borrow_index` from Cache for HF | No such reader; would bypass `put_market_index` |
| 3 | Treat `verified_hubs` value `false` as recorded failure | Not written; `contains_key` would skip storage (A090 Critical *if introduced*) |
| 4 | Assume detailed view warmed `token_prices` | False — soft quotes; money paths use hard `fetch_prices` |
| 5 | Delete lazy `cached_market_index` as dead after `load_markets` | Breaks views, keepers, Certora |
| 6 | Skip `Cache::new` on flash_loan to “remove dead maps” | Must still renew TTL (A093) and read pool address |

**Single-account max from dead paths:** none (no wrong number; at most extra fees).

**Protocol max:** none durable.

---

## 11. Cross-links

| Peer | Relation |
|---|---|
| **A086** | Type-level inventory; A100 **agrees** all fields exist for a reason; **refines** “used” into hit/payload/entrypoint |
| **A087** | Bulk vs lazy; keeper N+1; Certora no-op; detailed view soft quotes |
| **A088** | Sync fill-once correctness; A100 proves **hit unused**, so stale-hit is latent |
| **A089–A091** | Spoke maps/usage **not** dead |
| **A090 / A099** | Hub memo hit live; bool payload dead |
| **A092** | Event Vecs live; empty emit is intentional no-op |
| **A093 / A034 / A008** | `new` vs `new_view`; address-only `new` still renews TTL |
| **A094** | Index map’s important write is `put`, not unused |
| **A095** | Savings catalogue; A100 is the “saved read that never happens” slice |
| **A104** | Coverage hole A100 closed; do not elevate Wave-6 ranking |

**Agreements:** A086/A104 architecture defended; no unused map to delete for safety.

**Disagreements:** None that need `disagreements/`. Nuance only: A104’s “A086 all fields appear used” is true; A100’s **D1** on `pool_sync_data` is the unused-*memo* (hit), not an unused field.

---

## 12. Remediation notes (for A110; non-blocking)

| P | Action | Closes |
|---|---|---|
| P3 | Document on `cached_pool_sync_data`: “current call sites never hit; treat as single fetch; if you add a second get after a pool mutation, invalidate or bypass” | D1 + A088 |
| P3 | Document `verified_hubs` presence-only (A090) | D2 |
| P4 | Optional: allocate `Cache` in `health_factor` only on the indebted branch | D3 view ctor |
| P4 | Optional unit: two `cached_pool_sync_data` same key, poke storage, assert sticky (PIN current D1) | Test hole |
| P5 | Optional slimmer “address-only” helper for flash_loan/keepers/param upgrade — **only if** wasm/CPU budget measured | D3 cluster |
| Anti-fix | Do not remove `pool_sync_data` Map without replacing the three readers | Miss is live |
| Anti-fix | Do not remove lazy `cached_market_index` | Views / Certora / keepers |
| Anti-fix | Do not mid-tx refresh prices to “use” prefetch on debt-free skip | ADR-0005 |

---

## 13. Verdict

1. **There is no wholly unused Cache memo map.** A086’s field list is the right type-level answer; A100 does not recommend deleting `token_prices`, `market_indexes`, spoke maps, hub memo, or event buffers.
2. **The unused-memo result is `pool_sync_data`’s hit path:** a fill-once `Map` that production never reads twice, plus a blob whose `state` and most `params` are never consumed from Cache. That is budget + a latent A088 trap, not a live bypass.
3. **Secondary hygiene:** vestigial `verified_hubs` bool; unused `PriceFeed.timestamp`; full Cache on address-only mutators; `health_factor` ctor before early return; detailed view leaving `token_prices` empty; Certora bulk-index no-op; repay `put_market_index` without later get; prefetch unused after debt-free solvency skip.
4. **Severity info, status optimization-note.** No fund-loss, no skipped failed check (A099 still holds). Fills A104’s A100 hole without changing Wave-6 ranking versus A094 / A080.

**Bottom line:** Cache memos are **sparse per entrypoint, dense per type**. Dead paths are empty allocations and a write-only sync map — clean them in docs (and optionally in view ctor / address-only helpers), not by ripping fields out of `Cache`.
