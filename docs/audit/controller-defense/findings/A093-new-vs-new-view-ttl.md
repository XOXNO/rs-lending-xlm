# A093 — `Cache::new` vs `Cache::new_view` TTL side effects

- Agent: A093
- Theme: T7 (Cache constructors; overlaps T2 TTL inventory A034, T1 view rent-grief A008, T7 inventory A086)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/context/mod.rs:1–6,39–62` (`Cache::new` / `new_view`)
  - `common/src/ttl.rs:11–15` (`renew_instance`)
  - `common/src/constants/shared.rs:57–81` (instance threshold/bump)
  - `contracts/controller/src/storage/protocol.rs:3–6` (`renew_controller_instance` alias)
  - `contracts/controller/src/views.rs` (9× `new_view`)
  - `contracts/controller/src/lib.rs:65–69,509–524` (`renew_then!`; `get_market_index` `new_view`; bare instance getters)
  - `contracts/controller/src/{positions,strategies,keepers,markets}.rs` (18× production `Cache::new`)
  - `contracts/controller/src/account.rs:228–250` + `governance.rs:101–103` (direct instance renew, no Cache)
  - `contracts/controller/tests/storage/protocol.rs:73–94` (instance re-extend unit)
- Defense: The **only** durable side effect that differs between the two constructors is controller **instance** TTL renew. `Cache::new` calls `storage::renew_controller_instance` then delegates to `new_view`. `new_view` builds empty memo maps / buffers and never touches storage. Every production mutator that builds a Cache uses `new`; every production view that builds a Cache uses `new_view`. No production mutator is on `new_view`. Views therefore cannot force a 180-day controller-instance bump (A008 rent-grief defense). Parallel mutator funnels (`renew_then!`, direct `renew_controller_instance`) cover admin / ownership / delegate paths that never construct a Cache. Persistent user/shared touch-renew on later `get_user` / `get_shared` is orthogonal and intentional (INV-STOR-01); it is not gated by the constructor choice.
- Gap: none that breaks constructor TTL policy. Residuals: (1) no dedicated unit test asserting `new_view` leaves instance TTL unchanged (construction correctness is by code inspection + exhaustive call-site matrix; instance renew itself is unit-tested via `renew_controller_instance`); (2) views still touch-renew **user/shared** keys they read when below threshold — not an instance grief vector; (3) idle protocol with only views can age out instance storage — operational keep-alive via mutators/keepers/admin, not a code hole (A034 residual #2; STRIDE DoS.6); (4) permissionless keepers on `Cache::new` still bump instance even for empty Vec / no-op loops — caller-funded rent charity / annoyance, not theft (A015/A039/A102 adjacency); (5) nested monetary reentry that constructs a fresh `Cache::new` renews instance again in the same tx — Soroban no-ops when above threshold, harmless.
- Impact: **No fund theft, share mint, undercollateralized exit, or auth bypass** from constructor TTL choice. Wrong constructor would be an **availability / fee** regression: (a) a view on `Cache::new` would let any reader subsidize (and spam-pay for) 180-day instance bumps — fee surprise / rent-grief optics, not redirection of protocol funds; (b) a mutator on `new_view` would skip the INV-STOR-01 instance keep-alive for that call — instance could still be kept alive by other mutators, but that path would fail the “privileged/mutating traffic renews instance” contract. Under today’s graph neither misroute exists. Practical blast radius of residuals ≈ **negligible / operational**.
- Evidence: Exhaustive `Cache::new` / `Cache::new_view` grep under `contracts/controller/src` (18 mutator, 10 view); SEED Cache fact; INV-STOR-01; STRIDE I14 / DoS.6; peers A008, A034, A086, A015, A039, A104 adjacency; unit `renew_controller_instance_re_extends_instance_ttl`.
- Opinion: **Agree with A034 / A008 / A086:** the split is the right Soroban rent shape and is correctly wired on every production Cache site. Treat “new view path calls `Cache::new`” and “new money path calls `Cache::new_view`” as hard regressions. A093 does not invent new code facts beyond A034’s inventory; it deepens the T7 constructor differential, side-effect matrix, and misrouting checklist so Wave-6 Cache synthesis (A104) can close the A093 hole.

---

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, README finding format, `AGENT_MANIFEST` Wave 6 (A093), peers **A034**, **A008**, **A086**, adjacency **A015 / A039 / A088 / A090 / A091 / A104**.
2. Diff `Cache::new` vs `new_view` at source; trace `renew_controller_instance` → `common::ttl::renew_instance` → instance `extend_ttl`.
3. Enumerate **every** production `Cache::new` and `Cache::new_view` under `contracts/controller/src` (exclude `tests/`).
4. Classify non-Cache instance renew funnels (`renew_then!`, direct calls) so “views skip Cache renew” is not confused with “nothing renews instance.”
5. Separate constructor side effects from later path side effects (`get_user` / `get_shared` / persist / pool FFI / oracle).
6. Check auth-vs-renew ordering, empty-keeper / claim-only renew, reentry double-construct, and test coverage of the skip.
7. No production Rust edited. No git operations (COORDINATION).

---

## 1. Constructor differential (sole durable difference)

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

Module docs state the same contract (`context/mod.rs:1–4`): `new` renews instance TTL; `new_view` serves read-only entry points.

| Aspect | `Cache::new` | `Cache::new_view` |
|---|---|---|
| Instance `extend_ttl` | **Yes** — `renew_controller_instance` first | **No** |
| Memo / buffer init | Via `new_view` | Empty maps / `None` / empty Vecs |
| Persistent reads | None at construct | None at construct |
| Persistent writes | None at construct | None at construct |
| Account / spoke / event mutation | None at construct | None at construct |
| Intended callers | Monetary / keeper / strategy / liquidation mutators (+ `upgrade_liquidity_pool_params`) | `views.rs` helpers + `get_market_index` |

**Structural fact:** `new` ≡ `renew_controller_instance` + `new_view`. There is no second behavioral fork (no feature flag, no Certora override of these constructors under `context/`). SEED and A086 state the same fact; A034 §2 is the Wave-2 inventory this T7 file deepens.

### 1.1 What `renew_controller_instance` does

```11:15:common/src/ttl.rs
pub fn renew_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}
```

Controller re-exports as `storage::renew_controller_instance` (`protocol.rs:3–6`).

| Constant | Value | Ledgers (approx) |
|---|---|---|
| `ONE_DAY_LEDGERS` | 17_280 | 1 day |
| `TTL_THRESHOLD_INSTANCE` | 5 × day | ~86_400 |
| `TTL_BUMP_INSTANCE` | 180 × day | ~3_110_400 |

Soroban extends only when remaining TTL is below the threshold; repeated renews on a fresh instance are cheap no-ops for rent accounting. Unit `renew_controller_instance_re_extends_instance_ttl` locks re-extend after aging past threshold.

### 1.2 What rides on the controller instance

Instance keys (all extended together by one `extend_ttl` on the instance footprint):

| Key family | Role |
|---|---|
| `Pool`, `SwapAggregator`, `PriceAggregator`, `PositionNft`, `Accumulator` | Protocol pointers |
| `PositionLimits`, `MinBorrowCollateralUsd`, `AppVersion` | Limits / floor / version |
| `LastSpokeId`, `LastHubId` | Counters |
| Ownable / Pausable OZ instance entries | Owner, pause flag (via OZ storage, same instance footprint) |

A successful `Cache::new` therefore keeps **all** of the above alive for the 180-day bump window when below the 5-day threshold — including keys the mutator never reads (e.g. Accumulator on a supply path; A039 notes claim’s renew similarly keeps the whole instance). That is lifecycle maintenance (INV-STOR-01), not a value mutate.

Persistent keys (`Hub`, `Spoke*`, account sides, managers, Blend allowlist) are **not** extended by this call. They use `get_shared` / `set_shared` / `get_user` / `set_user` / `renew_user_account` (A034 §§4–5).

---

## 2. Exhaustive production call sites

Grep under `contracts/controller/src` (excluding `tests/`): **18× `Cache::new`**, **10× `Cache::new_view`**. Zero production mutators on `new_view`; zero production views on `new`.

### 2.1 `Cache::new` — instance renew (18)

| Module | Function | Notes |
|---|---|---|
| `positions/supply.rs:53` | `process_supply` | After `require_authorized_caller` |
| `positions/supply.rs:174` | `process_withdraw` | After auth / gates |
| `positions/debt.rs:48` | `process_borrow` | After auth |
| `positions/debt.rs:91` | `process_repay` | After auth |
| `positions/liquidation/mod.rs:58` | liquidate path | Mutator |
| `positions/liquidation/mod.rs:239` | `process_clean_bad_debt` | Mutator |
| `strategies/flash_loan.rs:32` | `process_flash_loan` | Mutator; no account keys; still renews instance |
| `strategies/flash_position.rs:80` | flash position | Mutator |
| `strategies/multiply.rs:132` | multiply | Mutator |
| `strategies/swap_debt.rs:53` | swap debt | Mutator |
| `strategies/swap_collateral.rs:51` | swap collateral | Mutator |
| `strategies/repay_debt_with_collateral.rs:56` | repay-with-collateral | Mutator |
| `strategies/migrate_blend.rs:67` | migrate blend | Mutator |
| `keepers.rs:21` | `update_indexes` | Auth then renew |
| `keepers.rs:32` | `claim_revenue` | Auth then renew; may be sole controller durable *lifecycle* effect (A039) |
| `keepers.rs:53` | `recapitalize` | Auth then renew |
| `keepers.rs:86` | `update_account_threshold` | Auth then renew; later may `renew_user_account` |
| `markets.rs:94` | `upgrade_liquidity_pool_params` | Owner/admin-gated mutator using Cache for pool address / sync |

`flash_loan` is the clearest “mutator without account writes still renews instance” exemplar — correct for INV-STOR-01 instance keep-alive.

### 2.2 `Cache::new_view` — instance skip (10)

| Location | Helper / entrypoint | Public surface |
|---|---|---|
| `views.rs:31` | `health_factor` | `get_health_factor`, `is_liquidatable` |
| `views.rs:62` | `collateral_amount_for_hub_asset` | `get_collateral_amount` |
| `views.rs:86` | `borrow_amount_for_hub_asset` | `get_borrow_amount` |
| `views.rs:136` | `liquidation_collateral_available` | `get_liquidation_collateral` |
| `views.rs:155` | `get_all_market_indexes_detailed` | `get_market_indexes_detailed` |
| `views.rs:205` | `liquidation_estimations_detailed` | `get_liquidation_estimate` |
| `views.rs:255` | `total_collateral_in_usd` | `get_total_collateral_usd` |
| `views.rs:275` | `total_borrow_in_usd` | `get_total_borrow_usd` |
| `views.rs:286` | `ltv_collateral_in_usd` | `get_ltv_collateral_usd` |
| `lib.rs:515` | inline | `get_market_index` |

### 2.3 Views that never construct a Cache (also skip instance renew)

These entrypoints neither call `Cache::new*` nor `renew_controller_instance`:

| Entrypoint | Mechanism |
|---|---|
| `account_exists` / `get_account_attributes` / `get_account_positions` | Direct `get_user` / meta (user touch-renew only) |
| `get_spoke` / `get_spoke_asset` / `get_spoke_usage` | Direct `get_shared` (shared touch-renew) |
| `get_pool_address` / `price_aggregator` / `get_min_borrow_collateral_usd` | Bare `instance().get` — **no** instance TTL bump |
| `is_blend_pool_approved` | Shared get |
| `get_app_version` | Instance get without renew |

**Critical optics:** instance *reads* do not renew. Protocol instance lifetime depends on mutator / admin / keeper / owner traffic, not on read spam. That is the rent-grief defense A008 names; A093 confirms every Cache-bearing view uses the skip constructor.

### 2.4 Instance renew without Cache (parallel funnels)

Not every mutator builds a Cache. Coverage checklist (A034 §3 restated for T7 completeness):

| Funnel | Mechanism | Examples |
|---|---|---|
| `renew_then!` | `renew_controller_instance` then admin body | Admin setters, pause/unpause/upgrade/migrate, hub/spoke/asset config, pool/NFT deploy/upgrade, `force_socialize_bad_debt` |
| Direct | `storage::renew_controller_instance` | `renew_account`, `add_delegate`, `remove_delegate`, `accept_ownership` |
| Cache | §2.1 | Position / strategy / keeper / pool-param upgrade |

So “views use `new_view`” does **not** mean “instance is never renewed.” It means **read callers are not the renew vector.**

---

## 3. Side-effect matrix (construction alone vs later path)

### 3.1 At construction time

| Side effect | `new` | `new_view` |
|---|---|---|
| Controller instance `extend_ttl` | Yes | No |
| Persistent value write | No | No |
| Persistent key create/delete | No | No |
| User / shared TTL bump | No | No |
| Pool / oracle / NFT FFI | No | No |
| Event publish | No | No |
| In-memory memo population | No (empty) | No (empty) |

Construction alone never mutates protocol accounting. The only durable difference is instance lifecycle.

### 3.2 Later on the same invocation (orthogonal to constructor)

| Later action | Typical on `new` paths | Typical on `new_view` paths | Durable? |
|---|---|---|---|
| `get_user` / `try_get_account` | Yes | Yes (risk / position views) | Touch-renew **that** user key if present |
| `get_shared` / spoke pin | Often | Sometimes (LTV restamp, risk) | Touch-renew shared key |
| `persist_account_positions` → `renew_user_account` | Finalize tails | Never | User sibling co-renew |
| `persist_spoke_usage` | Money paths | Never (A008 / A091) | Shared usage write |
| Pool mutation FFI | Money / keeper | Simulate `get_bulk_indexes` only | Pool contract state / its own TTL |
| Hard `fetch_prices` | Money | Risk views | Aggregator instance TTL (STRIDE I25); controller instance untouched by oracle call itself |
| Soft `fetch_prices_status` | Rare | Detailed indexes view | Aggregator; observational |
| `emit_position_batch` | Finalize | Never | Events only |

**Read-only ≠ zero rent side effects.** `new_view` only guarantees **no controller instance bump**. User/shared touch-renew and cross-contract aggregator TTL remain possible. A034 residual #1 / A008 agree. Do not claim “views have zero storage lifecycle effects.”

### 3.3 Cross-contract TTL spillover (not constructor-controlled)

STRIDE I14 / I25: price-reading views may extend the **aggregator’s** instance TTL. That is independent of `Cache::new` vs `new_view`. Controller constructor policy does not (and should not) suppress remote contract rent; it only refuses to make controller instance a grief sink for view spam.

---

## 4. Threat / regression analysis

### 4.1 Rent grief via views — **mitigated**

| Attack | Without `new_view` | With `new_view` |
|---|---|---|
| Spam `get_health_factor` / indexes | Each call could fund 180d controller instance bump when below threshold | Instance TTL unchanged; caller still pays CPU / possible user-key touch / oracle fees |
| Spam bare `get_pool_address` | N/A (never renewed on get) | Same — no bump |

This is the A008 / A104 “instance rent grief via views” defense. Severity of the *absence* of the defense would be **low** (fee / availability optics), not fund loss — but the defense is present and consistently applied.

### 4.2 Mutator accidentally on `new_view` — **not present**

Would skip instance keep-alive for that call. Accounting could still succeed; instance might age if *all* traffic were miswired views. Today: **zero** production mutator sites use `new_view`. Regression gate for future PRs.

### 4.3 View accidentally on `new` — **not present**

Would reintroduce the grief vector. Today: **zero** production view sites use `new`. `get_market_index` is the easy footgun (inline in `lib.rs`) and correctly uses `new_view`.

### 4.4 Auth ordering vs renew

Surveyed money/keeper paths authorize (or `require_authorized_caller`) **before** `Cache::new`. Failed auth therefore does not renew instance. Successful auth then pays for instance renew even if a later gate panics — caller-funded; no third-party grief of *their* balance beyond the caller’s own fees. Empty keeper Vec still constructs Cache after auth (A015/A102) — intentional cheap renew / annoyance, not drain.

### 4.5 Reentry / nested `Cache::new`

Flash-guarded monetary reentry that starts a **new** entrypoint builds a fresh `Cache::new` (A007 / A089 adjacency). That renews instance again. Above-threshold no-op → no rent amplification. Does not inherit outer memos. Constructor TTL policy is per-Cache, not per-tx singleton.

### 4.6 Idle protocol / views-only traffic

If only views and bare getters run for >~5 days of ledger aging past last bump, instance may fall below threshold and eventually archive (STRIDE DoS.6 residual). Mitigation is operational: keepers (`update_indexes` / `claim_revenue`), user mutators, admin `renew_then!`, or `renew_account`. Not a reason to put views on `Cache::new`.

### 4.7 Claim / flash_loan “TTL-only controller write”

A039: `claim_revenue` may have **no** controller durable *value* write; instance renew via `Cache::new` is the sole controller storage lifecycle effect. Same class as `flash_loan` (no account persist). Both correctly use `new`, not `new_view`, because they are mutators / keepers that must keep the instance alive.

---

## 5. Invariant and STRIDE mapping

| Id / row | How constructors relate |
|---|---|
| **INV-STOR-01** | Renew when read/written; mutators renew instance via `Cache::new` / `renew_then!` / direct; views renew only the persistent keys they touch |
| **INV-STOR-02\*** | NFT Owner asymmetry is **out of scope** for constructor choice (A017 / A034); neither `new` nor `new_view` lifts NFT Owner |
| **STRIDE DoS.6** | Persistent TTL expiry; instance renewed on privileged/mutating calls — matches `new` / not `new_view` |
| **STRIDE I14** | Views: no protocol state; price views may extend aggregator TTL — controller instance skip holds |
| **STRIDE I12** | `renew_account` is the explicit owner renew path (direct instance renew + user co-renew) — parallel to Cache, not a view |

---

## 6. Tests / verification anchors

| Check | Status | Location |
|---|---|---|
| Instance re-extend via `renew_controller_instance` | **Present** | `tests/storage/protocol.rs` (`renew_controller_instance_re_extends_instance_ttl`) |
| Views constructed with `new_view` | **By inspection** | `views.rs` + `lib.rs:515`; A008 |
| Mutators constructed with `new` | **By inspection** | §2.1 exhaustive table |
| Unit: `Cache::new` bumps instance TTL | **Indirect** (via renew helper, not via `Cache::new` wrapper) | Same protocol test |
| Unit: `Cache::new_view` leaves instance TTL unchanged | **Missing** | Residual § Gap (1) |
| Account co-renew / side-write isolation | Present (A034) | `tests/storage/account.rs` — not constructor-specific |

**Test gap (optimization-note / hygiene):** a small unit that ages instance below threshold, calls `Cache::new_view`, asserts TTL unchanged, then `Cache::new`, asserts bump to `TTL_BUMP_INSTANCE`, would lock the constructor contract against refactor slips. Not a security hole today — call-site matrix is clean.

---

## 7. Misrouting checklist (for future PRs / A104)

Treat as **fail CI / review** if any of:

1. A new `views.rs` helper or `lib.rs` view entrypoint calls `Cache::new`.
2. A new monetary / keeper / strategy / liquidation path calls `Cache::new_view`.
3. `new` gains side effects beyond `renew_controller_instance` + `new_view` (e.g. eager storage loads) without an ADR.
4. `new_view` gains any `extend_ttl` / persist / emit / pool-mutate call.
5. A “read-only” helper is documented as zero-rent while still using `get_user` / `get_shared` without noting touch-renew.

Optional lint shape: forbid `Cache::new` under `views.rs`; forbid `Cache::new_view` under `positions/`, `strategies/`, `keepers.rs` (allow tests).

---

## 8. Residuals (not gaps in defended claim)

1. **No `new_view`-skips-TTL unit test** — hygiene; construction correctness held by source + call-site inventory.
2. **View touch-renew of user/shared keys** — intentional INV-STOR-01; not instance grief.
3. **Idle views-only instance aging** — operational keep-alive; do not “fix” by putting views on `new`.
4. **Keeper empty-Vec / no-op claim still renews instance** — caller-funded; A015/A039/A102.
5. **Nested `Cache::new` renews again** — threshold no-op; harmless.
6. **A034 owns full TTL taxonomy** — A093 does not re-litigate account co-renew, `write_side_map`, or NFT Owner asymmetry.

---

## 9. Cross-links / agreements

| Peer | Relationship |
|---|---|
| **A034** | Wave-2 TTL inventory; A093 is the T7 constructor deep-dive A034 §9 item 6 anticipated. **Agree** on defended split and residuals. |
| **A008** | Names `new_view` as rent-grief defense. **Agree.** |
| **A086** | Inventory states `new` renews / `new_view` does not. **Agree.** |
| **A015 / A039 / A102** | Keeper / claim paths on `Cache::new` even when value writes empty. **Agree** — correct mutator classification. |
| **A088 / A090 / A091** | Note `new` → `new_view` delegation; memos unrelated to TTL. **Agree.** |
| **A104** | Listed A093 as coverage hole; adjacency already judged view/mutator TTL **defended**. This filing closes the hole with **defended / info**. |
| **A017** | `renew_account` auth + TTL-only — parallel funnel, not Cache constructors. |

**Disagreements:** none. No `disagreements/A093-vs-*.md` filed.

---

## 10. Verdict

`Cache::new` and `Cache::new_view` differ by exactly one durable side effect: controller instance TTL renew. Production wiring matches intent on all 28 Cache construction sites. Views cannot instance-rent-grief; mutators and parallel renew funnels keep INV-STOR-01 instance lifecycle. Status **defended**, severity **info**. Remaining work is optional unit coverage of the skip and review lint against constructor misrouting — not a protocol fund-risk fix.
