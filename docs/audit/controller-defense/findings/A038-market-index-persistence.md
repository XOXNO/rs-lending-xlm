# A038 — Market index persistence (controller Cache vs pool source of truth)

- Agent: A038
- Theme: T2 (storage mutations) / T7 (in-memory Cache); mid-tx overwrite footgun owned by A094
- Severity: info
- Status: defended
- Paths:
  - `contracts/pool/src/storage.rs:30–67` (`PoolKey::State` read/write; durable indexes)
  - `contracts/pool/src/cache/mod.rs:55–84` (pool Cache load + `commit` writes `borrow_index` / `supply_index`)
  - `contracts/pool/src/cache/report.rs:14–80` (`market_index()`, mutation DTOs)
  - `contracts/pool/src/interest.rs:16–91` (accrual + bad-debt supply writedown)
  - `contracts/pool/src/ops/market.rs:13–83` (create at RAY; `accrue` / `update_indexes`)
  - `contracts/pool/src/ops/seize.rs:18–35` (Borrow → writedown; Deposit → absorb; commits state)
  - `contracts/pool/src/lib.rs:168–174,303–317` (owner `update_indexes`; permissionless `get_bulk_indexes` simulate-only)
  - `common/src/types/controller.rs:540–565` (`ControllerKey` — **no** market-index variant)
  - `contracts/controller/src/context/mod.rs:25–74` (`Cache.market_indexes` ephemeral map; `load_markets`)
  - `contracts/controller/src/context/market_index.rs:11–42` (`put_market_index` / `fetch_market_indexes` / `cached_market_index`)
  - `contracts/controller/src/positions/{mod,supply}.rs` (`merge_*_leg` → `put_market_index`)
  - `contracts/controller/src/strategies/legs.rs:186–215` (net-settle → both merges → double put same DTO)
  - `contracts/controller/src/positions/liquidation/{mod,apply,bad_debt,math}.rs` (plan simulate; Credit/bad-debt seize without put)
  - `contracts/controller/src/keepers.rs:16–24` (`update_indexes` pool-only; Cache discarded)
  - `contracts/controller/src/views.rs:148–176` + `lib.rs:514–523` (view Cache; never durable)
  - `contracts/controller/src/spec_hooks.rs:10–20` (Certora no-op `fetch_market_indexes`)
  - `certora/controller/harness/storage.rs:74–87` (harness reads raw sync indexes, bypasses simulate)
- Defense: Durable market indexes live **only** on the pool under `PoolKey::State`. The controller never persists indexes: `ControllerKey` has no index key, and `Cache.market_indexes` is an empty Map created per entrypoint (`new` / `new_view`) and dropped when the invocation ends. Cross-transaction truth always re-enters through pool `get_bulk_indexes` (simulate) or mutation returns. Within a mutator tx, every production `merge_*_leg` / net-settle path overwrites Cache from the pool mutation DTO via `put_market_index`. Cap enforcement on entry uses the mutation DTO directly (A077/A081), not a re-read of Cache. Shared accrual math (`accrue_step` / `simulate_update_indexes`) keeps simulate views isomorphic to the write path when time is stamped the same.
- Gap: (1) **Engineering footgun (A094):** a future pool-merge helper that omits `put_market_index` leaves a pre-leg simulated index for later same-Cache HF/cap readers. Current merges put. (2) **Credit fee seize / bad-debt `seize_positions`:** pool may accrue and (on Borrow seize) write down `supply_index`, but controller never `put`s those hubs; safe today because post-liq `post_totals` run before bad-debt writedown, Credit share moves are index-independent, and same-ledger time makes fee-seize accrual a no-op relative to plan simulate. (3) **Keeper / admin `update_indexes`:** pool commits accrued indexes; controller Cache is not refreshed (and is unused after the call). (4) **Certora:** `fetch_market_indexes` is a no-op; harness `storage::market_index::get_market_index` reads raw sync state without simulate (A035) — verification epistemology, not production WASM. (5) Docs do not state the SoT sentence next to `put_market_index` in one place (A086/A088 residual).
- Impact: No path lets the controller invent, freeze, or durable-fork market indexes away from the pool. A forgotten mid-tx `put` (future) can distort **same-tx** HF/caps for touched hubs only — account/tx-local, not cross-market durable SoT corruption. Bad-debt supply writedown persists on the pool and is visible to all subsequent txs via simulate/mutations; it does not require a controller storage write. Blast radius of Cache mistakes ends when the entrypoint returns.
- Evidence: INV-IDX-01..05, INV-IDX-03 (bad debt lowers supply index on pool), INV-STOR-01 (lifecycle on real keys — indexes are pool keys), INV-ACCT share↔index formulas; ADR-0003; formulas.md interest/index sections; peers A077, A081, A086, A087, A088, A094, A098 (scope), A026/A027, A035; harness `bulk_indexes.rs`, `liquidation_accrual_timing.rs`, `bad_debt_index.rs`; Certora `index_rules.rs` / rate-index rules; SEED Cache facts.
- Opinion: **Pool is the sole durable source of truth for market indexes; controller Cache is a per-invocation projection + mutation overlay.** Wave-2 answer is clean for persistence. Keep treating `put_market_index` as mandatory after every pool position/net-settle merge (checklist with A094). Optional hygiene: document “no controller durable index; seize/bad-debt paths do not put” next to Credit/cleanup; do not add a controller index storage key.

---

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, README finding format; skim peers A077, A086, A087, A088, A094, A026, A027, A035, A039 (claim non-pollution note).
2. Confirmed `ControllerKey` has no market-index / sync-state variant; inventoried pool `PoolKey::State` fields and `commit` writers.
3. Traced every controller producer/consumer of `MarketIndexRaw`: `fetch` / `cached` / `put`, merge helpers, liquidation plan/apply/cleanup, keepers, views, strategies/net-settle.
4. Separated **durable SoT** (pool persistent) from **ephemeral Cache** (controller RAM) from **simulate projection** (`get_bulk_indexes`).
5. Cross-checked INV-IDX-* and formulas.md against write-down / accrual paths.
6. No novel critical gap: controller does not persist indexes; current merge graph refreshes Cache after position mutations.

Out of scope as primary claims: IRM correctness (IDX formal suite), forgotten-put remediation process (A094/A104/A110 RB-08), spoke-cap side selection (A081), price snapshot policy (ADR-0005 / A087).

---

## 1. Scope boundary vs peers

| Peer | Owns | This file adds |
|---|---|---|
| A077 | Cap uses pool mutation indexes | Persistence / SoT framing; Cache is not durable truth |
| A081 | Supply vs borrow field selection | Confirms entry caps use DTO, not Cache SoT |
| A086 | Cache field inventory | Deep dive on `market_indexes` lifecycle vs pool `State` |
| A087 | Bulk prefetch / batching | SoT note that bulk fill is simulate, not a write |
| A088 | `pool_sync_data` fill-once | Contrast: indexes **do** overwrite via `put`; sync does not |
| A094 | Mid-tx stale Cache if `put` omitted | A038 proves durable SoT is still pool even if Cache is wrong in-tx |
| A098 | Same-tx accrual races (Wave 6) | A038 inventories sources; defers race taxonomy |
| A026/A027 | Liq / bad-debt controller storage | Index books are pool-only; cleanup does not put Cache |
| A035 | Certora storage harness | Raw sync vs simulate divergence for index helpers |
| A039 | Claim path | Confirms claim does not pollute controller index Cache as SoT |

---

## 2. Architecture — who owns the numbers

```
                    DURABLE (cross-tx)
┌──────────────────────────────────────────────────────────┐
│  Liquidity Pool persistent storage                       │
│    PoolKey::Params(hub)  — IRM / decimals / flags        │
│    PoolKey::State(hub)   — cash, shares, revenue,        │
│                            borrow_index, supply_index,   │
│                            last_timestamp                │
│  Writers: pool Cache::commit after accrual / mutation /  │
│           seize / accrue keeper path                     │
└───────────────────────────▲──────────────────────────────┘
                            │ load / commit
                 pool in-tx Cache (per market)
                            │
        ┌───────────────────┴───────────────────┐
        │ mutation return MarketIndexRaw        │ get_bulk_indexes
        │ (post-accrual, post-leg)              │ = simulate_update_indexes(now, sync)
        └───────────────────▲───────────────────┘
                            │
┌───────────────────────────┴──────────────────────────────┐
│  Controller — NO durable index key                       │
│  ControllerKey: Pool, oracles, NFT, accounts, usage, …   │
│  Cache.market_indexes: Map<HubAssetKey, MarketIndexRaw>  │
│    • created empty in new / new_view                     │
│    • fill: fetch / lazy cached (simulate)                │
│    • overwrite: put_market_index(mutation DTO)           │
│    • discarded when entrypoint returns                   │
└──────────────────────────────────────────────────────────┘
```

**Verdict sentence:** If controller Cache and pool `State` disagree after a transaction completes, only the pool value exists; the Cache is gone. Persistence question collapses to “does the pool commit?” — yes, on every mutating pool op that lands indexes.

---

## 3. Pool durable persistence

### 3.1 Storage shape

`PoolStateRaw` (persistent under `PoolKey::State`) includes `borrow_index`, `supply_index`, `last_timestamp`, plus books (`supplied`, `borrowed`, `revenue`, `cash`). Create seeds both indexes at `RAY` (`ops/market.rs`).

`write_state` / `read_state` / `load_state` / `load_sync_data` are the only durable index I/O on the pool. Controller code never calls these; it only sees DTOs and views via FFI.

### 3.2 Commit paths that land indexes

| Pool path | Accrues? | Index effect | Returns index to controller? |
|---|---|---|---|
| deposit / withdraw / borrow / repay / strategy / flash / net_settle | `synced_market` → `global_sync` | May raise borrow/supply via `accrue_step`; fee paths may mint revenue shares | Yes — `PoolPositionMutation` / strategy / net-settle DTOs include `market_index()` |
| `update_indexes` (owner; controller keeper/admin) | Yes | Writes accrued indexes | No DTO (void) |
| `replace_rate_model` / `update_params` | Accrue then replace params | Accrued indexes committed before model swap | No index DTO to controller mutators that care |
| `seize_positions` Borrow | Yes then writedown | `apply_bad_debt_to_supply_index` may **lower** supply index (INV-IDX-03); burns debt | Snapshot event only — **no** `MarketIndexRaw` to controller |
| `seize_positions` Deposit | Yes then absorb | Indexes unchanged by absorb (shares → revenue); accrual may have bumped | Same — no controller put |
| `get_bulk_indexes` | Simulate only | **No write** | Returns projected `MarketIndexRaw` |
| `get_sync_data` | No simulate | Raw committed indexes (may be behind ledger time) | Used for sync memo / harness; not primary risk fill |

Shared arithmetic: `interest::accrue_chunk` and `simulate_update_indexes` both use `accrue_step`, so a simulate at `now` matches a successful accrue commit at the same stamped time (pool tests + Certora iso rules).

### 3.3 What the controller never writes

`ControllerKey` enumerates protocol pointers, hubs/spokes, usage, account meta/positions/delegates — **not** indexes. Grep of controller storage modules shows no `borrow_index` / `supply_index` persistent set. Account positions store **scaled shares** only; USD/risk revaluation always needs a live index from Cache←pool.

---

## 4. Controller Cache — ephemeral overlay

### 4.1 Lifecycle

```39:62:contracts/controller/src/context/mod.rs
    pub(crate) fn new(env: &Env) -> Self {
        storage::renew_controller_instance(env);
        Self::new_view(env)
    }
    // ...
            market_indexes: Map::new(env),
```

- `new` / `new_view`: empty `market_indexes`.
- No `storage::set_*` for indexes on finalize; `finalize_position_flow` persists positions + spoke usage + events only.
- Next entrypoint always starts cold → must fetch or receive mutation DTOs again.

### 4.2 Three APIs

| API | Semantics | Durable? |
|---|---|---|
| `fetch_market_indexes` | Uncached hubs → one `get_bulk_indexes` → insert | No — simulate projection |
| `cached_market_index` | Hit map else single-key bulk fetch + insert | No |
| `put_market_index` | Unconditional overwrite from caller-supplied raw | No — RAM only; should be post-mutation truth |

`fetch_market_indexes` skips keys already present (`collect_uncached_keys`). Therefore a prior `put` **wins** over a later bulk fetch for that hub — intentional post-leg freshness. A prior simulate fill **blocks** re-simulate until `put` overwrites — the A094 hazard if a leg mutates without putting.

Certora builds replace `fetch_market_indexes` with a no-op (`spec_hooks.rs`); rules seed via `put_market_index` or ghost summaries.

### 4.3 Who reads Cache indexes

| Consumer | Purpose | Expects |
|---|---|---|
| `risk/totals.rs` | HF / collateral / debt USD | Simulate for untouched hubs; put for touched |
| `liquidation/math.rs` | Plan sizing | Pre-leg simulate via plan `load_markets` |
| `keepers` threshold ParamUpd events | Observational index stamp | Lazy/simulate |
| Views `get_market_index` / detailed | Off-chain | Fresh `new_view` + simulate |
| Entry spoke caps (`apply_spoke_entry`) | Cap scaled | **Mutation DTO** (`outcome.market_index`), not Cache re-read (A077/A081) |

---

## 5. Mutation → `put_market_index` inventory (production)

Every path that merges a pool **position** or **net-settle** outcome into an account goes through helpers that put:

| Helper | File | Put? |
|---|---|---|
| `merge_debt_leg` | `positions/mod.rs` | Yes — after `apply_leg_usage` |
| `merge_supply_leg` | `positions/supply.rs` | Yes |
| `merge_withdraw_leg` | `positions/supply.rs` | Yes (before exit usage) |
| Strategy net-settle | `strategies/legs.rs` | Yes ×2 (withdraw + debt merges; same `result.market_index`) |

Callers: ordinary supply/withdraw/borrow/repay batches, liquidation Transfer repay/seize batches, strategy legs that use merges, flash-position debt/collateral merges that funnel through the same helpers.

`LegOutcome::from(&PoolPositionMutation)` copies `mutation.market_index` — pool-reported post-sync indexes, not caller inputs (A082 sibling).

---

## 6. Pool index mutations **without** controller `put`

These are the interesting SoT edges for A038 (not “missing durable controller writes” — there should be none).

### 6.1 Keeper / admin accrue

```16:24:contracts/controller/src/keepers.rs
pub(crate) fn update_indexes(...) {
    let mut cache = Cache::new(env);
    let pool_addr = cache.cached_pool_address();
    pool_update_indexes_call(env, &pool_addr, &assets);
}
```

Pool `ops::market::accrue` commits indexes. Controller Cache holds only the pool address memo and is dropped. No later risk read in that entrypoint. **Correct:** no need to put.

`upgrade_liquidity_pool_params` similarly accrues then updates params; does not value positions with Cache indexes afterward.

### 6.2 Credit liquidation fee seize

`apply_liquidation_share_credit` moves scaled shares on controller books; optional `pool_seize_positions` absorbs fee shares as revenue. No `put_market_index`.

Then `process_liquidation` computes `post_totals` via `calculate_account_risk_totals` → `cached_market_index`:

- Debt hubs: refreshed by repay `merge_debt_leg` puts.
- Collateral hubs: still plan-time simulated indexes (unless also repaid same hub).
- Same ledger timestamp ⇒ simulate ≡ accrued; absorb does not change indexes.
- Credit seize itself is share-denominated and **immune to index drift** between plan and apply (documented in `apply.rs`).

Residual: if a future host ever advanced ledger time mid-invocation, Credit collateral hubs would keep pre-fee-seize Cache entries. Not a present Soroban concern; document if composite flows grow.

### 6.3 Bad-debt cleanup

`execute_bad_debt_cleanup` calls `pool_seize_positions` (Borrow → supply writedown on **pool**). No `put_market_index`.

Order in liquidation:

1. `post_totals` computed **before** cleanup (uses Cache; debt/collateral already merged for Transfer, or Credit as above).
2. Cleanup socializes using those totals as inputs to the dust gate / event USD fields.
3. Account deleted — no further HF using Cache for that account.

Standalone `clean_bad_debt` / `force_socialize`: totals from simulate, then seize writedown, then delete. Next transaction’s readers see lowered supply index from pool SoT. **INV-IDX-03 lives on the pool.**

### 6.4 Claim revenue / recapitalize / cash flash

Claim/recap do not put market indexes into controller Cache for risk (A039). Cash flash is pool-local. No durable controller index side effects.

---

## 7. Cross-tx vs mid-tx consistency

| Scenario | Behavior | Bug? |
|---|---|---|
| Tx A accrues; Tx B views | B `new_view` → `get_bulk_indexes` projects from A’s committed state + time | No — pool SoT |
| Tx A mutates; controller Cache in A after `put` | Matches mutation return | No |
| Tx A mutates; forgotten `put`; later HF in A | Stale simulate for that hub | **A094 footgun** — not durable fork |
| Two txs concurrent on same ledger | Each sees own Cache; commits serialize per Soroban rules | Normal concurrency; A094 already notes simulate vs post-accrual across txs |
| Controller restart / new entrypoint | Empty Cache | Cannot revive stale overlay |

**Persistence conclusion:** There is no “controller index persistence bug” class that survives the entrypoint boundary. Residual risk is mid-invocation overlay discipline only.

---

## 8. Views and off-chain consumers

- `get_market_index` / `get_market_indexes_detailed`: `Cache::new_view` (no instance TTL renew) → simulate fill. Soft price status on detailed path (A065/A087). Observational; not a money path.
- Events stamp indexes from mutation DTOs or `cached_market_index` at emit time — audit trail, not SoT.
- `get_sync_data` on pool exposes raw committed indexes (may lag ledger time until accrue). Controllers that need “as of now” must use `get_bulk_indexes` or a mutating accrue — production risk paths do.

---

## 9. Formal / harness caveats (non-production)

| Mechanism | Distortion | Mitigation |
|---|---|---|
| Certora `fetch_market_indexes` no-op | Bulk warm empty | Rules `put` or lazy ghost fetch |
| Harness `storage::market_index::get_market_index` | Raw `get_sync_data.state` **without** simulate | Prefer Cache / `simulate_update_indexes` (A035); `index_rules` already routes iso/mono through real simulate |
| Ghost mutation indexes rewritten to snapshot | May hide put/forget bugs | Suite-review / A094 process residual |

None of these invent a durable controller index key.

---

## 10. Invariants mapping

| Invariant | How A038 reads it |
|---|---|
| INV-IDX-01/02 | Bounds enforced in pool/common rate math on write; controller does not store competing values |
| INV-IDX-03 | Supply writedown only via pool `apply_bad_debt_to_supply_index` + `commit` |
| INV-IDX-04/05 | Accrue vs simulate share `accrue_step`; keeper accrue commits SoT |
| INV-STOR-01 | Index lifecycle is pool persistent TTL (`renew_market`) + commit; controller Cache is not “persistent state” |
| Share valuation formulas | Always `scaled × index`; index must come from pool projection/mutation |

---

## 11. Gaps, residuals, non-findings

| Item | Severity | Owner |
|---|---|---|
| Forgotten `put_market_index` on new merge | low (future) | A094 / A104 / A110 RB-08 |
| Credit / bad-debt seize no put | info — call-graph safe today | A038 hygiene note; A098 if mid-tx time model changes |
| Docs: single SoT sentence near Cache APIs | info | A086/A088 doc residual |
| Certora raw sync helper | info (epistemology) | A035 |
| Adding a controller durable index key | **anti-pattern** | Do not “fix” by dual-writing |

**Non-findings:** Controller does not silently fork indexes across txs; views do not write; claim does not establish Cache SoT; pool simulate-only bulk API cannot corrupt durable state.

---

## 12. Remediation / hardening (optional)

1. **Document** in `context/market_index.rs` rustdoc: “Pool `PoolKey::State` is durable SoT; this map is per-invocation only; after any pool position/net-settle mutation call `put_market_index`; seize/accrue-only FFIs do not update this map.”
2. Keep **static checklist / lint** with A094: every new `merge_*` / pool position FFI consumer must put (+ `apply_leg_usage`).
3. If a future path re-values HF **after** `seize_positions` writedown in the same Cache, either `put` from a returned snapshot or re-fetch (clear map entry) for that hub — none required today.
4. Do **not** add `ControllerKey::MarketIndex`.

---

## 13. Verdict

**Market index persistence is defended.** The pool’s persistent market state is the only source of truth that survives an entrypoint. The controller Cache is an ephemeral simulate-and-overwrite overlay used for coherent mid-tx risk and events. Production position merges refresh that overlay from pool mutation DTOs; paths that mutate pool indexes without returning DTOs either do not re-read Cache for money decisions afterward or only re-read under same-timestamp equivalence.

Residual work is documentation plus the already-tracked A094 merge-checklist footgun — not a missing controller durable write or a live cross-tx SoT split.
