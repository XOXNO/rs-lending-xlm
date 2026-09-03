# A089 — `spoke_config` / `spoke_assets` Cache memoization

- Agent: A089
- Theme: T7 (also T5 pin coupling; T4 listing adjacency)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/context/mod.rs:25-61` (`Cache` fields; `spoke_config: Option<SpokeConfig>`; `spoke_assets: Map<HubAssetKey, SpokeAssetConfig>`; constructors)
  - `contracts/controller/src/context/spoke.rs:12-143` (`ensure_spoke_context`, `reset_spoke_context`, `cached_spoke_asset`, `require_spoke_asset*`, `require_listed_active_config`, `spoke_config`, `active_spoke`, `apply_spoke_entry`)
  - `contracts/controller/src/storage/spoke.rs:11-54` (`get_spoke` / `get_spoke_asset` / `set_spoke_asset` / `remove_spoke_asset`)
  - `common/src/types/controller.rs:94-124` (`SpokeConfig`, `SpokeAssetConfig` payloads)
  - `contracts/controller/src/keepers.rs:86-91,183-186` (sole production `reset_spoke_context`; `cached_spoke_asset` in threshold sync)
  - `contracts/controller/src/account.rs:55-62` (`active_spoke` / `spoke_config` on create)
  - `contracts/controller/src/positions/{mod,supply,debt}.rs` (listing / flags / stamp consumers)
  - `contracts/controller/src/positions/liquidation/{plan,apply}.rs` (curve via `spoke_config`; Credit stamp via `require_spoke_asset`)
  - `contracts/controller/src/risk/params.rs:40-59` (`restamp_listed_supply_ltv`)
  - `contracts/controller/src/strategies/flash_position.rs:80-158,254-260` (pre-callback listing warm; post-callback deposit/finalize on same Cache)
  - `contracts/controller/src/config/{spoke,asset}.rs` (admin writers bypass Cache — direct storage)
  - `contracts/controller/src/lib.rs:527-536,642-723` (views read storage directly; admin `#[only_owner]` mutators)
  - `contracts/controller/tests/spoke.rs:42-116` (unit coverage of require_spoke_asset; twin-spoke via **two** Caches)
- Defense: Untagged memos (`spoke_config` as bare `Option`; `spoke_assets` keyed by `HubAssetKey` only) are safe **because** every accessor calls `ensure_spoke_context` first, which pins exactly one `spoke_id` for the Cache lifetime (or until reset). Mismatch panics `#310 SpokeMismatch`. `reset_spoke_context` clears usage + config + assets together — the only production caller is the mixed-spoke keeper batch, which never buffers usage. Misses for unlisted assets are **not** negatively cached (re-fetch). Hits are fill-once for the pinned spoke. Durable keys always include `spoke_id` (A028). Admin listing/curve writers go through storage, not Cache. Public views read storage directly. Ordinary position/strategy/liquidation flows are single-spoke (account bind / Credit same-spoke).
- Gap: (1) **Fill-once / no overwrite API** — after a positive `spoke_assets` / `spoke_config` hit, mid-invocation storage edits (admin `edit_asset_in_spoke` / `set_spoke_asset_flags` / `set_spoke_liquidation_curve` / `remove_spoke` / delist) are invisible to that Cache. Admin mutators are intentionally ungated under the flash flag (A007). A nested owner-authorized admin call during `flash_position`’s callback can therefore leave post-callback `process_deposit` / `apply_spoke_entry` / `restamp_listed_supply_ltv` / flag helpers on **stale** listing or curve. Unprivileged attackers cannot write those keys. (2) **No dedicated unit test** that same-Cache `ensure(1)` then `ensure(2)` panics, nor that reset then load spoke 2 returns spoke-2 LTV (twin-spoke unit uses two Caches). (3) Module docs do not state the “untagged memo ⇔ pin” contract next to the fields (A083 §10 / A104 hole). (4) Side effect: any config/asset read creates an empty `SpokeUsageContext` for that spoke even when usage is unused — harmless today (`persist` no-ops if untouched).
- Impact: **No unprivileged cross-spoke listing bleed, wrong-curve liquidation, or silent fund theft** under current graphs. Wrong-spoke asset config would require defeating the pin (panic) or skipping `ensure` (no production path). Stale-after-admin-edit blast radius (governance/trust scenario only): within one invocation, overstated caps (INV-HALT-03 capacity), stale halt flags (INV-HALT-02 accept/reject), stale liquidation curve params, or stale LTV restamp — account/tx-local policy skew, not durable SoT corruption (storage remains truth; next invocation fetches fresh). Practical impact under honest owner ≈ **negligible**. Severity **info**.
- Evidence: Exhaustive grep of `spoke_config` / `cached_spoke_asset` / `require_spoke_asset*` / `active_spoke` / `reset_spoke_context` / `ensure_spoke_context` under `contracts/controller/src`; peers A083 (pin/isolation), A086 (inventory), A088 (fill-once analogy), A063/A064 (listing stack), A007 (admin under flash), A028 (durable keys), A104 §7 A089 hole; INV-AUTH-06, INV-HALT-02/03; ADR-0008/0009; unit `tests/spoke.rs`; harness mixed-spoke keeper (A083).
- Opinion: **Confirm A083 §10 / A104 adjacency:** architecture is defended. Untagged memos are a deliberate read-saving trade under a hard pin, not a latent mix-up bug. Document the pin contract and the fill-once rule beside `put_market_index`. Do not key `spoke_assets` by hub alone without the pin. Do not add mid-flow `reset_spoke_context` without persist. Optional P3: unit tests for pin panic + reset-clears-assets; optional checklist “never rely on Cache spoke memos after owner mutates listing in the same invocation.” Re-run if a multi-spoke mutation entrypoint appears.

---

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, README finding format, `AGENT_MANIFEST` Wave 6 (A089), peers **A083**, **A086**, **A088**, **A104**, adjacency **A063**, **A064**, **A007**, **A028**, **A078**.
2. Read `context/{mod,spoke}.rs`, storage spoke accessors, `SpokeConfig` / `SpokeAssetConfig` types, admin config writers, all production consumers.
3. Enumerated **every** production call site of `ensure_spoke_context`, `reset_spoke_context`, `spoke_config`, `active_spoke`, `cached_spoke_asset`, `require_spoke_asset*`, `require_listed_active_config`.
4. Classified hit/miss/negative-cache behavior; ordered memos against pool legs and flash callbacks; checked whether admin can invalidate storage under a live Cache.
5. Attack scenarios: cross-spoke hub-key collision, missing reset, stale flags/caps/curve, Credit receiver, keeper batch, view vs mutator.
6. No production Rust edited. No git operations (COORDINATION).

No novel Critical/High. Agrees with A083 (untagged memos correct iff pin) and A104’s provisional “Spoke pin / config / assets defended.” Residual class matches A088 fill-once incomplete invalidation (governance-trust, not unprivileged bypass).

---

## 1. Memo primitives

### 1.1 Fields (untagged by design)

```25:37:contracts/controller/src/context/mod.rs
pub(crate) struct Cache {
    env: Env,
    // ...
    spoke_usage: Option<SpokeUsageContext>,
    spoke_config: Option<SpokeConfig>,
    spoke_assets: Map<HubAssetKey, SpokeAssetConfig>,
    // ...
}
```

| Field | Key in RAM | Durable SoT key | Pin coupling |
|---|---|---|---|
| `spoke_config` | none (`Option`) | `ControllerKey::Spoke(spoke_id)` | Must match `spoke_usage.spoke_id` |
| `spoke_assets` | `HubAssetKey` only | `ControllerKey::SpokeAsset(spoke_id, HubAssetKey)` | Same — hub key alone is **insufficient** without pin |
| `spoke_usage` | embeds `spoke_id` in context | `ControllerKey::SpokeUsage(spoke_id, HubAssetKey)` | Is the pin |

Constructors (`new` / `new_view`) leave `spoke_config: None` and empty `spoke_assets`. Per-invocation only — dropped at end of entrypoint.

### 1.2 Pin and reset

```12:29:contracts/controller/src/context/spoke.rs
pub(crate) fn ensure_spoke_context(&mut self, spoke_id: u32) {
    if let Some(ctx) = &self.spoke_usage {
        assert_with_error!(
            &self.env,
            ctx.spoke_id() == spoke_id,
            SpokeError::SpokeMismatch
        );
        return;
    }
    self.spoke_usage = Some(SpokeUsageContext::new(&self.env, spoke_id));
}

pub(crate) fn reset_spoke_context(&mut self) {
    self.spoke_usage = None;
    self.spoke_config = None;
    self.spoke_assets = Map::new(&self.env);
}
```

Properties:

1. First spoke-scoped access creates `SpokeUsageContext` for that id (even if the caller only wanted config/assets).
2. Later different id → `#310` fail-closed.
3. Reset clears **all three** spoke memos; there is no API that clears assets while leaving a stale pin (or vice versa).
4. Correctness of untagged config/assets **depends entirely** on (1)–(3). Removing the pin while keeping hub-only keys would be a Critical design regression (A083 §10).

### 1.3 `spoke_config` — fill-once singleton per pin

```86:94:contracts/controller/src/context/spoke.rs
pub(crate) fn spoke_config(&mut self, spoke_id: u32) -> SpokeConfig {
    self.ensure_spoke_context(spoke_id);
    if let Some(spoke) = &self.spoke_config {
        return spoke.clone();
    }
    let spoke = storage::get_spoke(&self.env, spoke_id);
    self.spoke_config = Some(spoke.clone());
    spoke
}
```

| Property | Behavior |
|---|---|
| Miss | `storage::get_spoke` → panic `#300 SpokeNotFound` if absent |
| Hit | Clone memo; **no** re-read; does not re-embed `spoke_id` in the value |
| Invalidate / overwrite | **Only** via `reset_spoke_context` (clears to `None`) |
| Deprecation gate | `active_spoke` asserts `!is_deprecated` on the memoized (or freshly loaded) row |

Payload (`SpokeConfig`): `is_deprecated`, liquidation curve WADs/BPS (`liquidation_target_hf_wad`, `hf_for_max_bonus_wad`, `liquidation_bonus_factor_bps`).

### 1.4 `cached_spoke_asset` — fill-once per hub under pin

```40:52:contracts/controller/src/context/spoke.rs
pub(crate) fn cached_spoke_asset(
    &mut self,
    spoke_id: u32,
    hub_asset: &HubAssetKey,
) -> Option<SpokeAssetConfig> {
    self.ensure_spoke_context(spoke_id);
    if let Some(cfg) = self.spoke_assets.get(hub_asset.clone()) {
        return Some(cfg);
    }
    let loaded = storage::get_spoke_asset(&self.env, spoke_id, hub_asset)?;
    self.spoke_assets.set(hub_asset.clone(), loaded.clone());
    Some(loaded)
}
```

| Property | Behavior |
|---|---|
| Miss + listed | Load `SpokeAsset(spoke_id, hub)`; insert into map; return `Some` |
| Miss + unlisted | Return `None`; **do not** insert (no negative cache) |
| Hit | Return map entry; **no** re-fetch |
| Invalidate | Reset only (new empty map); no per-hub `remove` / `put_spoke_asset` helper |
| Require wrapper | `require_spoke_asset_config` → `#307 AssetNotInSpoke` on `None` |

Payload (`SpokeAssetConfig`): verb bits, halt flags (`paused` / `frozen` / `no_seize`), risk BPS, `supply_cap` / `borrow_cap`.

**Negative-cache absence is load-bearing for liveness:** `enforce_spoke_asset_flags` and `restamp_listed_supply_ltv` treat `None` as “delisted → skip / tolerate.” If a miss were sticky-`None`, a same-Cache admin **add** after a speculative miss would stay invisible; today a later miss re-reads storage. Positive hits remain sticky (see §5).

### 1.5 Accessor stack (all pin first)

| Accessor | Ensures pin? | Storage on miss | Panic / soft miss |
|---|---|---|---|
| `require_spoke_usage_context` | yes | (usage lazy inside context) | InternalError if missing after ensure |
| `cached_spoke_asset` | yes | `get_spoke_asset` | `None` |
| `require_spoke_asset_config` | via cached | — | `#307` |
| `require_spoke_asset` | via require config | — | `#307` + `AssetConfig` view |
| `require_listed_active_config` | via active + require | — | `#301` then `#307` |
| `spoke_config` | yes | `get_spoke` | `#300` |
| `active_spoke` | via spoke_config | — | `#301` if deprecated |
| `apply_spoke_entry` | via require config + usage | listing for caps | `#307` / cap errors |

There is **no** production path that reads `self.spoke_assets` / `self.spoke_config` without going through these helpers.

---

## 2. Why untagged keys are safe (defense layers)

Same four-layer model as A083, specialized to config/assets:

| Layer | Mechanism | Failure if removed |
|---|---|---|
| **L1 Durable** | `Spoke` / `SpokeAsset` keys include `spoke_id` | Same hub on two spokes would share one listing row |
| **L2 Pin** | `ensure` + joint reset | Hub-keyed RAM map serves spoke A’s LTV/caps/flags for spoke B |
| **L3 Account bind** | `AccountMeta.spoke_id` immutable; merges pass `account.spoke_id` | Caller `spoke_id` could select foreign listing while mutating another account |
| **L4 Entry/Credit gates** | `require_spoke_match`; Credit receiver spoke equality | Wrong-regime risk params / curve |

Today L1–L4 hold on inventoried paths (A083 deep-dive). A089’s unique claim: **L2 is what makes the RAM keying choice correct.**

---

## 3. Exhaustive production consumers

### 3.1 Who calls `reset_spoke_context`

**Exactly one** production site:

```86:91:contracts/controller/src/keepers.rs
let mut cache = Cache::new(env);
for account_id in account_ids {
    cache.reset_spoke_context();
    sync_account_thresholds(env, account_id, scope, &mut cache);
}
```

`sync_account_thresholds` uses `cached_spoke_asset(account.spoke_id, …)` only — no `apply_spoke_*`. Reset is mandatory for mixed-spoke batches; safe because usage buffer is clean (A078/A083). Liquidation, strategies, and ordinary position flows **never** reset.

### 3.2 Mutator consumers (same Cache lifetime)

| Flow | Spoke id source | Config / asset memos used | Multi-spoke? |
|---|---|---|---|
| `supply` / `withdraw` / `borrow` / `repay` | `account.spoke_id` (+ match on create paths) | listing, flags, stamp, caps via `apply_spoke_entry` | No |
| Strategies (multiply, swaps, migrate, flash_*) | single account | `require_can_*` / `require_listed_active_config` / merges / `restamp_listed_supply_ltv` | No |
| `flash_position` | account bind | Pre-callback: create/`active_spoke`, collateral supply gates, refund `require_listed_active_config`, borrow entry+caps; post-callback: `process_deposit`, finalize restamp | No (same spoke) |
| Liquidation plan | victim `account.spoke_id` | `spoke_config` → curve (**AllowDeprecated** existence only via create path elsewhere) | No |
| Liquidation apply Credit | receiver `spoke_id` (== victim) | `require_spoke_asset` to stamp new supply slot | Same spoke enforced |
| Keeper `update_account_threshold` | each account | `cached_spoke_asset`; **reset each iteration** | Yes — defended by reset |
| `create_account` / Credit(0) | caller/`AllowDeprecated` | `active_spoke` or `spoke_config` | Single id |

### 3.3 What does **not** use Cache memos

| Surface | Behavior |
|---|---|
| `get_spoke` / `get_spoke_asset` views (`lib.rs`) | Direct `storage::get_*` — always live |
| Admin `add/edit/remove` spoke asset, flags, curve, deprecate | Direct storage read/write; **no** Cache |
| Cap domain validation in `config/asset.rs` | Direct `fetch_pool_sync_data` (A088) |
| Fresh `Cache::new` / `new_view` per entrypoint | No cross-entrypoint memo bleed |

Admin cannot “poison” a user Cache from another top-level call. The only stale-admin story is **nested** admin under a live user/strategy Cache (§5).

### 3.4 Cap path couples listing memo to usage pin

```104:123:contracts/controller/src/context/spoke.rs
pub(crate) fn apply_spoke_entry(...) {
    let spoke_config = self.require_spoke_asset_config(spoke_id, hub_asset);
    let cap = side.cap(&spoke_config);
    // ...
    self.require_spoke_usage_context(spoke_id).apply_entry(...);
}
```

Same `spoke_id` argument drives both listing memo and usage context. Caps cannot silently come from spoke A’s `SpokeAssetConfig` while occupancy writes under spoke B without first defeating `ensure` (panic).

---

## 4. Ordering vs mutations (ordinary paths)

### 4.1 Single-spoke position / strategy (no admin nested)

Typical order:

1. `Cache::new`
2. Account load / create → may warm `spoke_config` / `active_spoke`
3. Entry gates → `require_listed_active_config` / flags → fills `spoke_assets`
4. Pool legs → indexes overwritten via `put_market_index` (A094); **spoke memos untouched**
5. `apply_spoke_entry` / exit → reuses listing memo for caps (consistent snapshot)
6. Optional `restamp_listed_supply_ltv` → same memo
7. Persist usage + positions; Cache dropped

Within one honest invocation, listing/curve are a **coherent snapshot** (same spirit as ADR-0005 price snapshot). Re-reading storage each time would not improve safety against concurrent admins (SAC: one invocation; nested admin is the only writer).

### 4.2 Liquidation

- Plan: `spoke_config(account.spoke_id)` once for curve; seizure/repay flags via `cached_spoke_asset`.
- Apply: same Cache; Credit stamp `require_spoke_asset(receiver.spoke_id, …)` with `receiver.spoke_id == account.spoke_id`.
- Deprecated spoke: curve still loads (`spoke_config`); new ActiveOnly exposure closed elsewhere (A063/A013).

No reset; single spoke — untagged memos correct.

### 4.3 Keeper mixed batch

Reset **before** each account clears prior `spoke_assets` / `spoke_config`. Without reset, Alice@spoke1 then Bob@spoke2 would either `#310` on ensure or (if pin were removed) serve Alice’s hub-keyed LTV for Bob — exactly the hazard the design prevents. Harness coverage: `test_update_account_threshold_mixed_spokes_batch` (A083).

---

## 5. Staleness / invalidation residual (fill-once)

### 5.1 What can change in durable storage while a Cache is live

| Admin / storage action | Affects memo field | Same Cache can already hold it? |
|---|---|---|
| `edit_asset_in_spoke` / `set_spoke_asset_flags` | `spoke_assets` entry | Yes, if listed earlier in the invocation |
| `remove_asset_from_spoke` | should become miss | Positive hit **sticks** as listed |
| `add_asset_to_spoke` after a prior miss | new listing | Miss not cached → **re-fetch sees add** |
| `set_spoke_liquidation_curve` / `remove_spoke` (deprecate) | `spoke_config` | Yes, if warmed |
| User pool legs | neither config memo | N/A |
| Usage persist | usage context only | Orthogonal (A091/A078) |

There is **no** `put_spoke_asset` / clear-hub helper analogous to `put_market_index`.

### 5.2 Flash + owner nested admin (A007 adjacency)

`ControllerAdmin` mutators do **not** call `require_not_flash_loaning` (A007 §3.3 intentional). `flash_position` keeps one Cache across:

1. Pre-guard validation (warms listing / active spoke / caps on borrow mint)
2. `with_flash_guard` → mint + **receiver callback**
3. Post-guard `process_deposit` + `strategy_finalize` (restamp)

If the transaction also carries owner authorization and the callback (or a hook it invokes) calls `edit_asset_in_spoke` / `set_spoke_asset_flags` / curve/deprecate, storage updates while RAM memos stay at pre-callback values.

| Stale direction | Effect on post-callback legs |
|---|---|
| Cap lowered in storage, high cap memoized | Possible **over-cap** entry (INV-HALT-03 capacity) |
| Cap raised | False deny — safer |
| `paused`/`frozen` set in storage, clear flags memoized | Entry/flag helpers may **accept** when storage says halt |
| Flags cleared in storage (only via edit, not ratchet flags API) | Memo may still block — safer / stuck until new tx |
| Curve tightened/loosened | Liquidation not in this path; if a future composite reused Cache, wrong bonus/target |
| Spoke deprecated in storage | Memo may still show non-deprecated for `active_spoke` hit |
| Delist after positive hit | Memo still `Some` → treat as listed for stamp/flags/caps |

**Unprivileged flash receiver alone cannot write these keys** (`#[only_owner]`). This is a governance/trust + Cache-discipline residual, parallel to A088’s mid-callback `is_flashloanable` flip note — not an open auth hole.

Post-guard **new** monetary entrypoints create a **new** `Cache::new` and do not inherit the outer memos (A007 residual is about in-memory account/strategy state, not these maps).

### 5.3 Comparison to peer memos

| Memo | Invalidation today | A089 analogue |
|---|---|---|
| `market_indexes` | `put_market_index` after legs | Spoke configs have **no** put |
| `pool_sync_data` | none (A088) | Same fill-once class |
| `token_prices` | none by design (ADR-0005) | Intentional snapshot |
| `verified_hubs` | success-only; failures not sticky (A099) | Spoke pin failures panic, not memoize false |
| `spoke_config` / `spoke_assets` | reset only | This finding |

---

## 6. Attack / misuse scenarios

| # | Attempt | If undefended | Actual outcome |
|---|---|---|---|
| 1 | Same hub-asset listed on spoke 1 (LTV 90%) and spoke 2 (LTV 50%); one Cache reads both without reset | Stamp/caps from wrong spoke | Second `ensure` → `#310`; or keeper reset clears map |
| 2 | `supply(spoke_id=B)` on account bound to A | Use B’s listing for A’s positions | `SpokeMismatch` at account guard (L3/L4) |
| 3 | Credit seize to foreign-spoke receiver | Stamp receiver with victim’s memoized listing | Receiver spoke equality + `#310` before credit |
| 4 | Keeper batch without reset | Panic or wrong LTV restamp | Reset each iteration; harness passes |
| 5 | Rely on negative cache after delist miss then re-add in same Cache | Sticky unlisted | Misses not stored; re-fetch sees add |
| 6 | Owner edits caps mid-`flash_position` callback | Post-deposit over-cap / stale flags | Possible under owner auth (residual §5); not unprivileged |
| 7 | View `get_spoke_asset` after mutator warmed Cache | View sees stale | Views use direct storage; separate invocation |
| 8 | Remove pin, keep hub-only `spoke_assets` | Systematic cross-spoke bleed | Must never ship (A083/A089 structural rule) |
| 9 | `reset_spoke_context` after `apply_spoke_entry`, before persist | Drop usage deltas | No such call on money paths; A078 footgun only |

None of 1–5 / 7–9 are live bypasses. #6 is the documented incomplete-invalidation residual.

---

## 7. Test and formal evidence

| Claim | Evidence | Gap? |
|---|---|---|
| Listed asset → `AssetConfig` conversion | Unit `require_spoke_asset_converts_listed_risk_config` | No |
| Twin-spoke different LTVs | `require_spoke_asset_reads_each_spoke_directly` uses **two** Caches | Does not prove reset-on-one-Cache |
| Unlisted → `#307` | `require_spoke_asset_panics_when_unlisted_on_spoke` | No |
| Entry without listing → `#307` via apply | `apply_supply_without_listing_panics` / borrow twin | No |
| Account spoke mismatch | Harness + unit (A083 matrix) | No |
| Mixed-spoke keeper | `test_update_account_threshold_mixed_spokes_batch` | Implies reset |
| Cache pin panic without reset | **Missing dedicated unit** | **Yes — P3** (also A083) |
| Reset clears assets then loads spoke 2 | **Missing** | **Yes — P3** |
| Stale-after-storage-edit same Cache | **Missing** (would document snapshot semantics) | Optional |
| Certora | Specs call `require_spoke_asset` under fixed `SPOKE_ID` | Does not prove cross-spoke memo non-interference |

---

## 8. Peer cross-links

| Peer | Relation |
|---|---|
| **A083** | Owns isolation; states untagged memos correct iff pin — A089 deep-dives that claim for config/assets |
| **A086** | Inventory + reset clears usage/config/assets |
| **A088** | Same fill-once incomplete-invalidation pattern for `pool_sync_data` |
| **A063 / A064** | Listing / deprecation / FreezePolicy consumers of these accessors |
| **A007** | Admin ungated under flash → enables §5 residual |
| **A028** | Durable key domain including `spoke_id` |
| **A078 / A091** | Usage lifecycle; reset drops dirty usage (not used on keeper path) |
| **A081** | Caps read from `require_spoke_asset_config` after pin |
| **A104** | Listed A089 as coverage hole; adjacency “defended” — this filing closes the hole |

No disagreement file required vs A083/A086/A104.

---

## 9. Structural rules to preserve

1. Every spoke-scoped config/asset read/write must call `ensure_spoke_context` first.
2. Cross-spoke work in one Cache must `reset_spoke_context` (after `persist_spoke_usage` if usage dirty).
3. Do **not** key `spoke_assets` by hub alone without the pin/assert.
4. Do **not** add a partial clear that leaves `spoke_assets` populated after changing pin identity.
5. Do **not** call `reset_spoke_context` between `apply_spoke_*` and `persist_spoke_usage`.
6. Prefer documenting fill-once: after warming listing/curve, do not assume mid-invocation admin edits are visible to the same Cache; if a future flow needs live flags post-admin, re-fetch or clear the hub entry explicitly.

---

## 10. Verdict

**`spoke_config` / `spoke_assets` memoization is defended** for correctness under the Cache spoke pin, joint reset, account bind, and current single-spoke money paths. Untagged RAM keys are an intentional optimization whose safety is entirely pin-shaped — and the pin fails closed.

**Residuals are hygiene / discipline only:** fill-once invisibility of nested owner listing/curve edits (esp. under flash callback); missing pin/reset unit tests; undocumented field-level contract. None demonstrate unprivileged cross-spoke bleed or fund theft via these memos.

Closes A104’s A089 coverage hole with a filed Wave-6 finding aligned to A083 §10 and A088’s invalidation narrative.
