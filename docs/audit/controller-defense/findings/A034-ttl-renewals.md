# A034 — TTL renewals: instance vs account vs view skips

- Agent: A034
- Theme: T2 (storage lifecycle; overlaps T1 renew_account auth — A017; T7 Cache constructors — A008 / A086 / A093)
- Severity: info
- Status: defended
- Paths:
  - `common/src/ttl.rs` (`renew_instance`)
  - `common/src/constants/shared.rs:57-81` (three TTL classes)
  - `contracts/controller/src/context/mod.rs:39-62` (`Cache::new` vs `new_view`)
  - `contracts/controller/src/storage/protocol.rs:3-6,154-205` (`renew_controller_instance`, `get_*/set_*` touch renew)
  - `contracts/controller/src/storage/account.rs:71-103,257-270` (`write_side_map`, `renew_user_account`)
  - `contracts/controller/src/positions/mod.rs:218-252` (`persist_account_positions` co-renew)
  - `contracts/controller/src/account.rs:228-250` (`renew_account`, delegate verbs)
  - `contracts/controller/src/keepers.rs:21-92,147-176` (keeper `Cache::new` + threshold account renew)
  - `contracts/controller/src/lib.rs:65-69,515-516` (`renew_then!`, view `new_view`)
  - `contracts/controller/src/views.rs` (all helpers use `new_view` or no Cache)
  - `contracts/controller/src/governance.rs:101-103` (`accept_ownership`)
- Defense: Three explicit TTL classes (instance / shared / user) with threshold+bump constants; mutators renew instance via `Cache::new` or `renew_then!` / direct `renew_controller_instance`; account sibling co-renew is centralized in `renew_user_account` after position writes; views use `Cache::new_view` so they never bump instance TTL; `get_user` / `get_shared` still touch-renew present keys (INV-STOR-01); owner-gated `renew_account` lifts controller user keys + NFT Owner to the user window (INV-STOR-02b).
- Gap: none that breaks storage-lifetime invariants. Residuals: (1) views still pay touch-renew rent on user/shared keys when below threshold — intentional INV-STOR-01, not instance grief; (2) `write_side_map` skips per-write renew and relies on caller `renew_user_account` — compensated on all persist paths; (3) `add_delegate` / `remove_delegate` renew instance + only the Delegates key via `set_user`, not siblings — by design, full co-renew is `renew_account`; (4) permissionless `update_account_threshold` renews account TTLs without being owner (A015) — maintenance charity, no accounting mutation; (5) NFT Owner asymmetry vs OZ passive reads remains operational (INV-STOR-02c/d), owned by A017 / NFT path, not this inventory.
- Impact: Incorrect instance renew on views would force every read caller to fund 180-day instance bumps (fee surprise / availability optics), not fund theft. Missing account co-renew after a side write could strand a sibling key while another stays live — closed by `persist_account_positions` / keeper / `renew_account`. Stranded NFT Owner while controller state lives is the INV-STOR-02 residual (restore/renew ops), not a controller write bug.
- Evidence: INV-STOR-01, INV-STOR-02a–d; SEED Cache fact; A008 / A017 / A021 / A015; unit `renew_controller_instance_re_extends_instance_ttl`, `renew_user_account_co_renews_all_live_siblings`, `set_supply_positions_does_not_renew_sibling_ttls`; harness renew_account / NFT TTL suite.
- Opinion: The split is the right shape for Soroban rent: mutators keep the contract instance alive; views refuse to be an instance-rent vector; account keys co-renew as a sibling set on write tails and on explicit owner renew. Treat any new view path that calls `Cache::new`, or any position persist that writes via `write_side_map` without a following `renew_user_account`, as a regression.

## Method

1. Inventory TTL constants and `common::ttl::renew_instance`.
2. Diff `Cache::new` vs `new_view`; enumerate every production call site of each.
3. Trace `renew_controller_instance`, `renew_then!`, and direct renews outside Cache.
4. Trace `renew_user_account` callers and contrast with `get_user` / `set_user` / `write_side_map`.
5. Map view entrypoints: which skip instance renew, which still touch-renew persistent keys.
6. Cross-check INV-STOR-01/02, A008, A017, A021, A015 so this file invents inventory — not a second auth review of `renew_account`.

---

## 1. TTL class taxonomy

From `common/src/constants/shared.rs` (`ONE_DAY_LEDGERS = 17_280`):

| Class | Threshold | Bump | Storage | Typical keys |
|---|---|---|---|---|
| Instance | 5d (`TTL_THRESHOLD_INSTANCE`) | 180d (`TTL_BUMP_INSTANCE`) | `env.storage().instance()` | Pool, aggregators, NFT addr, limits, pause/owner, counters |
| Shared | 5d (`TTL_THRESHOLD_SHARED`) | 180d (`TTL_BUMP_SHARED`) | persistent | Spoke, SpokeAsset, SpokeUsage, Hub, PositionManager, BlendPoolAllowed |
| User | 30d (`TTL_THRESHOLD_USER`) | 120d (`TTL_BUMP_USER`) | persistent | AccountMeta, SupplyPositions, BorrowPositions, Delegates |

```11:15:common/src/ttl.rs
pub fn renew_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}
```

Controller aliases this as `storage::renew_controller_instance`. Persistent renewals go through `renew_persistent_key` → `extend_ttl(key, threshold, bump)`. Soroban no-ops the bump when remaining TTL is still above threshold, so repeated touches on fresh keys are cheap.

---

## 2. `Cache::new` vs `Cache::new_view`

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
        Cache { /* empty maps */ }
    }
}
```

| Constructor | Instance TTL | Intended callers |
|---|---|---|
| `Cache::new` | **renew** | Monetary / keeper / strategy / liquidation mutators |
| `Cache::new_view` | **skip** | `views.rs` helpers + `get_market_index` |

`new` is `renew_controller_instance` then `new_view` — no other behavioral difference. SEED and A086 state the same fact.

### 2.1 Production `Cache::new` sites (instance renew)

| Module | Functions |
|---|---|
| `positions/supply.rs` | `process_supply`, `process_withdraw` |
| `positions/debt.rs` | `process_borrow`, `process_repay` |
| `positions/liquidation/mod.rs` | liquidate path, `process_clean_bad_debt` |
| `strategies/*` | flash_loan, flash_position, multiply, swap_debt, swap_collateral, repay_debt_with_collateral, migrate_blend |
| `keepers.rs` | update_indexes, claim_revenue, update_account_threshold, recapitalize |
| `markets.rs` | `upgrade_liquidity_pool_params` |

`flash_loan` renews instance even though it never touches account keys — correct: it is a mutator that must keep the controller instance alive.

### 2.2 Production `Cache::new_view` sites (instance skip)

| Location | Entrypoints / helpers |
|---|---|
| `views.rs` | health_factor, totals, amounts, positions, attributes, liquidation estimate / collateral, LTV collateral, market indexes detailed |
| `lib.rs:515` | `get_market_index` |

No production mutator uses `new_view`.

---

## 3. Instance renew without `Cache`

Not every mutator builds a Cache. Parallel funnels:

| Funnel | Mechanism | Examples |
|---|---|---|
| `renew_then!` | `renew_controller_instance` then admin body | All `ControllerAdmin` setters, pause/unpause/upgrade/migrate, hub/spoke/asset config, pool/NFT deploy/upgrade, `force_socialize_bad_debt` |
| Direct | `storage::renew_controller_instance` | `renew_account`, `add_delegate`, `remove_delegate`, `accept_ownership` |
| Cache | §2.1 | Position / strategy / keeper paths |

```65:69:contracts/controller/src/lib.rs
macro_rules! renew_then {
    ($env:ident, $body:expr) => {{
        storage::renew_controller_instance(&$env);
        $body
    }};
}
```

**Instance reads do not renew.** `get_pool`, `get_price_aggregator`, `get_min_borrow_collateral_usd_wad`, etc. are bare `instance().get` — views like `get_pool_address` / `price_aggregator` therefore neither bump instance TTL nor construct a Cache. Instance lifetime depends on mutator traffic (and owner/keeper activity), not on read spam.

---

## 4. Account renew: `renew_user_account`

```257:270:contracts/controller/src/storage/account.rs
pub(crate) fn renew_user_account(env: &Env, account_id: u64) {
    let persistent = env.storage().persistent();
    let keys = [
        ControllerKey::AccountMeta(account_id),
        ControllerKey::SupplyPositions(account_id),
        ControllerKey::BorrowPositions(account_id),
        ControllerKey::Delegates(account_id),
    ];
    for key in &keys {
        if persistent.has(key) {
            renew_user_key(env, key);
        }
    }
}
```

Missing keys are skipped (`has` gate) — empty supply/debt maps that were `remove`d do not get a zombie extend. Threshold/bump = user class (30d / 120d).

### 4.1 Call sites

| Caller | When | Also renews instance? | NFT Owner? |
|---|---|---|---|
| `persist_account_positions` | After every supply/debt side write via finalize | Via earlier `Cache::new` | No |
| `account::renew_account` | Explicit owner rent | Yes (direct) | Yes (`nft_renew_call`) |
| `keepers::sync_account_thresholds` | Every successful sync attempt (even if stamps unchanged) | Via `Cache::new` | No |

`strategy_finalize` → `finalize_position_flow` → `persist_account_positions` covers all strategy tails with `PositionSides::Both` (+ optional empty cleanup). Liquidation finalizes victim `Both` and Credit receiver `Supply`; both co-renew.

### 4.2 Side writers deliberately skip per-key renew

```89:103:contracts/controller/src/storage/account.rs
fn write_side_map<...>(env: &Env, key: &ControllerKey, map: &Map<HubAssetKey, V>) {
    let persistent = env.storage().persistent();
    if map.is_empty() {
        persistent.remove(key);
    } else {
        persistent.set(key, map);
    }
}
```

No `set_user` here — so a supply write alone would not bump debt/meta/delegates. Unit test `set_supply_positions_does_not_renew_sibling_ttls` locks that. Compensation: callers always follow with `renew_user_account` (persist) or call it first (keeper sync). Meta/delegate paths still use `set_user` (self-renew only). Documented in A021.

### 4.3 Touch renew on read (`get_user`)

```192:205:contracts/controller/src/storage/protocol.rs
pub(super) fn get_user<V: ...>(env: &Env, key: &ControllerKey) -> Option<V> {
    let value: Option<V> = env.storage().persistent().get(key);
    if value.is_some() {
        renew_persistent_key(env, key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
    }
    value
}
```

Any successful load of meta / supply / debt / delegates extends **that key only**. Views that call `try_get_account` / `get_supply_positions` therefore still renew user TTLs when keys exist and are below the 30-day threshold. That is INV-STOR-01 ("renew when read or written"), not a violation of the `new_view` instance skip.

Same pattern for shared keys via `get_shared` / `set_shared` (5d / 180d): `get_spoke`, `get_spoke_asset`, `get_spoke_usage`, and Cache spoke loaders renew shared entries on read even from view paths.

---

## 5. View skip matrix (what is *not* renewed)

| View surface | Instance | User keys | Shared keys | Notes |
|---|---|---|---|---|
| `views::*` with `Cache::new_view` | skip | touch via `get_user` if loaded | touch if Cache loads spoke/hub | Risk views load account + markets |
| `get_market_index` (`new_view`) | skip | none | none (indexes from pool FFI) | Pool may renew its own state |
| `account_exists` / attributes / positions (storage only) | skip | touch meta (± sides) | none | No Cache |
| `get_spoke` / `get_spoke_asset` / `get_spoke_usage` | skip | none | touch via `get_shared` | Direct storage views |
| `get_pool_address` / `price_aggregator` / `get_min_borrow_collateral_usd` / `is_blend_pool_approved` | skip | none | blend flag touch if approved | Pure instance or shared get |

**Defense intent (A008):** views must not be an instance-rent vector. Caller-funded bumps of *their own queried* user/shared keys when near expiry are acceptable lifecycle maintenance; bumping the whole controller instance on every `get_health_factor` is not.

---

## 6. Explicit `renew_account` (cross-link A017)

Not re-audited here for auth. Lifetime shape only:

```
renew_account
  ├─ renew_controller_instance          # instance
  ├─ require_auth + require_account_owner
  ├─ renew_user_account                 # all live user siblings
  └─ nft_renew_call → PositionNft::renew
        ├─ extend_user_persistent_ttl(Owner, Balance)  # user window
        └─ renew_instance                               # NFT contract instance
```

Pause-open, owner-only (delegates rejected), TTL-only mutations. Complementary permissionless `position-nft::renew` exists for keepers (INV-STOR-02b). Controller path is the paired lift for controller keys + NFT in one tx.

Delegate verbs renew **instance** then mutate Delegates via `set_user` (self TTL only) — they do not call `renew_user_account`. Full sibling co-renew remains an owner/`persist`/keeper concern.

---

## 7. Keeper side effect vs owner renew

`update_account_threshold` → `sync_account_thresholds` calls `renew_user_account` after pass filters (meta present, non-empty supply, resolvable NFT owner), **before** stamp refresh, and even when no risk params change (A015 §5.4).

| Path | Who can renew account keys | NFT Owner lift |
|---|---|---|
| `renew_account` | owner only | yes |
| `update_account_threshold` | any authenticated keeper | no |
| Position finalize | whoever can run the mutator | no |
| Views `get_user` | any reader (touch, single key) | no (OZ `owner_of` uses shorter window) |

Not a fund-risk gap: renewals cannot mint/burn shares or reassign ownership. Operational note only — permissionless maintenance can subsidize account rent beyond STRIDE I12's owner-centric framing.

---

## 8. End-to-end renew coverage checklist

| Concern | Enforced? | Where |
|---|---|---|
| Mutators renew controller instance | yes | `Cache::new` / `renew_then!` / direct |
| Views skip controller instance renew | yes | `new_view` + bare getters |
| Position writes co-renew live account siblings | yes | `persist_account_positions` |
| Side map write alone does not co-renew siblings | yes (intentional) | `write_side_map` + unit test |
| Owner can explicitly renew account + NFT | yes | `renew_account` |
| Shared protocol keys renew on read/write | yes | `get_shared` / `set_shared` |
| User keys renew on read/write | yes | `get_user` / `set_user` (+ co-renew helper) |
| NFT Owner lifted to user window on mint/renew_account/permissionless renew | yes | INV-STOR-02b |
| Passive NFT `owner_of` only OZ window | yes (dependency) | INV-STOR-02c |
| Flash session temp flag has no TTL bump | yes | A030 — same-tx latch |

---

## 9. Residuals (not gaps in scoped claim)

1. **View touch-renew of user/shared keys** — still occurs; only instance is skipped. Document when reasoning about "read-only = zero rent side effects."
2. **Idle protocol with only views** — instance can age out without mutator/keeper traffic; operational keep-alive, not a code hole.
3. **Delegate path sibling isolation** — adding a delegate does not extend supply/debt TTL; owner should `renew_account` for full lift.
4. **Keeper account renew** — broader than owner-only; charity / maintenance.
5. **INV-STOR-02c/d NFT asymmetry** — unchanged; bots restore or permissionless-renew Owner entries (see A017 residuals).
6. **A093** (manifest: new vs new_view TTL side effects) — this file is the Wave-2 inventory; A093 may restate Cache optics under T7 without new code facts.

---

## 10. Tests / verification anchors

| Check | Location |
|---|---|
| Instance re-extend via `renew_controller_instance` | `contracts/controller/tests/storage/protocol.rs` (`renew_controller_instance_re_extends_instance_ttl`) |
| Co-renew all live siblings | `contracts/controller/tests/storage/account.rs` (`renew_user_account_co_renews_all_live_siblings`) |
| Side write does not co-renew siblings | same file (`set_supply_positions_does_not_renew_sibling_ttls`) |
| Delegates TTL via `renew_user_account` | same file (`renew_user_account_renews_delegates_ttl`) |
| Owner renew / non-owner / balances unchanged | harness `tests/test-harness/tests/controller/account.rs` |
| NFT Owner window closed by `renew_account` | harness `position_nft_ttl_and_ownership_reads.rs` |
| Views use `new_view` (no instance renew by construction) | `views.rs` + `lib.rs:515`; A008 |

---

## 11. Cross-links

- **A017** — `renew_account` auth + TTL-only mutation defense (do not duplicate).
- **A008** — view bounds + `new_view` as rent-grief defense (instance only).
- **A021** — account key layout; side-writer / co-renew design.
- **A015** — keeper threshold path renews account TTL as side effect.
- **A086 / A093** — Cache inventory; constructor TTL side effects under T7.
- **A024** — withdraw path includes `Cache::new` + finalize `renew_user_account` in its write table.
