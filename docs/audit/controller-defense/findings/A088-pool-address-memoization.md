# A088 — `pool_address` / `pool_sync_data` memoization

- Agent: A088
- Theme: T7 (also T6 read savings)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/context/mod.rs:25-61` (`Cache` fields; `new` / `new_view`)
  - `contracts/controller/src/context/pool.rs:8-28` (`cached_pool_address`, `cached_pool_sync_data`)
  - `contracts/controller/src/context/market_index.rs:17-42` (pool address consumers for bulk/lazy indexes)
  - `contracts/controller/src/external/pool.rs:147-153` (`fetch_pool_sync_data`)
  - `contracts/controller/src/storage/protocol.rs:29-42` (`get_pool` / `set_pool`)
  - `contracts/controller/src/markets.rs:17-34,86-104` (`deploy_pool` one-shot; `upgrade_liquidity_pool_params`)
  - `contracts/controller/src/strategies/flash_position.rs:80-94,125-128` (sole mutator sync-data gate)
  - `contracts/controller/src/strategies/flash_loan.rs:32-37` (pool address only; no sync memo)
  - `contracts/controller/src/views.rs:62-88` (view decimals via sync memo)
  - `contracts/controller/src/config/asset.rs:71-73` (direct `fetch_pool_sync_data`, bypasses Cache)
  - `contracts/pool/src/lib.rs:299-301,110-112` (`get_sync_data`; `update_params`)
  - `contracts/pool/src/storage.rs:49-54,74-94` (`load_sync_data`; `write_rate_model`)
  - `contracts/pool/src/ops/flash.rs:76-84` (pool-side `is_flashloanable` for cash flash)
  - `common/src/types/pool.rs:16-41,457-460,540-553` (`MarketParamsRaw`, `PoolSyncData`, `PoolStateRaw`)
- Defense: `pool_address` is a fill-once singleton read from instance storage; the only writer (`set_pool`) is the one-shot `deploy_pool` path, so the memo cannot diverge from storage for any live mutator Cache. `pool_sync_data` is a fill-once `Map<HubAssetKey, PoolSyncData>` with **no** invalidation API and **no** post-leg overwrite (unlike `put_market_index`). Production money/risk paths after pool legs use mutation indexes + account positions, not sync blobs. The only mutator reader of `cached_pool_sync_data` is `flash_position`’s pre-leg `is_flashloanable` assert — ordered **before** any pool mutation on that Cache. Views build a fresh `new_view` Cache per call. Admin cap validation bypasses Cache via direct `fetch_pool_sync_data`. Cash `flash_loan` does not use controller sync memo; the pool re-checks `is_flashloanable` itself.
- Gap: Incomplete invalidation rule — after any pool FFI that mutates params or state for a hub, a later `cached_pool_sync_data` hit for that hub would return the pre-mutation snapshot. No production call site does that safety-critical re-read today. Module docs do not state the fill-once / no-invalidate contract (A086 residual; A104 P2/P3). Unlike `market_indexes`, there is no `put_pool_sync_data` / clear helper for touched hubs. Future code that gates on sync params/flags **after** a leg (or after `upgrade_liquidity_pool_params` in a composite flow that reused one Cache) would be wrong. Certora summaries can draw independent sync vs bulk-index nondets for the same market (suite-review note) — harness concern, not production WASM.
- Impact: **No fund-theft, share-mint, or undercollateralized exit** from these two memos under current call graphs. `pool_address` mis-target is unreachable after deploy (immutable instance key). Stale `pool_sync_data` blast radius, if a future reader is added post-mutation: wrong **boolean/param** decision for one hub within one invocation (e.g. false-allow / false-deny `is_flashloanable`, wrong `asset_decimals` for a view-style unscale, or mis-read of raw unaccrued `state` indexes/cash). Upper bound is account/tx-local accept/reject or observability skew — not durable protocol SoT corruption (pool storage remains truth; next invocation fetches fresh). Practical impact today ≈ **negligible**. Severity stays **info**; do not elevate unless a post-mutation sync safety read appears.
- Evidence: Exhaustive `cached_pool_*` / `fetch_pool_sync_data` / `set_pool` grep under `contracts/controller`; peer A086 (inventory + sync residual), A094 (index overwrite footgun; address called immutable), A104 §4.2–4.3 / §7 A088 hole, A044 (pool flashloanable; controller cash flash skips sync memo), A007 (flash guard bounds mid-callback monetary reentry); `deploy_pool` `PoolAlreadyDeployed`; pool `write_rate_model` leaves `asset_id` / `asset_decimals` unchanged; SEED Cache facts; certora-suite-review F3 sync vs bulk-index independence.
- Opinion: **Confirm A086 / A104:** architecture defended; sync-data non-invalidation is a real incomplete rule with low practical risk on today’s graph. Document fill-once semantics next to `put_market_index` overwrite. Do not clear sync data on every leg unless a post-leg reader is introduced — prefer a checklist: “never re-read `cached_pool_sync_data` after mutating that hub in the same Cache.” Optional small helper to `remove` a hub’s sync entry after param upgrades if composite admin+user flows ever share a Cache (they do not today).

---

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, README finding format, `AGENT_MANIFEST` Wave 6 (A088), peers **A086**, **A094**, **A104** (and adjacency A007/A044/A034).
2. Read `context/{mod,pool,market_index}.rs`, `external/pool.rs::fetch_pool_sync_data`, `storage/protocol.rs` pool key, `markets.rs` deploy/upgrade, pool `get_sync_data` / `update_params` / flash `prepare`.
3. Enumerated **every** production `cached_pool_address`, `cached_pool_sync_data`, and `fetch_pool_sync_data` call site under `contracts/controller/src`; ordered each against pool mutations in the same `Cache` lifetime.
4. Classified what `PoolSyncData` fields can change mid-ledger (params via owner `update_params`; state via any accruing mutation) vs what money paths actually consume after legs.
5. Checked reentrancy: can owner/callback flip `is_flashloanable` after controller memoized it; does any path re-check via Cache.
6. No production Rust edited. No git operations (COORDINATION).

No novel Critical/High. Agrees with A086 residual and A104 ranking (sync-data incomplete invalidation below A094 index footgun).

---

## 1. Memo primitives

### 1.1 `Cache` fields

```25:37:contracts/controller/src/context/mod.rs
pub(crate) struct Cache {
    env: Env,
    token_prices: Map<Address, PriceFeedRaw>,
    market_indexes: Map<HubAssetKey, MarketIndexRaw>,
    pool_address: Option<Address>,
    pool_sync_data: Map<HubAssetKey, PoolSyncData>,
    // ... spoke_*, verified_hubs, event buffers ...
}
```

Both constructors leave `pool_address: None` and an empty `pool_sync_data` map. `Cache::new` renews instance TTL then `new_view`; `new_view` does not renew (A034/A093 adjacency). Per-invocation only — not durable across transactions.

### 1.2 `cached_pool_address` — fill-once singleton

```10:17:contracts/controller/src/context/pool.rs
pub(crate) fn cached_pool_address(&mut self) -> Address {
    if let Some(addr) = &self.pool_address {
        return addr.clone();
    }
    let addr = storage::get_pool(&self.env);
    self.pool_address = Some(addr.clone());
    addr
}
```

| Property | Behavior |
|---|---|
| Miss | `storage::get_pool` → instance `ControllerKey::Pool`; panic `PoolNotInitialized` if unset |
| Hit | Clone memoized `Address` |
| Invalidate / overwrite API | **None** |
| Cross-contract | None on hit after first fill |

### 1.3 `cached_pool_sync_data` — fill-once per hub

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

| Property | Behavior |
|---|---|
| Miss | `LiquidityPoolClient::get_sync_data` via memoized pool address |
| Hit | Return map entry; **no** re-fetch |
| Invalidate / overwrite | **None** (contrast `put_market_index`) |
| Payload | Full `PoolSyncData { params: MarketParamsRaw, state: PoolStateRaw }` |

Pool `get_sync_data` loads **raw** params+state and renews market TTL — it does **not** run `simulate_update_indexes`. Accrued index views for risk use `get_bulk_indexes` / mutation returns + `market_indexes`, not this blob’s `state.*_index`.

### 1.4 Contrast with sibling memos (A086/A094)

| Field | Refresh rule | Used after pool legs for money/HF? |
|---|---|---|
| `token_prices` | Fill-once (ADR-0005) | Yes — by design frozen |
| `market_indexes` | Simulate fill + **`put_market_index` overwrite** | Yes — must track post-accrual |
| `pool_address` | Fill-once | Yes (FFI target) — safe because immutable |
| `pool_sync_data` | Fill-once, **no overwrite** | **No** on current mutator paths after legs |

---

## 2. `pool_address` correctness

### 2.1 Storage lifecycle

| Operation | Who | Effect |
|---|---|---|
| `deploy_pool` | `#[only_owner]` | Deploy at fixed salt; `set_pool`; panics `PoolAlreadyDeployed` if `try_get_pool` is `Some` |
| `get_pool` / `cached_pool_address` | Any Cache user | Read instance key |
| Re-point / clear pool | **No production API** | Address is effectively immutable after first deploy |

`upgrade_pool` upgrades WASM at the **existing** address; it does not rewrite `ControllerKey::Pool`. Therefore a Cache that memoized the address cannot point at a different contract mid-tx via protocol admin.

### 2.2 Call-site inventory (`cached_pool_address`)

All production sites under `contracts/controller/src` (FFI targeting / receiver bans):

| Area | File (representative) | Role of address |
|---|---|---|
| Market index fetch | `context/market_index.rs` | Bulk/lazy `get_bulk_indexes` |
| Sync fetch | `context/pool.rs` | `get_sync_data` |
| Supply / withdraw | `positions/supply.rs` | Pool supply/withdraw FFI |
| Borrow / repay / strategy borrow | `positions/debt.rs` | Pool debt FFI |
| Shared merge helpers | `positions/mod.rs` | Pool legs |
| Liquidation / bad debt | `liquidation/apply.rs`, `bad_debt.rs` | Seize / socialize FFI |
| Keepers | `keepers.rs` | `update_indexes`, recapitalize, claim revenue |
| Flash loan / flash position | `strategies/flash_*.rs` | Pool flash / receiver ≠ pool ban |
| Strategy legs | `strategies/legs.rs` | Debt/collateral pool addrs |
| Param upgrade | `markets.rs` | Accrue + `update_params` |

Public view `get_pool_address` (`lib.rs`) calls `storage::get_pool` **directly** — no Cache, no memo (A034).

### 2.3 Same-tx change scenarios — rejected

| Hypothesis | Why unreachable |
|---|---|
| Second `deploy_pool` after Cache fill | Panics `PoolAlreadyDeployed`; also owner-only, separate entrypoint |
| Mid-flash reentrancy rewrites pool key | No `set_pool` on any flash-/user-reachable path; monetary reentry blocked (A007) |
| WASM upgrade changes address | Upgrade mutates code at same address |
| View Cache sees different pool than mutator | Both read same instance key; views don’t need memo correctness for safety |

**Verdict:** `pool_address` memoization is **correct**. A094’s “immutable for tx” claim holds for the stronger reason “immutable for protocol lifetime after deploy.”

---

## 3. `pool_sync_data` payload and mutators

### 3.1 What the blob contains

`MarketParamsRaw` (gate-relevant subset): rate curve, `reserve_factor`, **`is_flashloanable`**, **`flashloan_fee`**, `asset_id`, **`asset_decimals`**.

`PoolStateRaw`: `supplied` / `borrowed` / `revenue` / `borrow_index` / `supply_index` / `last_timestamp` / `cash` — all change on accruing pool mutations.

### 3.2 What can change while a Cache is live

| Pool / controller action | Params change? | State change? | Same Cache could already hold sync? |
|---|---|---|---|
| supply / borrow / withdraw / repay / seize / strategy mutate | No (identity/rate model) | Yes | Yes, if sync was warmed earlier |
| pool `flash_loan` | No | Yes (fee booking) | Only if sync warmed earlier in same Cache |
| `update_indexes` (keeper / pre-param) | No | Yes (accrual write) | Possible in theory |
| `upgrade_liquidity_pool_params` → `update_params` | **Yes** (`is_flashloanable`, fee, curve, …) | Yes (accrue first) | Only if that Cache had warmed sync — **today it does not** |
| `create_market` | New hub only | New hub | N/A for existing keys |
| `write_rate_model` | Flips model fields; **leaves `asset_id` / `asset_decimals`** | Via prior accrue in controller path | — |

### 3.3 Direct vs cached fetch

| Consumer | API | Cache involved? |
|---|---|---|
| `flash_position` flashloanable gate | `cached_pool_sync_data` | Yes |
| `views::{collateral,borrow}_amount_for_hub_asset` decimals | `cached_pool_sync_data` | Yes (`new_view`) |
| `config/asset` spoke cap domain | `fetch_pool_sync_data` + `storage::get_pool` | **No** — always live |

Admin listing/edit therefore cannot be poisoned by a stale Cache entry from a prior leg in another entrypoint (separate invocations anyway).

---

## 4. Exhaustive sync-data read vs mutation timing

### 4.1 Mutator: `flash_position` (only production money-path sync reader)

Ordered excerpt:

1. `Cache::new`
2. `cached_pool_address` (receiver ≠ pool)
3. **`cached_pool_sync_data(debt).params.is_flashloanable`** — **first and only** sync read
4. Account load, collateral/refund validation, `prefetch_strategy_prices`
5. `with_flash_guard` → `mint_and_forward` → `borrow_into_controller` (pool **state** mutates; `put_market_index` on merge)
6. Receiver callback; further legs / finalize use **indexes + positions**, not sync

```80:94:contracts/controller/src/strategies/flash_position.rs
    let mut cache = Cache::new(env);
    let pool_addr = cache.cached_pool_address();
    // ...
    assert_with_error!(
        env,
        cache.cached_pool_sync_data(debt).params.is_flashloanable,
        FlashLoanError::FlashloanNotEnabled
    );
```

**Stale-after-mutation does not apply** to this gate: the check precedes mutation. A second hit would only matter if code re-asserted flashloanable after the borrow/callback; it does not.

**Mid-callback param flip:** `upgrade_liquidity_pool_params` is `#[only_owner]` and does **not** call `require_not_flash_loaning`. In principle an owner-authorized nested call during the flash-position window could disable flashloanable on the pool **after** the controller already passed step 3. Effects:

- Controller does not re-read sync; gate already passed (intentional one-shot check).
- Subsequent cash `flash_loan` in another top-level call would hit pool `prepare` live check.
- Nested monetary position entrypoints still fail `FlashLoanOngoing` (A007).
- Owner flipping flags mid-callback is a governance/trust scenario, not a Cache-bypass by an unprivileged attacker.

Defense-in-depth note: cash `flash_loan` never uses controller sync memo; pool `ops::flash::prepare` asserts live `is_flashloanable` (A044). `flash_position` debt is ordinary strategy borrow — controller sync check is the dedicated “caller-chosen receiver” policy (comment in `flash_position.rs`).

### 4.2 Mutator: `process_flash_loan`

Uses `cached_pool_address` only. Sync memo never filled. Pool enforces flashloanable + fee from **live** params.

### 4.3 Mutator: `upgrade_liquidity_pool_params`

```94:103:contracts/controller/src/markets.rs
    let mut cache = Cache::new(env);
    let pool_addr = cache.cached_pool_address();
    pool_update_indexes_call(...);
    pool_update_params_call(...);
```

Creates a Cache, never calls `cached_pool_sync_data`. No stale sync consumer in this entrypoint. Does not clear a map that was never filled.

### 4.4 Ordinary position / liquidation / keeper / strategy legs

Widespread `cached_pool_address` + `put_market_index` after merges. **Zero** `cached_pool_sync_data` calls. Sync map stays empty for the entire invocation.

### 4.5 Views

```62:64:contracts/controller/src/views.rs
    let mut cache = Cache::new_view(env);
    let market_index = cache.cached_market_index(hub_asset);
    let decimals = cache.cached_pool_sync_data(hub_asset).params.asset_decimals;
```

- Fresh Cache per view call → no cross-tx memo.
- Within the call: one sync fetch + one (simulated) index fetch; no intervening mutation.
- Uses sync for **decimals only**; valuation indexes come from `cached_market_index` (simulate path), not `sync.state`.
- `asset_decimals` is not updated by `write_rate_model` — even a hypothetical mid-tx param upgrade would not change the decimals field used here.

### 4.6 Timing matrix (summary)

| Site | Sync read position | Later pool mutation in same Cache? | Later sync re-read? | Hazard |
|---|---|---|---|---|
| `flash_position` | Pre-leg | Yes (borrow + …) | **No** | None today |
| Views | Only read | No | No | None |
| `config/asset` | Live fetch | N/A (no Cache) | N/A | None |
| All other mutators | Never | Often | Never | None |
| Hypothetical post-leg gate via Cache | After mutation | — | Hit → **stale** | Future footgun (A086/A104) |

---

## 5. Invalidation rules (as-implemented vs desired)

### 5.1 As implemented

| Event | `pool_address` | `pool_sync_data` | `market_indexes` |
|---|---|---|---|
| First read | Fill | Fill per hub | Fill (simulate) |
| Pool position mutation | Unchanged | **Stale if present** | Overwritten via `put_market_index` |
| `update_params` | Unchanged | **Stale if present** | May be stale unless re-fetched/`put` (param path uses fresh Cache, no index memo use after) |
| `reset_spoke_context` | Unchanged | Unchanged | Unchanged |
| End of entrypoint | Dropped with Cache | Dropped | Dropped |

There is no `pool_sync_data.remove`, no clear-on-mutation, and no module-doc statement of this contract (gap vs A094’s documented overwrite discipline for indexes).

### 5.2 Why current code remains safe

1. Money and HF after legs trust **mutation indexes** and **account position maps**, not sync state (A077/A082/A094).
2. The only mutator sync field consumed is **`is_flashloanable`**, checked **before** legs.
3. View decimals are stable identity metadata.
4. Caps on listing use **live** sync fetch.
5. Cash flashloanable is enforced again on the pool with live params.

### 5.3 Future footgun catalogue (do not ship without invalidation)

| Added pattern | Failure mode |
|---|---|
| Re-check `is_flashloanable` after callback using Cache | Miss owner mid-window disable/enable; or miss pool-side change |
| Read `sync.state.borrow_index` for HF/caps after a leg | Pre-accrual / pre-mutation indexes; bypasses `put_market_index` |
| Read `flashloan_fee` from Cache to invoice after mutation | Wrong fee vs pool live terms |
| Share one Cache across `upgrade_liquidity_pool_params` + user flash in a new composite entrypoint | Sync filled before upgrade → stale flags |
| Use sync `asset_decimals` after a (currently impossible) decimals migration API | Wrong unscale |

Remediation if any of the above is needed: `pool_sync_data.remove(hub)` (or clear map) after touching that hub’s pool params/state, **or** always `fetch_pool_sync_data` for post-mutation safety reads (bypass Cache). Prefer documenting “sync = preflight params only.”

---

## 6. Cross-links

| Peer | Relation |
|---|---|
| **A086** | Owns inventory; this file **confirms** sync non-invalidation residual and exhausts call-site timing A104 asked for |
| **A094** | Sibling staleness class for **indexes**; higher practical priority because HF/caps **do** re-read `market_indexes` after legs |
| **A104** | Ranked sync residual #2; coverage hole A088 closed by this filing — expect synthesis refresh |
| **A044** | Cash flash: pool live `is_flashloanable`; controller skips sync memo |
| **A007** | Flash guard limits unprivileged mid-callback monetary reentry; does not block owner admin |
| **A034** | `new_view` / public `get_pool_address` without Cache memo |
| **A077** | Caps use mutation indexes, not sync simulate |
| Certora suite-review F3 | Prover summaries may disagree sync vs bulk index — not a production memo bug |

**Agreements:** With A086 (defended + residual), A094 (address immutable; index overwrite is the sharper footgun), A104 §4.3 reader table and impact bound.

**Disagreements:** None. Does **not** elevate sync residual to medium/high; does **not** claim a live post-mutation sync safety check exists.

---

## 7. Impact quantification (T8)

| Loss class | From `pool_address` / `pool_sync_data` memo? |
|---|---|
| Protocol share mint / free cash | **No** |
| Undercollateralized gated exit left on-chain | **No** (sync not on HF path) |
| Wrong pool FFI target mid-tx | **No** (immutable address) |
| Wrong flashloanable admit on `flash_position` due to stale Cache | **No** under current order; only if check moved after mutation or composite Cache reuse |
| Wrong view amount decimals | **No** practical (immutable decimals; fresh view Cache) |
| Same-tx wrong boolean/param if future post-leg sync read | **Yes** — availability or unintended allow of a flag check; still no automatic share mint |
| Durable wrong SoT | **No** — pool storage unaffected; next tx refetches |

**Single-account max (current code):** none from these memos.

**Single-account max (future misuse):** incorrect accept/reject of one flash-like / param gate in one tx.

**Market/protocol max:** none durable.

---

## 8. Remediation notes (for A110; docs-only unless call graph changes)

| P | Action | Closes |
|---|---|---|
| P2 (A104) | Document in `context/mod.rs` / `pool.rs`: address fill-once immutable; sync fill-once **preflight**; indexes overwrite via `put_market_index` | A086/A088 docs gap |
| P3 (A104) | If any post-leg sync safety read is added: clear that hub’s `pool_sync_data` entry (or bypass Cache) | Future footgun |
| Checklist | Review: no new `cached_pool_sync_data` after pool mutation for that hub in the same Cache | Process |
| Anti-fix | Do not “refresh” sync on every leg preemptively without a reader — budget only | Hygiene |

---

## 9. Verdict

1. **`pool_address` memoization is defended and correct** — one-shot deploy, no rewrite API, safe fill-once singleton for all FFI sites.
2. **`pool_sync_data` memoization is defended for current call sites** — fill-once without invalidation is an incomplete rule, but every production reader is either pre-mutation (`flash_position`) or mutation-free (`views`), and admin paths bypass Cache.
3. **Residual matches A086/A104:** document the contract; clear-on-touch only if post-leg sync gates appear; keep severity **info**.
4. **Do not conflate with A094:** forgetting `put_market_index` is a live engineering footgun on paths that already re-read indexes; forgetting sync invalidation is latent until someone adds a post-leg sync reader.

**Bottom line:** A088 closes the Wave-6 deep-dive hole on these two fields. No novel Critical/High. Ship documentation of fill-once sync semantics; leave runtime invalidation optional until the call graph requires it.
