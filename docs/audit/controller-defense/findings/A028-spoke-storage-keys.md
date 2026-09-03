# A028 — Spoke config / spoke asset / spoke usage key families

- Agent: A028
- Theme: T2
- Severity: low
- Status: partial
- Paths: `contracts/controller/src/storage/spoke.rs:1-83`; `contracts/controller/src/storage/protocol.rs:164-190` (`increment_counter`, `get_shared`, `set_shared`); `common/src/types/controller.rs:94-161,540-557` (`SpokeConfig`, `SpokeAssetConfig`, `SpokeUsageRaw`, `ControllerKey`); `contracts/controller/src/config/{spoke,asset}.rs`; `contracts/controller/src/spoke_usage.rs:78-81,91-97`; `contracts/controller/src/context/spoke.rs:39-143`; `contracts/controller/src/lib.rs:526-542,642-720`; `contracts/controller/src/positions/liquidation/plan.rs:67`; `common/src/constants/shared.rs:61-81`; `contracts/controller/tests/storage/spoke.rs`
- Defense: Four disjoint `ControllerKey` variants with typed `pub(crate)` accessors only; counter in instance storage; config/usage in persistent shared TTL; `Spoke` soft-deprecates in place; `SpokeAsset` hard-deletes after zero-usage gate; `SpokeUsage` auto-prunes both-zero rows; reads fail closed (`SpokeNotFound` / `Option`) or default only on the public usage view; user paths cannot write config keys
- Gap: No dedicated renew/reconcile for shared spoke keys. `renew_account` keeps user legs alive without touching `Spoke` / `SpokeAsset` / `SpokeUsage`. Expired `Spoke`/`SpokeAsset` fail-close position and liquidation paths while accounts remain. Orphaned non-zero `SpokeUsage` (account TTL expiry without exit, or A080 under/over-count) has no admin write path — only TTL death or further exits — and permissionless views re-arm shared TTL on read
- Impact: No direct fund theft from key-family mixups (typed wrappers + enum discriminants). Residual blast radius is **availability / soft-cap integrity**: (1) dormant-spoke config expiry → `SpokeNotFound` / `AssetNotInSpoke` on liquidate/exit while NFT+account keys still live via owner renew; (2) zombie usage → false `SpokeSupplyCapReached` / `SpokeBorrowCapReached` until row expires or is exited. Caps are soft governance limits (A080); distortion is capacity, not insolvency math
- Evidence: INV-STOR-01, INV-HALT-03, ADR-0002, ADR-0015; unit tests `try_get_spoke_renews_shared_ttl_on_read`, `spoke_asset_discrete_key_roundtrip`, `spoke_usage_prunes_zero_entry`; harness `spoke_caps.rs`, `spoke.rs`; Certora `spoke_rules.rs` / fixture usage helpers; cross-links A029, A034, A076, A080, A017
- Opinion: The storage surface itself is well defended for **type isolation, write gating, and zero-prune hygiene**. The residual is lifecycle asymmetry between shared market keys and user keys — same family of operational hazard as INV-STOR-02 for NFT TTL, but for spoke/cap rows. Treat A028 as “accessors sound; TTL/reconcile incomplete,” not as a key-collision or unauthorized-write bug.

## Method

1. Enumerated every accessor in `storage/spoke.rs` and its `ControllerKey` + storage class.
2. Traced every production caller of `set_*` / `remove_*` / `increment_spoke_id` (admin config vs usage persist).
3. Compared TTL class (`TTL_*_SHARED` vs `TTL_*_USER`) and renew paths against `renew_account` (A017) and views.
4. Checked delete/deprecate semantics for orphaned child keys and cap integrity (INV-HALT-03 / A080).
5. Cross-checked fail-closed read behavior on entrypoints that depend on these keys (liquidation curve, listing gates).

---

## 1. Key-family inventory

| Key | Storage class | Value type | Create | Update | Delete / prune | Read miss |
|---|---|---|---|---|---|---|
| `ControllerKey::LastSpokeId` | **instance** | `u32` | `increment_spoke_id` via `increment_counter` (default 0) | atomic +1, panic `MathOverflow` | never | treated as 0 |
| `ControllerKey::Spoke(u32)` | persistent **shared** | `SpokeConfig` | `set_spoke` on `add_spoke` | `set_spoke` on deprecate / curve edit | **never removed** (soft-delete = `is_deprecated`) | `get_spoke` → `SpokeNotFound` (#300); `try_get_spoke` → `None` |
| `ControllerKey::SpokeAsset(u32, HubAssetKey)` | persistent **shared** | `SpokeAssetConfig` | `set_spoke_asset` on add/edit/flags | same | `remove_spoke_asset` (raw `persistent.remove`) | `Option::None` → callers panic `AssetNotInSpoke` (#307) or skip |
| `ControllerKey::SpokeUsage(u32, HubAssetKey)` | persistent **shared** | `SpokeUsageRaw` | `set_spoke_usage` when non-zero | same | remove when **both** RAY legs are 0 | `Option::None` (treated as zero by usage context / view) |

Discriminant separation is structural: Soroban `#[contracttype]` enum variants cannot collide across `Spoke` / `SpokeAsset` / `SpokeUsage` / instance singletons. Composite identity for asset and usage is `(spoke_id, HubAssetKey { hub_id, asset })` — same token under two hubs is two markets (ADR-0002); same hub-asset on two spokes is two independent usage/cap domains (A083 territory).

`LastSpokeId` lives with other protocol counters (`LastHubId`) in instance storage, renewed by `Cache::new` / admin `renew_then!`, not by `get_shared`. First successful `add_spoke` yields id `1`; id `0` remains reserved (`account::create_account_with` asserts `spoke_id >= 1`).

---

## 2. Accessor enclosure (type safety)

```6:79:contracts/controller/src/storage/spoke.rs
pub(crate) fn increment_spoke_id(env: &Env) -> u32 { ... }
pub(crate) fn get_spoke / try_get_spoke / set_spoke ...
pub(crate) fn get_spoke_asset / set_spoke_asset / remove_spoke_asset ...
pub(crate) fn get_spoke_usage / set_spoke_usage ...
```

- Raw `get_shared` / `set_shared` are `pub(super)` inside `storage/` only (`protocol.rs:175-190`). Crate-visible API is typed per key family (`storage/mod.rs:31-34`).
- A wrongly-typed write under a spoke key cannot compile through the public accessors. This matches the module comment in `storage/mod.rs` and parallels A029’s instance/shared split.
- `remove_spoke_asset` bypasses `set_shared` (correct for deletion). `set_spoke_usage` likewise uses raw `remove` on the zero path — same pattern as `set_blend_pool_approved(false)` / inactive position-manager clear in `protocol.rs`.
- Storage layer is intentionally dumb: it does **not** re-validate risk bounds, flag ratchet, or non-negative RAY. Validation sits in `config/asset.rs` / `spoke_usage.rs` before write. Acceptable layering; a future direct `set_shared` misuse inside `storage/` would be the regression class to watch.

---

## 3. Who may write (mutation surface)

| Writer | Keys touched | Gate |
|---|---|---|
| `config::spoke::add_spoke` | `LastSpokeId`, `Spoke(id)` | `#[only_owner]` + `renew_then!` |
| `config::spoke::remove_spoke` | `Spoke(id)` flag only | `#[only_owner]`; panics if already deprecated |
| `config::spoke::set_spoke_liquidation_curve` | `Spoke(id)` curve fields | `#[only_owner]` + `validate_liquidation_curve` |
| `config::asset::upsert_spoke_asset` / `set_spoke_asset_flags` | `SpokeAsset` | `#[only_owner]` (+ ratchet on flags path) |
| `config::asset::remove_asset_from_spoke` | `SpokeAsset` remove | `#[only_owner]`; requires usage both-zero |
| `SpokeUsageContext::persist` ← `Cache::persist_spoke_usage` ← position finalize / bad-debt | `SpokeUsage` set or prune | User/keeper flows after pool legs; not a public storage API |

**No user entrypoint writes `Spoke` or `SpokeAsset`.** Position/strategy paths only mutate `SpokeUsage` through the cache. Admin views that *read* config renew TTL but do not widen write authority.

`try_get_spoke` is re-exported only under `feature = "certora"` (`storage/mod.rs:39-40`) for specs — production crate surface stays panic-on-miss for `get_spoke`.

---

## 4. Lifecycle semantics (defended)

### 4.1 `Spoke` — soft deprecate, never unlink

`remove_spoke` flips `is_deprecated` and keeps the row. That preserves account spoke binding (`AccountMeta.spoke_id`) and liquidation curve reads after deprecation. Child `SpokeAsset` / `SpokeUsage` rows are **not** cascaded away (by design: open positions may still unwind).

Implications for storage audit (not necessarily bugs):

- Deprecated spoke still occupies a shared persistent slot until TTL expiry if untouched.
- `edit_asset_in_spoke` / `set_spoke_asset_flags` / `remove_asset_from_spoke` do not re-check `is_deprecated` at the storage layer (Add path does). Config policy, not key hygiene.
- Re-using a spoke id after deprecate is impossible: ids only increment.

### 4.2 `SpokeAsset` — discrete listing key + gated hard delete

Round-trip covered by `spoke_asset_discrete_key_roundtrip`. Removal path:

1. Assert listing exists.
2. Load usage `unwrap_or_default()`; require both RAY legs `== 0` else `SpokeAssetInUse` (#309).
3. `remove_spoke_asset` — deletes config only.

If usage was already pruned to absent (`None`), step 2 treats it as zero and delist succeeds. If a non-zero usage row remains, delist fails — correct coupling to INV-HALT-03 / ADR-0015.

**No explicit `SpokeUsage` remove on delist** — relies on prior zero/prune. Consistent if all writers go through `set_spoke_usage`.

### 4.3 `SpokeUsage` — zero prune

```66:78:contracts/controller/src/storage/spoke.rs
    if usage.supplied_scaled_ray == 0 && usage.borrowed_scaled_ray == 0 {
        env.storage().persistent().remove(&key);
    } else {
        set_shared(env, &key, usage);
    }
```

- Absent row ≡ zero (lazy create on entry in `SpokeUsageContext::apply_entry`).
- Full exit that reaches (0,0) deletes the key (rent + INV-STOR-01 empty-state spirit for this family).
- Unit: `spoke_usage_prunes_zero_entry`; spoke unit tests cover entry-create / exit-prune.

Semantics of apply_entry/exit and missing-row no-op are owned by A076 / A080 — A028 only records that the durable key family correctly implements “write non-zero / delete zero.”

---

## 5. TTL class and renewal (partial gap)

Shared constants (`common/src/constants/shared.rs`):

| Class | Threshold | Bump |
|---|---|---|
| Shared (spoke config/asset/usage, hubs, …) | 5 days | **180 days** |
| User (account meta/positions/delegates) | 30 days | **120 days** |

Renewal rules via `get_shared` / `set_shared`:

- Present read → `extend_ttl(threshold_shared, bump_shared)`.
- Write → set then extend.
- Missing read → no extend.
- Delete → no extend (entry gone).

Verified for `Spoke`: `try_get_spoke_renews_shared_ttl_on_read`.

### 5.1 Incidental renew paths (defense)

Any live traffic that loads these keys re-arms 180d:

- Position/strategy finalize touching usage.
- Listing checks / `require_spoke_asset` / `spoke_config` (incl. liquidation plan curve at `plan.rs:67`).
- Permissionless views `get_spoke`, `get_spoke_asset`, `get_spoke_usage` (`lib.rs:527-541`) — **views are de-facto keepers** for shared spoke TTL when values exist.

### 5.2 Asymmetry vs `renew_account` (gap)

`renew_account` (A017) extends **user** keys + NFT Owner/Balance only. It does **not** touch `Spoke` / `SpokeAsset` / `SpokeUsage`.

Failure mode A — config expiry while accounts live:

1. Accounts on spoke S only call `renew_account` for >180d (no supply/withdraw/borrow/repay/liquidate, no admin edits, no `get_spoke*` views).
2. `Spoke(S)` and `SpokeAsset(S, ·)` expire → reads return miss.
3. Liquidation loads `cache.spoke_config` → `SpokeNotFound`; exits needing listing → `AssetNotInSpoke`.
4. User keys + NFT remain live → **stranded solvency / exit / liquidation availability** on that spoke until admin recreates config (ids are not recycled; admin must `set_spoke`/`set_spoke_asset` again via owner paths — `set_spoke` exists only inside config helpers, so recovery is `add_spoke` cannot restore same id; **practical recovery is re-listing under operational runbooks or a future admin restore tool**, which does not exist as a first-class “undelete”).

More precisely: once `Spoke(id)` is gone, `set_spoke` from `add_spoke` always uses a **new** id. There is **no** admin API to rewrite `Spoke(id)` for an arbitrary existing id except `remove_spoke` / `set_spoke_liquidation_curve`, both of which call `get_spoke` first and panic if missing. **Expired `Spoke(id)` is currently unrecoverable at that id** without a contract upgrade or raw storage intervention. That elevates the operational severity of shared-key expiry for spoke config beyond “call a view sometimes.”

Failure mode B — zombie usage (capacity DoS):

1. Positions recorded in `SpokeUsage`, then account persistent keys expire without `apply_exit` (TTL death ≠ coded exit).
2. Usage row remains non-zero (until its own 180d from last touch).
3. Cap headroom shrinks vs true live positions (possibly zero).
4. `get_spoke_usage` view **renews** the zombie, potentially forever.
5. No `ControllerAdmin` setter to zero/reconcile usage (contrast: config keys have owner mutators).

Failure mode C — usage expiry while positions live (cap under-count):

1. Positions kept alive via `renew_account` only.
2. `SpokeUsage` expires after 180d quiet.
3. `get_spoke_usage` → `None` ≡ zero → new entries can fill the configured cap **on top of** still-live positions (same economic class as A080 missing-row under-count).

A028 and A080 agree on impact class: soft-cap distortion, not direct theft. A028 adds the **storage-TTL mechanism** that can *create* missing or zombie rows without a logic bug in `apply_exit`.

### 5.3 Relation to INV-STOR-01

INV-STOR-01 claims market records renew when read or written and empty account state is removed. Spoke families renew on read/write and prune zero usage — **compliant when traffic exists**. The invariant text does not require a permissionless shared renew analogous to `position-nft::renew`. Residual is operational, parallel to documented NFT enumeration TTL gaps (INV-STOR-02d), not an accessor defect.

---

## 6. Fail-closed vs defaulting reads

| API | Miss behavior | Callers |
|---|---|---|
| `get_spoke` | panic `#300` | admin mutators, `Cache::spoke_config`, public view |
| `get_spoke_asset` | `None` | cache / config; public view unwraps to `#307` |
| `get_spoke_usage` | `None` | usage context; remove-asset gate; public view **`unwrap_or_default()`** |

Deliberate asymmetry: missing usage is “empty market”; missing config/listing is “unknown market” and must not silently invent risk parameters. Liquidation curve **requires** live `Spoke` — expiry fail-closes liquidations (availability), which is safer than inventing curve defaults.

---

## 7. Cross-key consistency checklist

| Property | Verdict |
|---|---|
| Config write without validation at storage layer | Expected; validated in `config/asset.rs` |
| Usage write without cap check at storage layer | Expected; enforced in `enforce_spoke_cap` before buffer update |
| Delist requires zero usage | Enforced |
| Deprecate cascades delete assets/usage | No — intentional |
| Same `HubAssetKey` isolated per `spoke_id` | Yes |
| User path cannot forge listing | Yes (`#[only_owner]` only) |
| Counter overflow | Fail-closed `MathOverflow` |
| Zero usage leaves rent-paying empty row | No — pruned |
| Admin can fix corrupted usage | **No** — gap |
| Admin can restore expired `Spoke(id)` | **No** — gap |
| Views renew shared TTL | Yes — mitigation and zombie-prolongation |

---

## 8. Tests / formal anchors

| Check | Location |
|---|---|
| Shared TTL re-arm on spoke read | `contracts/controller/tests/storage/spoke.rs` (`try_get_spoke_renews_shared_ttl_on_read`) |
| SpokeAsset set/get/remove | same (`spoke_asset_discrete_key_roundtrip`) |
| Usage zero prune | same (`spoke_usage_prunes_zero_entry`) |
| Cap + usage integration | `tests/test-harness/tests/controller/spoke_caps.rs`; `contracts/controller/tests/spoke.rs` |
| Listing / deprecate rules | Certora `certora/controller/spec/spoke_rules.rs`; harness `controller/spoke.rs` |
| Usage missing-row policy | A080; Certora comments around `SpokeUsage` asymmetry |

---

## 9. Cross-agent links

| Peer | Agreement |
|---|---|
| A029 | Same shared vs instance discipline; spoke keys are the market half of protocol storage |
| A034 | Owns broader TTL matrix; A028 supplies spoke-specific expiry/zombie cases |
| A017 | `renew_account` does not defend spoke shared keys — complementary residual |
| A076 | Usage buffer semantics sound; durable key prune matches persist of zeros |
| A080 | Missing usage → under-count; A028 adds TTL-driven missing/zombie rows and lack of reconcile admin |
| A033 | Usage durable write before events — orthogonal; keys themselves are source of truth |

---

## 10. Verdict

**Defended:** key discriminant isolation; typed accessor enclosure; owner-only config mutation; soft-deprecate vs hard-delist split; usage zero-prune; fail-closed config/listing reads; overflow-safe spoke id allocation.

**Partial / residual:** shared spoke key TTL lifecycle vs user `renew_account`; unrecoverable expired `Spoke(id)` at the same id; no admin usage reconcile; view-driven TTL can preserve zombie cap usage.

**Not found:** cross-variant key collision; user-writable config keys; storage-level type confusion through public API; delist while non-zero usage without `#309`.

Recommended follow-ups (for synthesis A108/A110, not in-scope code changes here): permissionless or owner `renew_spoke_keys(spoke_id, hubs…)` analogous to NFT renew; owner `reconcile_spoke_usage` / force-zero under pause; document keeper view cadence for `get_spoke` / `get_spoke_asset` / `get_spoke_usage` in ops runbooks; consider whether `set_spoke` should be restorable for existing ids after archival.
