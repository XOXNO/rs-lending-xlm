# Multi-Hub + Spokes-Only Refactor — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use `- [ ]` checkboxes.
>
> **First action after approval:** copy this file into the repo at `docs/superpowers/plans/2026-06-28-multi-hub-spokes-migration.md` (create `docs/superpowers/plans/`). It is the zero-context source of truth.
>
> **Authority rule for every agent:** all type names, field names/types, enum variants, function signatures, and error names come from **Appendix A — Frozen Contracts**. Copy them verbatim. NEVER invent, rename, reorder fields, add fields, or guess a signature. If something you need is not in Appendix A or a task's Interfaces block, STOP and ask — do not improvise.

**Goal:** Turn rs-lending-xlm into an Aave-v4-shaped design: one pool contract namespaced by `hub_id` (isolated liquidity per hub), a spokes-only risk model (no global market; every account binds a self-contained spoke), and per-account position-manager delegation — keeping the protocol's modularity (per-`account_id` isolation, inlined-snapshot grandfathering).

**Architecture:** Two axes. **Liquidity** on the pool, keyed by `HubAssetKey { hub_id, asset }` (rates, indices, cash, hub caps, flash). **Risk** on the controller, keyed by `(spoke_id, HubAssetKey)` (LTV, liquidation params, spoke caps, flags). An account binds one `spoke_id`; positions are keyed by `HubAssetKey`; the collateral snapshot pins risk params at entry (grandfathering). The same asset on two hubs = two isolated positions that never net and never cross-socialize.

**Tech Stack:** Rust + Soroban SDK (`#[contracttype]`, `Map`, `Vec`, `Address`, `i128`). Crates: `contracts/{pool,controller,governance}`, `common` (repo root `/common`), `interfaces/{pool,controller}`, integration `tests/test-harness`. Fixed-point: `*_ray`=27-dec RAY, `*_bps`=bps, `*_wad`=18-dec WAD; `Ray`/`Bps`/`Wad` in `common::math::fp`.

## Global Constraints

- **Posture: testnet/fresh.** No data migration. Redefine storage types directly; bump `AppVersion`; redeploy + reseed.
- **Delegation: per-account.** `Delegates(account_id) → Vec<Address>`.
- **Premium/`collateral_risk` deferred.** Do not add it.
- **No `receive_shares_enabled`** (no aToken; every supply is collateral). **No stored `deficit`** (`clean_bad_debt` writes down the `(hub,asset)` supply index synchronously via existing `interest.rs::apply_bad_debt_to_supply_index`).
- **`spoke 0` = canonical general spoke** — a normal self-contained spoke auto-created at controller init; accounts default to `spoke_id = 0`. Never a privileged resolution branch.
- **TDD every task:** failing test → watch fail → minimal impl → watch pass → commit. Conventional Commits, subject ≤72 chars.
- **Verification bar before each commit:** `cargo check --all-targets`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p <crate>`. No `unwrap()`/`expect()` outside tests; typed errors; no magic literals.

### Universal guardrails (apply to ALL tasks — violating these is a wrong assumption)
1. **Use Appendix A verbatim.** Exact field names/types and exact function signatures. Do not add/remove/rename/reorder fields. Do not add fields "for convenience."
2. **Phase 0 is behavior-preserving.** `hub_id` is always `0`. NEVER branch on `hub_id`, never read a hub registry, never change any numeric behavior in Phase 0. The only change is the key/coordinate *type*.
3. **Positions store scaled shares** (`*_scaled_ray`), never underlying amounts. Convert with the existing `Ray`/index helpers (`common::rates`, `cache.rs::calculate_scaled_*`). Caps are whole-asset units → `Ray::from_asset(cap, decimals)` as today.
4. **Snapshot is write-once-on-create.** `AccountPositionRaw.{liquidation_threshold_bps, liquidation_bonus_bps, loan_to_value_bps}` are seeded only when a supply position is first created (`Account::get_or_create_supply_position`). NEVER overwrite them on an existing position except through an explicit re-seed path (not in scope here).
5. **One file = one owner.** Two tasks running in parallel MUST NOT edit the same file. The File-Ownership table is authoritative; if your task needs a file owned by a parallel task, they are not parallel — sequence them.
6. **Pricing resolves by `hub_asset.asset`** (token-rooted), independent of `hub_id`. The risk/oracle axis is hub-independent.

---

## Execution Model — dependency graph & parallel tracks

Barriers (`═══`) are hard sync points: everything above must be merged + green before anything below starts. Within a phase, `Track A`/`Track B` run on disjoint file sets (see File-Ownership) and may be done by separate agents concurrently.

```
0.1 HubAssetKey (SOLO — blocks everything)
        │
0.2 Pool re-key (keys + PoolAction)         ← pool + common only
        │
0.4a Controller+interface re-key             ← needs frozen PoolAction
        │
0.4b Harness/test re-key  (PARALLEL per test file: many agents, one file each)
══════════════════════ Phase 0 barrier: full `cargo test` green, hub_id=0, zero behavior change ══════════════════════
1.1 Rename eMode→Spoke + split keys + DEFINE FINAL STRUCTS (SOLO — blocks phase 1)
        │
   ┌────┴────────────────────────┐
Track A: 1.2 flags + liq triple   Track B: 1.3 token-rooted oracle + override
   └────┬────────────────────────┘
1.4a Remove global market; spokes-only resolution; spoke 0 init  (needs A+B)
        │
1.4b Harness/test migration to spokes  (PARALLEL per test file)
══════════════════════ Phase 1 barrier ══════════════════════
2.1 Hub registry + create_hub  →  2.2 list (hub,asset) markets  →  2.3 enable hub_id>0 + isolation tests   (sequential)
══════════════════════ Phase 2 barrier ══════════════════════
   ┌───────────────────────────────┬───────────────────────────────────────────┐
Track C: Phase 3 delegation         Track D: Phase 4 strategies (4.1‖4.2‖4.3 parallel — disjoint strategy files)
   └───────────────────────────────┴───────────────────────────────────────────┘
        (C and D are independent; run concurrently)
```

**Where real parallelism exists:** `1.2 ∥ 1.3`; harness/test migration (`0.4b`, `1.4b`) one-agent-per-test-file; `Phase 3 ∥ Phase 4`; `4.1 ∥ 4.2 ∥ 4.3`. **Phase 0 and Phase 2 are intentionally near-sequential** (shared-type ripple / registry chain) — do not attempt to parallelize their internal tasks; you will collide on `lib.rs`/`pool.rs`.

### File-Ownership (no two parallel tasks share a file)
| Track | Owns (exclusive) |
|---|---|
| 0.2 Pool re-key | `common/src/types/pool.rs`, `contracts/pool/src/**`, `contracts/pool/tests/**` |
| 0.4a Controller re-key | `interfaces/controller/src/**`, `contracts/controller/src/**` (logic), `contracts/controller/tests/**` |
| 0.4b / 1.4b Harness | `tests/test-harness/src/**`, `tests/test-harness/tests/**` (one agent per file) |
| 1.2 flags+liq | `contracts/controller/src/positions/**`, `contracts/controller/src/positions/liquidation_math.rs` |
| 1.3 oracle | `contracts/controller/src/oracle/**`, `contracts/controller/src/storage/instance.rs` (AssetOracle accessor only), `contracts/controller/src/governance/config.rs` (oracle fns only) |
| Phase 3 (C) | `contracts/controller/src/helpers/account.rs`, `validation.rs`, `storage/account.rs` (Delegates), `storage/instance.rs` (PositionManager), `positions/{borrow,withdraw}.rs` (`to` arg) |
| Phase 4 (D) | `contracts/controller/src/strategies/**` |

> 1.2 and 1.3 both *would* touch `types/controller.rs`. To keep them parallel, **1.1 defines ALL final struct fields** (Appendix A) so 1.2/1.3 edit only logic files, never the types file.

### Rename map (apply consistently — old → new)
| Old | New |
|---|---|
| `EModeCategory` / `EModeCategoryRaw` (header) | `Spoke` / `SpokeConfig` |
| `EModeAssetConfig` | `SpokeAssetConfig` |
| `EModeSpokeUsageRaw` | `SpokeUsageRaw` |
| `EModeAssetArgs` | `SpokeAssetArgs` |
| `EModeUsageContext` | `SpokeUsageContext` |
| `e_mode_category_id` (field/param) | `spoke_id` |
| `ControllerKey::EModeCategory(u32)` | `ControllerKey::Spoke(u32)` |
| `ControllerKey::LastEModeCategoryId` | `ControllerKey::LastSpokeId` |
| `get/set_emode_category`, `get/set/remove_emode_asset`, `increment_emode_category_id` | `…spoke…` equivalents |
| `add_e_mode_category` / `add_asset_to_e_mode_category` / `edit_asset_in_e_mode_category` / `remove_*` | `add_spoke` / `add_asset_to_spoke` / `edit_asset_in_spoke` / `remove_*` |
| `apply_e_mode_to_asset_config` | removed (resolution is self-contained — see 1.4a) |

---

## Context — why

Today: one central pool (`ControllerKey::Pool`) with per-asset `MarketParamsRaw`/`PoolStateRaw`; a controller with a **global market** (`Market(Address)`→`MarketConfig`, `e_mode_category_id==0`) and **eMode categories** overlaying 5 risk fields on top. The eMode category is already a self-contained spoke (`EModeAssetConfig` carries full LTV/threshold/bonus/caps; `emode_caps.rs` enforces spoke≤hub). We add the two things we lack: **multiple hubs** (isolated liquidity + bad-debt per hub) and **spokes-only** (drop the privileged global market). The enabling coordinate is `HubAssetKey`. Risk stays keyed by `(spoke_id, HubAssetKey)`; the account carries `spoke_id`; price stays token-rooted with optional per-spoke override. Bulk endpoints change coordinate `Address → HubAssetKey` and `e_mode_category: u32 → spoke_id: u32`.

---

## Phase 0 — Introduce `HubAssetKey`, behavior-preserving re-key (hub_id = 0)

### Task 0.1 — Define `HubAssetKey` (SOLO, blocker)
**Files:** Create in `common/src/types/pool.rs` (near `PoolKey`); export through `common/src/types/mod.rs`.
**Produces:** `common::types::pool::HubAssetKey` (Appendix A §1).
**Guardrails:** exact name/fields from §1; derive `Clone, Debug, Eq, PartialEq`.
- [ ] Test in `common`: equal when `(hub_id,asset)` equal; unequal when `hub_id` differs.
- [ ] Fail → add struct → pass → commit `feat: add HubAssetKey coordinate type`.

### Task 0.2 — Re-key `PoolKey` + `PoolAction` + pool internals (pool/common only)
**Files (owned):** `common/src/types/pool.rs` (`PoolKey` §2, `PoolAction` §3, `MarketStateSnapshot.asset→hub_asset`), `contracts/pool/src/{cache.rs,views.rs,utils.rs,lib.rs,interest.rs?}`, `contracts/pool/tests/**`.
**Consumes:** `HubAssetKey` (§1).
**Produces:** `PoolKey::{Params,State}(HubAssetKey)`; `PoolAction { position, amount, hub_asset: HubAssetKey }`; every pool accessor takes `&HubAssetKey` instead of `&Address`.
**Exact edit sites (do all; ~12):** `cache.rs:39,45` (read), `cache.rs:83` (`save` — store the `HubAssetKey`; add a `hub_asset: HubAssetKey` field to `Cache`, drop the `params.asset_id` rebuild), `views.rs:12,19`, `utils.rs:35,40,79,96,105,113`, `lib.rs:279,285,299` (`create_market`), `lib.rs:89` (`load_position` dispatch on `action.hub_asset`), `lib.rs:457-461` (`create_strategy` destructure).
**Guardrails:** `hub_id` always `0`; do not gate flash on the new fields yet (Phase 2). Update `MarketParamsRaw` to include `is_flashloanable`/`flashloan_fee_bps` (§4) **as inert fields** now (both `From` impls `:97-135`, `verify:51-76`, `MarketParams:80-95`, and every test `market_params` builder e.g. `tests/flows.rs:112-129`) — they are wired in Phase 2.
- [ ] Update `tests/flows.rs::{market_params, TestSetup}` to build/seed via `HubAssetKey{0,asset}` (seed at `flows.rs:161-166`).
- [ ] `cargo test -p pool` → fix all sites → green → commit `refactor: key pool by HubAssetKey and add inert flash params (hub_id=0)`.

### Task 0.4a — Re-key controller types, storage, HF loop, endpoints, pool client
**Files (owned):** `interfaces/controller/src/{types/controller.rs, lib.rs}`, `contracts/controller/src/{storage/account.rs, helpers/math.rs, helpers/emode_caps.rs, positions/**, external/pool.rs, cache/**, oracle/price.rs}`, `contracts/controller/tests/**`.
**Consumes:** `PoolAction { …, hub_asset }` (§3), `HubAssetKey` (§1).
**Produces:** `Account.{supply_positions,borrow_positions}: Map<HubAssetKey, _>`; endpoint coordinate `Vec<(HubAssetKey, i128)>` (§5, with `hub_id=0` from callers); `make_pool_action` builds `hub_asset`.
**Pattern (repeat at every site):** any `Map<Address,_>` position map, any `seen: Map<Address,bool>` dedup, any `(Address,i128)` payload, any per-asset HF/oracle loop key → `HubAssetKey`. Pricing still calls by `hub_asset.asset`. Concrete sites: `storage/account.rs:45-124`, `helpers/math.rs:114-182` (HF loop), `helpers/emode_caps.rs` usage map, `positions/{supply.rs:154-168, borrow.rs:102-124, withdraw.rs:109-177, repay.rs:74-86, liquidation.rs:196-238}`, dedup `helpers/utils.rs::aggregate_*`, `positions/mod.rs::make_pool_action:118-128`, `external/pool.rs:11-145`.
**Guardrails:** behavior-preserving; `hub_id=0`; endpoints still take `e_mode_category: u32` here (renamed to `spoke_id` only in Phase 1 — keep param name `e_mode_category` in 0.4a to avoid churn, or accept the rename now if Appendix §5 is followed exactly).
- [ ] `cargo test -p controller` → green → commit `refactor: key controller positions and endpoints by HubAssetKey (hub_id=0)`.

### Task 0.4b — Harness/test re-key (PARALLEL: one agent per test file)
**Files (owned, one per agent):** `tests/test-harness/src/ops/{supply,borrow,withdraw,repay,internal,account}.rs`, then each `tests/test-harness/tests/controller/*.rs`.
**Consumes:** the new endpoint sigs from 0.4a.
**Edit:** harness ops pass `Vec<(HubAssetKey,i128)>` with `hub_id:0`; e.g. `src/ops/supply.rs:26` `ctrl.supply(&addr,&account_id,&0u32,&assets)` → assets keyed by `HubAssetKey`. Add a harness helper `hub_asset(asset) -> HubAssetKey { hub_id:0, asset }`.
- [ ] Per file: make it compile + pass unchanged. Commit per file `test: re-key <file> to HubAssetKey`.

**Phase 0 barrier:** `cargo test` full workspace green; diff is pure type-change; `hub_id=0` everywhere.

---

## Phase 1 — Spokes-only risk model

### Task 1.1 — Rename eMode→Spoke, split keys, DEFINE FINAL STRUCTS (SOLO, blocker)
**Files:** `interfaces/controller/src/types/controller.rs` (apply Rename map; **write the FINAL structs from Appendix A §6–§10 with ALL fields now**), `storage/emode.rs`→spoke accessors **keyed by `(spoke_id, HubAssetKey)`** as two discrete keys `SpokeAsset`/`SpokeUsage` (drop embedded Maps), `storage/instance.rs` (`LastSpokeId`), `cache/mod.rs:251-320`, `helpers/emode_caps.rs` (`SpokeUsageContext`, usage per `(spoke_id, HubAssetKey)`), `emode.rs`, `governance/config.rs` (rename entrypoints), interface `admin.rs`.
**Produces:** final `SpokeConfig` (§7), `SpokeAssetConfig` (§8, all fields incl. `paused/frozen/liquidation_fees_bps/oracle_override`), `SpokeUsageRaw` (§9), `AccountMeta { owner, spoke_id, mode }` (§10), `ControllerKey` final variant set (§11) — but new logic for the new fields lands in 1.2–1.4.
**Guardrails:** define fields exactly per Appendix A; do not yet implement paused/frozen/oracle-override behavior (that's 1.2/1.3) — just the data + storage; keep tests compiling by defaulting new fields in test builders.
- [ ] Update `tests/storage/emode.rs` (now spoke). `cargo test -p controller` green. Commit `refactor: rename eMode→spoke, split spoke-asset/usage keys, define final structs`.

### Task 1.2 — Flags + configurable liquidation curve (Track A)
**Files (owned):** `contracts/controller/src/positions/**`, `positions/liquidation_math.rs`.
**Consumes:** `SpokeAssetConfig.{paused,frozen,liquidation_fees_bps}` (§8), `SpokeConfig.{liquidation_target_hf_wad,health_factor_for_max_bonus_wad,liquidation_bonus_factor_bps}` (§7).
**Edit:** add gates in each `positions/*` validate path — `paused` blocks all; `frozen` blocks supply/borrow, allows repay/withdraw. In `liquidation_math.rs` replace literal `1_020_000_000_000_000_000` (`:419`) and `WAD+WAD/100` (`:431`) with `SpokeConfig` fields; thread `SpokeConfig` through `estimate_liquidation_amount:451`, `calculate_linear_bonus_with_target:383`.
- [ ] Tests: `paused` rejects all four verbs; `frozen` rejects supply/borrow only; custom `liquidation_target_hf_wad` drives the seize target.
- [ ] Commit `feat: spoke paused/frozen gates + configurable liquidation curve`.

### Task 1.3 — Token-rooted oracle + per-spoke override (Track B)
**Files (owned):** `contracts/controller/src/oracle/**`, `storage/instance.rs` (AssetOracle accessor only), `governance/config.rs` (oracle fns only).
**Consumes:** `ControllerKey::AssetOracle(Address)` (§11), `SpokeAssetConfig.oracle_override` (§8).
**Edit:** `set_market_oracle_config:396` writes `AssetOracle(asset)`; `oracle/price.rs::token_price:10-55` + `oracle/mod.rs::price_components:45` resolve `SpokeAsset(spoke_id, hub_asset).oracle_override.unwrap_or(AssetOracle(asset))` — thread `spoke_id` from the account/cache into the resolver.
- [ ] Tests: price from `AssetOracle`; override wins when set.
- [ ] Commit `feat: token-rooted AssetOracle with per-spoke override`.

### Task 1.4a — Remove global market; self-contained resolution; spoke 0 init (needs A+B)
**Files:** delete `storage/market.rs` + `ControllerKey::Market`; remove `MarketConfig`/`MarketStatus`/global `AssetConfigRaw` from `types/controller.rs` (relocate per §6 notes); replace `emode.rs::effective_asset_config:32` with `effective_asset_config` that reads `SpokeAsset(spoke_id, hub_asset)` directly (no overlay) and seeds the snapshot; `cache/mod.rs` drop `cached_market_config`; controller init creates `spoke 0`; `helpers/account.rs::load_or_create_account:52` requires valid `spoke_id` (default 0); `validate_e_mode_asset:51`→`validate_spoke_lists_asset` (asset listed = `SpokeAsset` key exists, spoke not deprecated).
**Guardrails:** spoke 0 is a normal spoke — no special-casing in resolution; listing gate per §"invariants".
- [ ] Tests: account on spoke 0 supplies/borrows a listed `(hub0,asset)`; unlisted `hub_asset` rejected; grandfathering — lower spoke LTV after supply, existing position keeps old LTV.
- [ ] Commit `feat: remove global market; spokes-only resolution + general spoke 0`.

### Task 1.4b — Harness/test migration to spokes (PARALLEL per file)
**Files (owned, one per agent):** `tests/test-harness/src/{presets.rs,fixtures.rs,ops/*}`, each `tests/test-harness/tests/controller/*.rs`.
**Edit:** create a general spoke 0 in the builder; pass `spoke_id`; eMode tests use named spokes.
- [ ] Per file green. Commit per file `test: migrate <file> to spokes`.

**Phase 1 barrier:** no `Market(Address)`; every account binds a spoke; token-rooted pricing; grandfathering intact; harness green.

---

## Phase 2 — Multi-hub activation (sequential)

### Task 2.1 — Hub registry + `create_hub`
**Files:** `storage/instance.rs` (`LastHubId`, `Hub(u32)` accessors), `HubConfig` (§12), interface `admin.rs` + `governance/config.rs::create_hub(env)->u32` (mirror `add_spoke`).
- [ ] Test: id increments; inactive hub blocks ops. Commit `feat: hub registry + create_hub`.

### Task 2.2 — List `(hub,asset)` markets; wire flash
**Files:** `router.rs::create_liquidity_pool:148-203` (+`hub_id` param → `HubAssetKey`), `external/pool.rs::fetch_pool_sync_data:106` (thread `HubAssetKey`), wire `is_flashloanable`/`flashloan_fee_bps` into `MarketParamsRaw` build + `flash_loan` gate, `update_pool_caps:205`/`validate_hub_caps_against_category_spokes:201` per `(hub,asset)`.
- [ ] Test: same asset on hub0 & hub1 with different params; `Params(hub0,asset) != Params(hub1,asset)`. Commit `feat: per-(hub,asset) markets + flash gating`.

### Task 2.3 — Enable `hub_id>0` + isolation tests (keystone)
**Files:** remove any leaked `hub_id==0` assumption.
- [ ] **Isolation test** (`tests/test-harness/tests/controller/multi_hub.rs`): supply USDC@hub0 + USDC@hub1 as two positions; borrow each; assert independent indices, no netting; `clean_bad_debt` on hub0 writes down only `State(hub0,USDC)`.
- [ ] **Cash partition test:** borrow from hub0 cannot draw hub1 `cash`.
- [ ] Commit `feat: enable multi-hub with isolated liquidity and bad-debt`.

**Phase 2 barrier:** multi-hub proven by isolation + cash tests.

---

## Phase 3 — Delegation (Track C, concurrent with Phase 4)

### Task 3.1 — Registry + per-account delegates + auth chokepoint
**Files (owned):** `storage/instance.rs` (`PositionManager(Address)`, `PositionManagerConfig` §13), `storage/account.rs` (`Delegates(u64)→Vec<Address>`), `governance/config.rs::set_position_manager`, owner-gated `add_delegate`/`remove_delegate`, `helpers/account.rs::load_or_create_account:52-101` + `validation.rs:34` → `require_owner_or_delegate` (§14).
- [ ] Tests: owner ok; stranger rejected; registered+opted-in ok; registered-not-opted-in rejected; `repay` open to anyone. Commit `feat: per-account position-manager delegation`.

### Task 3.2 — Destination safety on delegated borrow/withdraw
**Files (owned):** `positions/{borrow.rs:28,withdraw.rs:50}`.
**Edit:** add `to: Option<Address>` to `borrow` (§5); route to `to.unwrap_or(caller)`.
- [ ] Test: delegated borrow with `to=owner` sends to owner. Commit `feat: explicit destination for delegated borrow/withdraw`.

---

## Phase 4 — Strategies (Track D; 4.1‖4.2‖4.3 disjoint files)

### Task 4.1 — `multiply` → spoke + HubAssetKey
**Files (owned):** `strategies/multiply.rs`, `strategies/positions.rs` (shared helpers — coordinate via Appendix A §5).
- [ ] Sig per §5; flash from `debt.hub_id`. Test: leverage on spoke S with `(hub0,COLL)`/`(hub0,USDC)`. Commit `feat: multiply on HubAssetKey+spoke_id`.

### Task 4.2 — `swap_debt` (cross-hub) + `swap_collateral`
**Files (owned):** `strategies/swap_debt.rs`, `strategies/swap_collateral.rs`.
- [ ] Test: `swap_debt(existing=(hub0,USDC), new=(hub1,USDC))` refinances to cheaper hub. Commit `feat: swap_debt/swap_collateral on HubAssetKey`.

### Task 4.3 — `migrate_blend` + remainder
**Files (owned):** `strategies/{migrate_blend.rs,repay_debt_with_collateral.rs,flash_loan.rs}`.
- [ ] Re-key + parity tests. Commit `refactor: remaining strategies on HubAssetKey+spoke_id`.

---

## Verification (end-to-end)

- `cargo check --all-targets --all-features` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean.
- `cargo test -p pool -p controller -p governance -p test-harness` green.
- Keystone `tests/test-harness/tests/controller/multi_hub.rs`: isolation, cash isolation, bad-debt isolation, cross-hub `swap_debt`. Migrated `supply/borrow/withdraw/repay/liquidation/emode` suites green.
- Testnet: redeploy (bumped `AppVersion`); `create_hub`×2; list USDC on both; accounts on spoke 0 + a curated spoke; verify isolated borrows, a liquidation, a hub-scoped `clean_bad_debt`.

## Deferred
Premium/`collateral_risk` (slots in as `collateral_risk` on `SpokeAssetConfig`, `risk_premium_threshold` on `SpokeConfig`/`SpokeUsage`, premium fields on positions — no field above moves); liquidator-chosen collateral leg; per-owner delegation; reinvestment controller; spoke enumeration set.

---

# Appendix A — Frozen Contracts (authoritative; copy verbatim, never invent)

**§1 — `common/src/types/pool.rs`**
```rust
#[contracttype] #[derive(Clone, Debug, Eq, PartialEq)]
pub struct HubAssetKey { pub hub_id: u32, pub asset: Address }
```
**§2 — PoolKey**
```rust
#[contracttype] #[derive(Clone, Debug)]
pub enum PoolKey { Params(HubAssetKey), State(HubAssetKey) }
```
**§3 — PoolAction (entry wrappers unchanged: each holds `action: PoolAction`)**
```rust
#[contracttype] #[derive(Clone, Debug)]
pub struct PoolAction { pub position: ScaledPositionRaw, pub amount: i128, pub hub_asset: HubAssetKey }
```
**§4 — MarketParamsRaw (add 2 fields; PoolStateRaw UNCHANGED)**
```rust
#[contracttype] #[derive(Clone, Debug)]
pub struct MarketParamsRaw {
    pub max_borrow_rate_ray: i128, pub base_borrow_rate_ray: i128,
    pub slope1_ray: i128, pub slope2_ray: i128, pub slope3_ray: i128,
    pub mid_utilization_ray: i128, pub optimal_utilization_ray: i128, pub max_utilization_ray: i128,
    pub reserve_factor_bps: u32,
    pub supply_cap: i128, pub borrow_cap: i128,
    pub is_flashloanable: bool, pub flashloan_fee_bps: u32,   // NEW
    pub asset_id: Address, pub asset_decimals: u32,
}
```
**§5 — Controller endpoint signatures (`interfaces/controller/src/lib.rs`)**
```rust
fn supply(env: Env, caller: Address, account_id: u64, spoke_id: u32, assets: Vec<(HubAssetKey, i128)>) -> u64;
fn borrow(env: Env, caller: Address, account_id: u64, borrows: Vec<(HubAssetKey, i128)>, to: Option<Address>);
fn withdraw(env: Env, caller: Address, account_id: u64, withdrawals: Vec<(HubAssetKey, i128)>, to: Option<Address>) -> Vec<(HubAssetKey, i128)>;
fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>);
fn liquidate(env: Env, liquidator: Address, account_id: u64, debt_payments: Vec<(HubAssetKey, i128)>);
fn multiply(env: Env, caller: Address, account_id: u64, spoke_id: u32, collateral: HubAssetKey, debt_to_flash_loan: i128, debt: HubAssetKey, mode: PositionMode, swap: Bytes, initial_payment: Option<(HubAssetKey, i128)>, convert_swap: Option<Bytes>) -> u64;
fn swap_debt(env: Env, caller: Address, account_id: u64, existing_debt: HubAssetKey, amount: i128, new_debt: HubAssetKey, swap: Bytes);
fn swap_collateral(env: Env, caller: Address, account_id: u64, current: HubAssetKey, amount: i128, new: HubAssetKey, swap: Bytes);
```
**§6 — `Account` (`interfaces/controller/src/types/controller.rs`)**
```rust
#[contracttype] #[derive(Clone, Debug)]
pub struct Account {
    pub owner: Address,
    pub spoke_id: u32,
    pub mode: PositionMode,
    pub supply_positions: Map<HubAssetKey, AccountPositionRaw>,
    pub borrow_positions: Map<HubAssetKey, DebtPositionRaw>,
}
```
**§7 — SpokeConfig**
```rust
#[contracttype] #[derive(Clone, Debug)]
pub struct SpokeConfig {
    pub is_deprecated: bool,
    pub liquidation_target_hf_wad: i128,
    pub health_factor_for_max_bonus_wad: i128,
    pub liquidation_bonus_factor_bps: u32,
}
```
**§8 — SpokeAssetConfig**
```rust
#[contracttype] #[derive(Clone, Debug)]
pub struct SpokeAssetConfig {
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
    pub paused: bool,
    pub frozen: bool,
    pub loan_to_value_bps: u32,
    pub liquidation_threshold_bps: u32,
    pub liquidation_bonus_bps: u32,
    pub liquidation_fees_bps: u32,
    pub supply_cap: i128,
    pub borrow_cap: i128,
    pub oracle_override: Option<MarketOracleConfig>,
}
```
**§9 — SpokeUsageRaw**
```rust
#[contracttype] #[derive(Clone, Debug, Default)]
pub struct SpokeUsageRaw { pub supplied_scaled_ray: i128, pub borrowed_scaled_ray: i128 }
```
**§10 — AccountMeta (positions UNCHANGED: `AccountPositionRaw`/`DebtPositionRaw` as today)**
```rust
#[contracttype] #[derive(Clone, Debug)]
pub struct AccountMeta { pub owner: Address, pub spoke_id: u32, pub mode: PositionMode }
```
**§11 — ControllerKey (final variant set; names authoritative)**
```rust
#[contracttype] #[derive(Clone, Debug)]
pub enum ControllerKey {
    PoolTemplate, Pool, Aggregator, Accumulator, AccountNonce,
    PositionLimits, AppVersion, MinBorrowCollateralUsd,
    LastSpokeId, LastHubId,
    Hub(u32),
    AssetOracle(Address),
    Spoke(u32),
    SpokeAsset(u32, HubAssetKey),
    SpokeUsage(u32, HubAssetKey),
    PositionManager(Address),
    AccountMeta(u64),
    Delegates(u64),
    SupplyPositions(u64),
    BorrowPositions(u64),
}
```
**§12 — HubConfig**
```rust
#[contracttype] #[derive(Clone, Debug)] pub struct HubConfig { pub is_active: bool }
```
**§13 — PositionManagerConfig**
```rust
#[contracttype] #[derive(Clone, Debug)] pub struct PositionManagerConfig { pub is_active: bool }
```
**§14 — auth helper (`helpers/account.rs`)**
```rust
// returns Ok(()) if authorized; typed error otherwise. Called by supply(create path)/borrow/withdraw/strategies; NOT by repay.
fn require_owner_or_delegate(env: &Env, account_id: u64, caller: &Address) -> Result<(), ControllerError>;
// rule: caller == AccountMeta(account_id).owner
//   || (storage::position_manager(caller).map_or(false,|c| c.is_active) && storage::delegates(account_id).contains(caller))
```
**§15 — storage accessor names to ADD (mirror existing style in `storage/`)**
```
instance.rs:  get_last_hub_id/increment_hub_id; get_hub/set_hub(u32, HubConfig);
              get_asset_oracle/set_asset_oracle(Address, MarketOracleConfig);
              get_position_manager/set_position_manager(Address, PositionManagerConfig);
spoke (was emode.rs): get_spoke/set_spoke(u32, SpokeConfig);
              get_spoke_asset/set_spoke_asset/remove_spoke_asset(u32, HubAssetKey, SpokeAssetConfig);
              get_spoke_usage/set_spoke_usage(u32, HubAssetKey, SpokeUsageRaw);
account.rs:   get_delegates/set_delegates(u64, Vec<Address>); add_delegate/remove_delegate.
```

## Self-review
Spec coverage: every locked decision maps to a task. Type consistency: names in Appendix A are reused identically across phases and the rename map. Parallel safety: File-Ownership ensures no shared-file edits across concurrent tracks; 1.1 front-loads all struct fields so 1.2∥1.3 never touch the types file.
