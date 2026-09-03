# A073 — Interest model / market params read-side trust

- Agent: A073
- Theme: T4 (input validation) / T1 (trust boundary TB8 controller↔pool); adjacency T7 (`pool_sync_data` memo)
- Severity: info
- Status: defended
- Paths:
  - Sync read FFI: `contracts/controller/src/external/pool.rs:147-153` (`fetch_pool_sync_data` → `LiquidityPoolClient::get_sync_data`); `contracts/pool/src/lib.rs:298-301`; `contracts/pool/src/storage.rs:48-54` (`load_sync_data`)
  - Cache: `contracts/controller/src/context/pool.rs:19-28` (`cached_pool_sync_data`); `context/mod.rs` (`pool_sync_data` map)
  - Mutator consumer: `contracts/controller/src/strategies/flash_position.rs:87-94` (`params.is_flashloanable`)
  - View consumers: `contracts/controller/src/views.rs:53-98` (`params.asset_decimals` for unscale)
  - Admin consumer: `contracts/controller/src/config/asset.rs:71-73` (direct `fetch_pool_sync_data` → `require_cap_within_asset_domain`)
  - Write / SoT: `contracts/controller/src/markets.rs:64-104` (`create_liquidity_pool`, `upgrade_liquidity_pool_params`); `contracts/pool/src/ops/market.rs:17-61` (`create` / `replace_rate_model` + `verify`); `contracts/pool/src/storage.rs:74-95` (`write_rate_model` leaves `asset_id` / `asset_decimals`)
  - Types / verify: `common/src/types/pool.rs:16-216` (`MarketParamsRaw`, `InterestRateModel::verify`); governance propose `contracts/governance/src/validate/asset.rs:42-62`, `op.rs:265-296`
  - Index path (not sync blob): `fetch_pool_bulk_indexes` / mutation `MarketIndexRaw` + `put_market_index` (A038 / A077 / A088)
- Defense: Durable IRM and market params live **only** on the pool under `PoolKey::Params`. The controller never stores a competing rate model. Production controller paths that need “as of now” indexes use `get_bulk_indexes` (simulate over sync) or post-mutation DTOs — **not** raw `PoolSyncData.state` indexes. Safety-critical rate / utilization / reserve / flash-fee enforcement runs inside the owner-gated pool (`require_utilization_below_max`, accrual `accrue_step`, cash-flash `is_flashloanable` + fee). Controller sync-field consumption is **narrow**: (1) `is_flashloanable` once, **before** any pool leg on `flash_position`; (2) `asset_decimals` on view unscale and spoke-cap domain checks. Rate-curve fields (`base_borrow_rate`…`max_utilization`, `reserve_factor`, `flashloan_fee`) have **zero** reads under `contracts/controller/src` for gating or valuation. Writes that populate params are `#[only_owner]` → pool `verify` (and governance propose-time `validate_market_creation` / `InterestRateModel::verify` under correct ownership).
- Gap: (1) **Write asymmetry (ops)** — controller `create_liquidity_pool` checks only `params.asset_id == asset` + hub active; it does **not** re-fetch token `decimals()` or clamp `MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS`. Governance propose does. Direct owner (or mis-wired owner) can persist decimals that pass pool `verify` (`<= WAD_DECIMALS`) but disagree with the SAC — subsequent sync/mutation decimals are **self-consistent and wrong vs token**. (2) **No re-verify on read** — `fetch_pool_sync_data` returns the stored blob without `MarketParamsRaw::verify`; correct under honest pool Wasm, broken if `upgrade_pool` ships lying `get_sync_data` (Sensitive trust root; A009). (3) **Sync memo non-invalidation** — fill-once `cached_pool_sync_data` (A088/A104); safe today because the only mutator reader is pre-leg. (4) **Events omit flash fields** — `CreateMarketEvent` / `UpdateMarketParamsEvent` flatten curve + reserve but omit `is_flashloanable` / `flashloan_fee` / `asset_decimals` (ops observability, not on-chain gate). (5) **Certora** — `get_sync_data_summary` draws verify-shaped nondets; still independent of bulk-index draws historically (suite-review / A035) — verification epistemology, not production WASM.
- Impact: No path found where the controller **mis-applies** a hostile or stale interest-rate curve read from sync to mint shares, bypass HF, or under-count caps. Index / rate manipulation classes stay on the pool accrual surface (INV-IDX-*, ADR-0016) and STRIDE **Tamper.6** residual **Low** holds from the controller read-side. Blast radius of wrong sync **boolean/decimals** under current graphs: `flash_position` false-allow/deny for one debt market in one tx, or view/admin domain skew — account/tx-local or config hygiene, not protocol SoT fork. Wrong create-time decimals (owner-direct) can desync ray accounting from token units for that market’s lifetime — **listing / create trust**, same class as A055 listing residuals, not a mid-tx sync-cache bug.
- Evidence: INV-IDX-01..05; ADR-0003 / ADR-0016; formulas.md interest sections; STRIDE Tamper.6 / I10; threat-model flashloanable listing note; peers A038, A044, A045, A077, A086, A088, A094, A104, A009, A035; pool tests `flows.rs` rate-model replace; harness max-utilization / flash_position flashloanable pins; Certora `index_rules` / `rates_rules` / `get_sync_data_summary` verify-shaped assumes.
- Opinion: **Read-side interest-model trust is defended by non-use:** the controller does not interpret the IRM for risk or money movement; it trusts pool-enforced accrual and returns. Treat sync as a **params/flags snapshot** for a tiny consumer set, not as an alternate index SoT. Optional hardening (not fund-critical today): (a) controller `create_liquidity_pool` reuse governance `validate_market_creation` (or shared helper) so owner-direct matches propose-time decimals checks; (b) document “never gate post-leg safety on `cached_pool_sync_data`”; (c) emit `is_flashloanable` / decimals on market events for indexers. Confirms A107’s provisional **CONFIRM Low** on Tamper.6 for the controller corpus.

## Scope and method

1. Read `shared/COORDINATION.md`, `SEED.md`, AGENT_MANIFEST A073, README finding format; skim peers A038, A044, A045, A077, A086, A088, A094, A104, A009, A035, A102 (A073 unfiled), A107 Tamper.6.
2. Confirmed `A073-interest-model-read-trust.md` absent.
3. Inventoried every production `cached_pool_sync_data` / `fetch_pool_sync_data` / `get_sync_data` / `MarketParamsRaw` / `InterestRateModel` touch under `contracts/controller/src`.
4. Separated **write-side verify** (create / update_params) from **read-side consumers** (sync blob fields actually used).
5. Cross-checked that risk totals, spoke caps at mutation time, and HF gates consume **indexes + prices + positions**, not IRM slopes from sync.
6. Out of primary claim: accrual arithmetic correctness (IDX / Certora rates suite), mid-tx forgotten `put_market_index` (A094), cash-flash pullback (A044), spoke listing flags (A064), oracle freshness (A065).

---

## 1. Trust model (who owns which numbers)

```
                    DURABLE PARAMS (cross-tx)
┌──────────────────────────────────────────────────────────┐
│  Liquidity Pool persistent storage                       │
│    PoolKey::Params(hub)  — IRM + flags + asset_decimals  │
│    PoolKey::State(hub)   — cash, shares, indexes, time   │
│  Writers: create / write_rate_model / commit after accrue│
└───────────────────────────▲──────────────────────────────┘
                            │ get_sync_data (raw, no simulate)
                 ┌──────────┴──────────┐
                 │ Controller Cache    │  fill-once pool_sync_data
                 │ (ephemeral)         │
                 └──────────┬──────────┘
        Used fields today:  │
          is_flashloanable  ├── flash_position pre-leg gate
          asset_decimals    ├── views unscale; admin cap domain
          (curve / reserve / flash fee / state indexes) ──► UNUSED on controller mutators
```

**Indexes for risk** enter through a different door:

| Door | Accrues? | Controller use |
|---|---|---|
| `get_sync_data` → `state.*_index` | **No** (raw committed) | **Not** used for HF / caps / liquidation |
| `get_bulk_indexes` | Simulate via `simulate_update_indexes(now, sync)` | Prefetch / views / pre-pool risk |
| Mutation DTO `market_index` | Post-accrue write path | `put_market_index` + entry caps (A077) |

So “interest model read trust” splits into two questions:

1. Does the controller **trust sync params** for decisions? → only flashloanable + decimals, narrowly.
2. Does the controller **re-derive rates** from a sync IRM? → **no**.

Tamper.6 (extreme rates / time gaps) is therefore a **pool accrual** property (INV-IDX + ADR-0016), not a controller sync-parse bug.

---

## 2. Payload inventory — what `PoolSyncData` carries

From `common/src/types/pool.rs`:

`MarketParamsRaw`: `max_borrow_rate`, `base_borrow_rate`, `slope1..3`, `mid_utilization`, `optimal_utilization`, `max_utilization`, `reserve_factor`, `is_flashloanable`, `flashloan_fee`, `asset_id`, `asset_decimals`.

`PoolStateRaw` (sync `.state`): `supplied`, `borrowed`, `revenue`, `borrow_index`, `supply_index`, `last_timestamp`, `cash`.

`InterestRateModel` is the curve + flash subset **without** `asset_id` / `asset_decimals` — what `update_params` / `write_rate_model` patch.

### 2.1 Verify on write (not on read)

`InterestRateModel::verify` / `MarketParamsRaw::verify` enforce:

- `base_borrow_rate >= 0`; monotone slopes through `max_borrow_rate`; `max_borrow_rate` in `(base, MAX_BORROW_RATE_RAY]`
- `0 < mid < optimal < RAY`; `optimal <= max_utilization <= RAY`
- `reserve_factor < BPS`; `flashloan_fee <= MAX_FLASHLOAN_FEE_BPS`
- `asset_decimals <= WAD_DECIMALS` (18)

Called on:

| Path | Who calls `verify` |
|---|---|
| Pool `create_market` | Pool `ops::market::create` |
| Pool `update_params` | Pool `replace_rate_model` **after** accrue-commit |
| Governance `CreateLiquidityPool` | `validate_market_creation` (+ token decimals match + `MIN..=MAX`) |
| Governance `UpgradeLiquidityPoolParams` | `args.params.verify` |
| Controller `create_liquidity_pool` | **No local verify** — relies on pool |
| Controller `upgrade_liquidity_pool_params` | **No local verify** — relies on pool |
| `fetch_pool_sync_data` / Cache hit | **Never** |

Fail-closed on bad write: pool panics before persist. Stale/invalid **reads** are not re-checked — by design for a controller-owned pool.

### 2.2 Immutability after create

`write_rate_model` patches curve + flash fields only; **`asset_id` and `asset_decimals` are sticky** for the market lifetime. Controllers that memoize decimals from sync therefore cannot see a mid-tx decimals flip via `upgrade_liquidity_pool_params` (A088 note confirmed).

---

## 3. Exhaustive controller consumers of sync / params

### 3.1 `cached_pool_sync_data` / `fetch_pool_sync_data`

| Call site | Field(s) | When | Safety-critical? |
|---|---|---|---|
| `flash_position.rs` | `params.is_flashloanable` | Mutator, **before** account legs / pool borrow | **Yes** — policy gate for caller-chosen receiver debt mint |
| `views.rs` collateral/borrow amount | `params.asset_decimals` | `Cache::new_view` | No — display unscale; HF views use indexes+prices without this decimals for share valuation |
| `config/asset.rs` upsert spoke asset | `params.asset_decimals` | Admin, direct fetch (no Cache) | Config hygiene — i128-safe cap domain vs ray rescale |
| (none) | rate curve / `reserve_factor` / `flashloan_fee` / `max_utilization` / `state.*` | — | **No production reader** |

Grep under `contracts/controller/src` for `base_borrow_rate`, `slope1`, `reserve_factor`, `max_utilization`, `flashloan_fee` hits **events + admin write forwarding only**, not mutator gates.

### 3.2 Parallel trust: decimals on money paths

Entry spoke-usage scaling uses **`PoolPositionMutation.asset_decimals`** from the pool mutation report (`positions/supply.rs`, `debt.rs` → `LegDirection::Entry`), which the pool copies from **live** `cache.params().asset_decimals` — the same Params key sync would read, but **fresh at mutation time**, not from a fill-once memo.

Cap enforcement after legs uses mutation indexes (A077), not sync state.

### 3.3 What enforces IRM economics (pool-side)

| Control | Location | Controller re-check? |
|---|---|---|
| Accrual / index bounds | `interest.rs` + `common::rates::*` | No — consumes returned indexes |
| `max_utilization` | `pool/src/guards.rs` on borrow / some withdraws | No |
| `reserve_factor` split | accrual `accrue_step` | No |
| Cash flash `is_flashloanable` + fee | `pool/ops/flash.rs` | Controller cash `flash_loan` skips sync pre-check (A044); pool enforces |
| Strategy / `flash_position` debt mint fee | Pool `create_strategy` / borrow | `flash_position` requires market flashloanable on controller; fee policy separate (ADR-0020 / A045) |

---

## 4. Path deep-dives

### 4.1 `flash_position` — sole mutator sync gate

```87:94:contracts/controller/src/strategies/flash_position.rs
    // Strategy debt through `multiply` stays open on such a market because the
    // funds only ever reach the governance-owned router. Here they reach a
    // caller-chosen contract, which is exactly what the flag denies.
    assert_with_error!(
        env,
        cache.cached_pool_sync_data(debt).params.is_flashloanable,
        FlashLoanError::FlashloanNotEnabled
    );
```

Ordering relative to money:

1. Auth / pause / mode / Wasm receiver / not controller|pool
2. **Sync `is_flashloanable`**
3. Load account, validate collaterals/refunds, prefetch
4. Pool strategy/borrow legs, callback, measure, finalize + HF

No later re-read of sync for this flag. Pool does **not** re-assert `is_flashloanable` on ordinary borrow/strategy mint — so this controller check **is** the gate (unlike cash `flash_loan`, where the pool `prepare` path asserts live). Trust: stored Params bit + fill-once memo before any same-Cache param mutation (admin upgrade does not share this Cache with user `flash_position`).

Residual if governance enables flashloanable on a non-exact asset: listing class (threat-model / A044 / A055) — not a sync-read bug.

### 4.2 Views — decimals for human amounts

`collateral_amount_for_hub_asset` / `borrow_amount_for_hub_asset` combine:

- `cached_market_index` ← bulk simulate (accrued projection)
- `cached_pool_sync_data(...).params.asset_decimals` ← raw params

Wrong decimals skew **displayed** token amounts; they do not change stored scaled shares or on-chain HF (risk totals use share × index × price in WAD/RAY space without this sync decimals field — A065 adjacency).

### 4.3 Spoke asset caps — admin domain check

```71:73:contracts/controller/src/config/asset.rs
    let market = fetch_pool_sync_data(env, &storage::get_pool(env), &hub_asset);
    require_cap_within_asset_domain(env, args.supply_cap, market.params.asset_decimals);
    require_cap_within_asset_domain(env, args.borrow_cap, market.params.asset_decimals);
```

Bypasses Cache (always live FFI). Bounds caps so `calculate_scaled_cap` cannot overflow when rescaling with those decimals. If Params decimals were wrong at create, this check and later mutation scaling share the same wrong SoT — consistent internal math, wrong vs chain token metadata.

### 4.4 Create / upgrade write path (feeds what reads trust)

**Create (`markets::create_liquidity_pool`):**

1. `require_hub_active`
2. `params.asset_id == asset` else `#WrongToken`
3. `pool_create_market_call` → pool `params.verify` + init indexes at RAY

Missing vs governance: token `decimals()` equality, `MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS` (pool allows `0..=18` via `<= WAD_DECIMALS` only).

**Upgrade params (`upgrade_liquidity_pool_params`):**

1. `Cache::new` (TTL)
2. `pool_update_indexes_call` — accrue under **old** model
3. `pool_update_params_call` — pool accrues again via `renewed_market` commit, then `model.verify`, `write_rate_model`
4. Event from rate model (no flash/decimals fields)

Controller pre-accrue + pool replace accrue is intentional (old curve through current ledger). Sync memos on **other** invocations see new curve after commit; same invocation’s Cache is discarded after admin return.

Timelock: both create and upgrade-params are **Standard** `AdminOperation` when owned by governance (A009). Extreme rate grief is delayed config, not an unauthenticated read forge.

---

## 5. Non-goals that look like A073 but are not

| Concern | Owner finding / layer |
|---|---|
| Durable index SoT / Cache `put` | A038, A094 |
| `pool_sync_data` fill-once invalidation | A088, A104 |
| Cap uses mutation index | A077, A081 |
| Cash flash fee + pullback | A044 |
| `flash_position` measure / mode | A045, A018 |
| Hostile pool Wasm after Sensitive upgrade | A009 Tamper.7 |
| Certora sync vs bulk nondet independence | A035, suite-review |
| IRM curve math / chunked accrual | Certora rates / IDX; ADR-0016 |

A073’s distinctive claim: **controller does not need to trust IRM field semantics on the read path because it does not consume them for decisions.**

---

## 6. Attack / residual catalog

| Scenario | Reachable? | Outcome |
|---|---|---|
| User forges sync IRM via calldata | No | Sync is pool FFI; user cannot inject params |
| Stale Cache `is_flashloanable` after same-tx param flip | Not on current graphs | User mutators never call `upgrade_*`; fill-once residual documented (A088) |
| Use raw sync `state.borrow_index` for HF | No code path | Would understate accrued debt if someone added it — checklist item |
| Recompute borrow rate on controller from sync for a gate | No code path | — |
| Owner-direct create with `asset_decimals != token.decimals()` | Yes if owner ≠ careful gov path | Market-lifetime unit desync; gov propose blocks |
| Owner sets `max_utilization` / slopes to grief borrowers | Yes after Standard delay | Economic / availability; pool still verifies well-formedness; not silent share mint |
| Lying `get_sync_data` after malicious pool Wasm | Sensitive upgrade precondition | Protocol-total trust failure (A009) — outside read-side validation |
| Certora rule assumes impossible IRM | Mitigated by verify-shaped summary | Epistemology residual (A035) |

---

## 7. STRIDE / invariant cross-check

| ID | Relation |
|---|---|
| **Tamper.6** | Residual Low affirmed for controller: no read-side IRM reinterpretation; accrual bounds remain pool/common (INV-IDX-01..05). Closes A107’s “provisional; A073 unfiled” caveat for this corpus. |
| **I10** | Index / interest interaction — controller consumes pool projections/mutations. |
| **TB8** | Controller↔pool ownership: Params SoT on pool; controller owner-only mutators. |
| **INV-IDX-*** | Enforced on pool write/accrue; controller does not store competing indexes/params. |
| **DoS.8** | Utilization blocks are pool-enforced from Params `max_utilization` — not a controller sync parse. |

---

## 8. Tests and formal evidence (pointers)

- Pool: `contracts/pool/tests/flows.rs` — `update_params` / rate-model replace / verify rejects
- Controller unit: flash_position flashloanable rejection; entrypoints without pool panic on upgrade
- Harness: `max_utilization`, strategy/flash_position edge suites
- Governance: `validate/asset.rs` market creation decimals match
- Certora: `InterestRateModel::verify`-shaped `nondet_market_params` / `get_sync_data_summary`; `simulate_update_indexes` isomorphism rules (indexes, not controller sync gates)

No dedicated harness named “controller re-verifies sync IRM” — correctly absent if the design is non-use.

---

## 9. Remediation / checklist (optional)

| Priority | Item | Why |
|---|---|---|
| P3 | Share `validate_market_creation` (or equivalent) into controller `create_liquidity_pool` | Close owner-direct vs gov propose asymmetry on decimals |
| P3 / docs | Comment on `cached_pool_sync_data`: preflight flags/decimals only; never post-leg | Prevent A088 footgun |
| P4 | Include `is_flashloanable`, `flashloan_fee`, `asset_decimals` on market events | Indexer / ops parity with storage |
| Process | Code review: any new `cached_pool_sync_data` field use must justify trust vs pool live re-read | Especially curve fields or `state` indexes |

None of these are required to keep Tamper.6 / fund-safety claims for **current** call graphs.

---

## 10. Verdict

**Defended (info).**

The interest-model / market-params **read** surface from pool sync data is intentionally thin and correctly placed:

1. Pool Params are the sole durable SoT; controller Cache is ephemeral and does not persist IRM.
2. Controller mutators do not interpret rate-curve fields from sync.
3. The one safety boolean (`is_flashloanable`) is checked before money movement on the one path that needs it.
4. Decimals from sync are view/admin-domain only; money-path decimals come from mutation DTOs tied to the same sticky Params.
5. Write-side `verify` + owner/governance gating bound what sync can ever contain under honest pool Wasm.

Residual severity stays **info** (ops write asymmetry + memo/event hygiene). Do not elevate unless a future path starts gating solvency, caps, or share math on sync IRM fields or raw sync indexes without simulate/mutation refresh.
)
