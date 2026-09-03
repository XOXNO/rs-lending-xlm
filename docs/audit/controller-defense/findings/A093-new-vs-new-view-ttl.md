# A093 — `Cache::new` vs `Cache::new_view` TTL side effects

- Agent: A093
- Theme: T7 (Cache constructors; overlaps T2 TTL inventory A034; T1 view bounds A008)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/context/mod.rs:1–6,39–62` (`Cache::new` / `new_view` docs + impl)
  - `contracts/controller/src/storage/protocol.rs:3–6,154–205` (`renew_controller_instance` alias; `get_user` / `get_shared` touch renew)
  - `common/src/ttl.rs:11–15` (`renew_instance` → `instance.extend_ttl`)
  - `common/src/constants/shared.rs:57–81` (instance / shared / user TTL classes)
  - `contracts/controller/src/lib.rs:65–69,515–516` (`renew_then!`; `get_market_index` → `new_view`)
  - `contracts/controller/src/views.rs` (nine helpers; all `new_view` or no Cache)
  - Production `Cache::new` sites: `positions/{supply,debt,liquidation/mod}.rs`, `strategies/{flash_loan,flash_position,multiply,swap_debt,swap_collateral,repay_debt_with_collateral,migrate_blend}.rs`, `keepers.rs`, `markets.rs`
  - Double-renew composites: `lib.rs` `upgrade_liquidity_pool_params`, `force_socialize_bad_debt` (`renew_then!` + inner `Cache::new`)
  - Cross-contract: `contracts/price-aggregator/src/lib.rs:40–44,74–88` (`warmed_session` renews **aggregator** instance on `prices` / `quotes`)
- Defense: The two constructors differ by **exactly one durable side effect**: `new` calls `storage::renew_controller_instance` before returning the same empty memo maps that `new_view` builds. Every production mutator that builds a Cache uses `Cache::new` (18 sites). Every production view that builds a Cache uses `Cache::new_view` (10 sites: 9 in `views.rs` + `get_market_index`). No production mutator uses `new_view`; no production view uses `new`. That partition is the rent-grief defense (A008): permissionless readers cannot fund 180-day controller instance bumps. Persistent user/shared keys still touch-renew on read via `get_user` / `get_shared` (INV-STOR-01) — intentional, orthogonal to the constructor split. Instance-key bare getters (`get_pool`, aggregators, limits) never renew.
- Gap: (1) **No unit test** asserts `Cache::new` extends instance TTL while `Cache::new_view` leaves it unchanged — only `renew_controller_instance_re_extends_instance_ttl` covers the helper directly; the constructor partition is enforced by call-site convention + review, not a regression test. (2) **Double renew** on owner `upgrade_liquidity_pool_params` and `force_socialize_bad_debt` (`renew_then!` then `Cache::new`) — harmless (Soroban no-ops when above threshold) but slightly obscures “one renew per entrypoint” mental model. (3) **Semantic trap for readers**: “view = zero rent side effects” is false — views still renew *queried* user/shared keys and may renew the **price-aggregator** instance via `prices`/`quotes` (STRIDE I14/I25); only **controller** instance renew is skipped. (4) Idle protocol with only views can still age out controller instance storage (operational keep-alive via mutators/keepers/admin), not a code hole.
- Impact: Miswiring a view onto `Cache::new` would turn every HF / liquidation-estimate / market-index query into a caller-funded controller instance bump (fee surprise / availability optics / grief vector) — **not** fund theft, share mint, or undercollateralization. Miswiring a mutator onto `new_view` would skip instance keep-alive on that path until some other mutator/admin renews — availability risk (DoS.6 adjacency) if that path were the only traffic, still not accounting corruption. Under the live call graph neither miswire exists. Practical blast radius of residuals: fee noise on double-renew admin paths; observability/ops misunderstanding of view touch renews. Severity stays **info**.
- Evidence: SEED Cache fact (“`Cache::new` renews instance TTL; `new_view` does not”); INV-STOR-01; STRIDE DoS.6 / I14; peers A008, A034, A017, A015, A039, A086, A088, A091, A104; exhaustive `rg Cache::new` / `Cache::new_view` under `contracts/controller/src`; unit `renew_controller_instance_re_extends_instance_ttl`; A034 §2 claim “No production mutator uses `new_view`” re-verified.
- Opinion: Constructor TTL split is **sound and defended** — the right shape for Soroban rent. A093 confirms A034/A008/A086 adjacency with an exhaustive production call-site partition and a precise side-effect matrix (controller instance vs user/shared vs foreign contract). Treat any new view that calls `Cache::new`, or any new mutator Cache built with `new_view`, as a regression. Prefer a tiny unit that constructs both and asserts instance TTL delta; optional comment on the two `renew_then!`+`Cache::new` composites.

---

## Method

1. Read `shared/COORDINATION.md` (findings-only; **no git**), `SEED.md`, `AGENT_MANIFEST` Wave 6 A093, README format.
2. Diff `Cache::new` vs `new_view` at source; trace `renew_controller_instance` → `common::ttl::renew_instance`.
3. Enumerate **every** production `Cache::new` / `Cache::new_view` under `contracts/controller/src` (exclude `tests/`).
4. Classify durable TTL side effects on each constructor path: controller instance, user keys, shared keys, foreign contracts (pool / price aggregator / NFT).
5. Cross-check double-renew (`renew_then!` + `Cache::new`), bare instance getters, and view touch-renew residuals against A034 / A008 / A086 / A104.
6. No production Rust edited.

---

## 1. Executive verdict

**`new` and `new_view` are identical empty Caches except that `new` renews the controller’s instance TTL first.**

| Property | `Cache::new` | `Cache::new_view` |
|---|---|---|
| Memo maps / buffers | Empty (same initializer) | Empty |
| Controller `instance.extend_ttl` | **Yes** (`renew_controller_instance`) | **No** |
| Intended surface | Mutators (positions, strategies, keepers, market param upgrade) | Views (`views.rs` + `get_market_index`) |
| Production call sites (src) | **18** | **10** |
| Cross-partition misuse in production | **None** | **None** |

Highest residual is **test / documentation**, not a live fund-risk gap: the partition is convention-enforced; view paths still have non-instance rent side effects that must not be confused with the constructor skip.

---

## 2. Constructor source (sole durable difference)

```39:62:contracts/controller/src/context/mod.rs
impl Cache {
    /// Renews the controller's instance storage TTL and returns a fresh, empty cache for a state-changing entrypoint.
    pub(crate) fn new(env: &Env) -> Self {
        storage::renew_controller_instance(env);
        Self::new_view(env)
    }

    /// Returns a fresh, empty cache for a read-only entrypoint, without renewing the instance
    /// storage TTL: all memoization maps and update buffers initialized but unpopulated.
    pub(crate) fn new_view(env: &Env) -> Self {
        Cache {
            env: env.clone(),
            token_prices: Map::new(env),
            market_indexes: Map::new(env),
            pool_address: None,
            pool_sync_data: Map::new(env),
            spoke_usage: None,
            spoke_config: None,
            spoke_assets: Map::new(env),
            verified_hubs: Map::new(env),
            supply_updates: Vec::new(env),
            debt_updates: Vec::new(env),
        }
    }
}
```

Module docs at `context/mod.rs:1–6` state the same contract. SEED and A086 agree.

```3:6:contracts/controller/src/storage/protocol.rs
/// Extends the controller contract's instance storage TTL using the
/// protocol-wide instance threshold and bump constants.
pub(crate) use common::ttl::renew_instance as renew_controller_instance;
```

```11:15:common/src/ttl.rs
pub fn renew_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}
```

| Constant | Ledgers | Approx. days (`ONE_DAY_LEDGERS = 17_280`) |
|---|---|---|
| `TTL_THRESHOLD_INSTANCE` | 86_400 | 5d |
| `TTL_BUMP_INSTANCE` | 3_110_400 | 180d |

Soroban no-ops the bump when remaining TTL is still above threshold — repeated `Cache::new` on a fresh instance is cheap.

**What constructors do *not* do:** neither writes instance keys, neither opens spoke usage, neither touches user/shared persistent storage, neither emits events. All of those are later method / storage-helper effects, identical regardless of which constructor built the Cache.

---

## 3. Exhaustive production call-site partition

### 3.1 `Cache::new` — instance renew (18 sites)

| Module | Line | Function / role |
|---|---|---|
| `positions/supply.rs` | 53 | `process_supply` |
| `positions/supply.rs` | 174 | `process_withdraw` |
| `positions/debt.rs` | 48 | `process_borrow` |
| `positions/debt.rs` | 91 | `process_repay` |
| `positions/liquidation/mod.rs` | 58 | `process_liquidation` |
| `positions/liquidation/mod.rs` | 239 | `socialize_bad_debt` (permissionless clean + owner force) |
| `strategies/flash_loan.rs` | 32 | `process_flash_loan` |
| `strategies/flash_position.rs` | 80 | `process_flash_position` |
| `strategies/multiply.rs` | 132 | `process_multiply` |
| `strategies/swap_debt.rs` | 53 | `process_swap_debt` |
| `strategies/swap_collateral.rs` | 51 | `process_swap_collateral` |
| `strategies/repay_debt_with_collateral.rs` | 56 | `process_repay_debt_with_collateral` |
| `strategies/migrate_blend.rs` | 67 | `process_migrate_blend` |
| `keepers.rs` | 21 | `update_indexes` |
| `keepers.rs` | 32 | `claim_revenue` |
| `keepers.rs` | 53 | `recapitalize` |
| `keepers.rs` | 86 | `update_account_threshold` |
| `markets.rs` | 94 | `upgrade_liquidity_pool_params` |

Notes:

- `process_deposit` **reuses** the caller’s `&mut Cache` — no second constructor (strategies / supply already renewed once).
- `flash_loan` renews instance even though it never writes account keys — correct mutator keep-alive (A034).
- `claim_revenue`’s only controller durable *lifecycle* effect is this instance renew (A039).
- Empty keeper Vecs still construct `Cache::new` → instance renew (A015 / A102 fee-annoyance residual, not drain).

### 3.2 `Cache::new_view` — instance skip (10 sites)

| Location | Line | Helper / entrypoint |
|---|---|---|
| `views.rs` | 31 | `health_factor` (`can_be_liquidated` delegates) |
| `views.rs` | 62 | `collateral_amount_for_hub_asset` |
| `views.rs` | 86 | `borrow_amount_for_hub_asset` |
| `views.rs` | 136 | `liquidation_collateral_available` |
| `views.rs` | 155 | `get_all_market_indexes_detailed` |
| `views.rs` | 205 | `liquidation_estimations_detailed` |
| `views.rs` | 255 | `total_collateral_in_usd` |
| `views.rs` | 275 | `total_borrow_in_usd` |
| `views.rs` | 286 | `ltv_collateral_in_usd` |
| `lib.rs` | 515 | `get_market_index` |

Views that **never** build a Cache (still skip instance renew): `account_exists`, `get_account_positions`, `get_account_attributes`, and lib getters that call `storage::get_pool` / `get_spoke` / `get_spoke_asset` / `get_spoke_usage` / blend approval / min-borrow / aggregators directly.

### 3.3 Partition check

```
rg 'Cache::new_view' positions/ strategies/ keepers.rs markets.rs account.rs governance.rs
→ empty

rg 'Cache::new(' views.rs
→ empty
```

A034’s claim holds under re-audit. Test code freely uses both constructors (mostly `new_view`); that does not affect production WASM.

---

## 4. Side-effect matrix (what renews where)

### 4.1 Controller instance storage

| Path class | Renews controller instance? | Mechanism |
|---|---|---|
| `Cache::new` mutators | **Yes** | Constructor |
| `Cache::new_view` views | **No** | Constructor skip |
| Admin `renew_then!` | **Yes** | `lib.rs` macro before admin body |
| Direct `renew_controller_instance` | **Yes** | `renew_account`, `add_delegate`, `remove_delegate`, `accept_ownership` |
| Bare `instance().get` views | **No** | `get_pool`, aggregators, limits, accumulator, min-borrow |

Instance keys (Pool, aggregators, NFT addr, Accumulator, PositionLimits, counters, pause/owner) ride the instance TTL window. Successful mutators/keepers/admin keep the whole set alive; view spam cannot.

### 4.2 Persistent user keys (orthogonal to constructor)

`get_user` / `set_user` always touch-renew the **loaded/written key** (30d threshold / 120d bump). Views that call `try_get_account` / `get_supply_positions` / etc. therefore renew user TTLs when present and below threshold — INV-STOR-01 “renew when read,” **not** a violation of `new_view`.

Sibling co-renew (`renew_user_account`) is a mutator/finalize/keeper/`renew_account` concern (A034 §4), never triggered by constructor choice alone.

### 4.3 Persistent shared keys (orthogonal)

`get_shared` / `set_shared` renew spoke / spoke-asset / spoke-usage / hub / blend-approval / position-manager keys (5d / 180d). View paths that load spoke config for LTV restamp (`ltv_collateral_in_usd` → `restamp_listed_supply_ltv` → `cached_spoke_asset` → `get_spoke_asset`) or direct `get_spoke*` entrypoints renew shared entries. Again: intentional INV-STOR-01; not instance grief.

### 4.4 Foreign contracts

| Contract | When a controller view may renew *its* TTL | Notes |
|---|---|---|
| Price aggregator | Risk / amount / detailed views that call `fetch_prices` or `fetch_prices_status` | `warmed_session` → `renew_instance` on aggregator (STRIDE I14/I25). **Not** controller instance. |
| Pool | Index/sync reads via pool FFI | Pool may renew its own market/instance keys; out of A093 controller-constructor scope. |
| Position NFT | Passive-only paths that resolve owner may touch NFT storage under OZ windows | INV-STOR-02c; not via Cache constructors. |

**Defense intent clarified:** `new_view` prevents **controller** instance rent grief. It does not promise zero cross-contract or zero persistent-key rent.

---

## 5. Double renew and parallel funnels

### 5.1 `renew_then!` + `Cache::new` (two bumps, same tx)

```65:69:contracts/controller/src/lib.rs
macro_rules! renew_then {
    ($env:ident, $body:expr) => {{
        storage::renew_controller_instance(&$env);
        $body
    }};
}
```

| Entrypoint | Outer | Inner |
|---|---|---|
| `upgrade_liquidity_pool_params` | `renew_then!` | `markets::…` → `Cache::new` |
| `force_socialize_bad_debt` | `renew_then!` | `process_force_socialize…` → `socialize_bad_debt` → `Cache::new` |

Permissionless `clean_bad_debt` uses `Cache::new` only (no `renew_then!`) — A027 noted the asymmetry; still renews once via Cache.

Harmless when TTL is fresh. Residual: documentation / mental-model clutter, not a safety bug.

### 5.2 Mutators without Cache

Many admin paths renew only via `renew_then!` (no Cache). Account delegate / renew / ownership paths renew via direct `renew_controller_instance`. Parallel funnels are intentional (A034 §3); A093’s claim is only about the Cache constructor pair.

### 5.3 One renew per Cache lifetime

No production entrypoint constructs two independent `Cache::new` instances in sequence for the same top-level call. Nested helpers take `&mut Cache`. Post-flash monetary reentry builds a **new** Cache in a new entrypoint (A007 / A089) — each renews once for that invocation.

---

## 6. View residual rent map (read-only ≠ zero side effects)

| View surface | Controller instance | User keys | Shared keys | Aggregator instance |
|---|---|---|---|---|
| `health_factor` / totals / liq collateral / liq estimate | skip (`new_view`) | touch via account load | possible if risk loads spoke assets | yes if prices fetched |
| `collateral/borrow_amount_for_hub_asset` | skip | touch position key | no (sync via pool FFI) | no (indexes+decimals only) |
| `ltv_collateral_in_usd` | skip | touch account | yes (`cached_spoke_asset`) | yes (risk load) |
| `get_market_index` / detailed | skip | none | none on controller | detailed: quotes → yes |
| `get_account_*` / attributes (no Cache) | skip | touch | none | none |
| `get_spoke*` | skip | none | touch | none |
| `get_pool_address` / aggregators / floors | skip | none | blend flag if approved | none |

This is the precise answer to “what side effects does `new_view` still allow?” — none on controller instance; INV-STOR-01 touch renews and foreign-contract renews remain.

---

## 7. Threat / invariant alignment

| Claim | Holds? | Anchor |
|---|---|---|
| INV-STOR-01 renew on read/write for persistent keys | Yes | `get_user` / `get_shared`; views participate |
| Mutators keep controller instance alive | Yes | `Cache::new` / `renew_then!` / direct |
| Views are not a controller instance rent vector | Yes | `new_view` + bare getters (A008) |
| STRIDE DoS.6 TTL expiry mitigated by renew on privileged/mutator traffic | Yes | Instance renew on mutators; `renew_account` for user+NFT |
| STRIDE I14 view TTL note | Yes, refined | Aggregator instance may renew; controller instance must not |
| Rent grief via permissionless HF spam | Mitigated for controller instance | Would regress if a view switched to `Cache::new` |

No Critical/High. No fund-flow consequence from the constructor split itself.

---

## 8. Hazards (ranked)

| ID | Hazard | Present today? | Blast radius if it appeared |
|---|---|---|---|
| H1 | View entrypoint calls `Cache::new` | **No** (partition clean) | Caller-funded 180d instance bumps; fee/DoS optics |
| H2 | Mutator builds Cache with `new_view` | **No** | Missed instance keep-alive on that path |
| H3 | Assuming “view = zero rent” and designing fee markets around it | Docs risk | Underestimate user/shared/aggregator renew costs |
| H4 | Double `renew_then!`+`Cache::new` | **Yes** (2 admin paths) | None practical (no-op when fresh) |
| H5 | No constructor TTL unit regression test | **Yes** | Future refactor could silently break H1/H2 |
| H6 | Idle-view-only protocol ages instance out | Operational | Availability until mutator/keeper/admin runs |

---

## 9. Tests / verification anchors

| Check | Location | Covers A093? |
|---|---|---|
| Instance re-extend via helper | `contracts/controller/tests/storage/protocol.rs` (`renew_controller_instance_re_extends_instance_ttl`) | Helper only — **not** constructor pair |
| Co-renew / side-write TTL | `tests/storage/account.rs` | User class; orthogonal |
| Views use `new_view` by construction | Source partition §3.2 + A008 | Review-only |
| Suggested missing unit | Construct `new` vs `new_view` under aged instance; assert TTL bump vs unchanged | Would close H5 |

Certora harness does not model constructor TTL (no matches under controller certora for `renew_instance` / `new_view`).

---

## 10. Cross-links

| Peer | Relation |
|---|---|
| **A034** | Wave-2 full TTL taxonomy; A093 is the T7 deep-dive on Cache constructors; agrees “no mutator uses `new_view`” |
| **A008** | `new_view` as rent-grief defense; A093 supplies exhaustive sites + residual touch matrix |
| **A086** | Field inventory; states `new` renews / `new_view` does not |
| **A017** | Explicit `renew_account` (direct instance renew, not Cache) |
| **A015 / A039** | Keeper / claim_revenue: `Cache::new` as sole controller lifecycle write |
| **A088 / A091** | Constructor adjacency; memos identical after either ctor |
| **A104** | Listed A093 as coverage hole; this file fills it — verdict remains **defended** |
| **A027** | `clean_bad_debt` vs force: force double-renews via `renew_then!`+Cache; clean renews once via Cache |

---

## 11. Opinion / remediation (optional, non-blocking)

1. **Keep the partition.** Do not “optimize” views onto `Cache::new` for any reason.
2. **Add a unit** `cache_new_renews_instance_ttl_new_view_does_not` mirroring the existing helper test.
3. **Document** in `context/mod.rs` or ops notes: views may still renew user/shared keys and the price-aggregator instance.
4. **Optional cleanup:** drop redundant `renew_then!` around bodies that always `Cache::new` (`upgrade_liquidity_pool_params`, `force_socialize_bad_debt`) — cosmetic only; Soroban already no-ops the second bump when fresh.

**Final:** `Cache::new` vs `new_view` TTL side effects are **defended** under the live call graph. Residuals are test coverage, double-renew noise, and semantic precision about non-instance view renews — not undefended fund risk.
