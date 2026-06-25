# MarketId End-to-End Implementation Plan

Status: PLAN (not started). Author: architecture session 2026-06-24.
Scope: introduce an incremental `MarketId` so the same underlying token can be
listed as N independent markets, each with its own `AssetConfig` (controller
risk params) + its own pool ledger (`MarketParams` + `PoolState`), with user
positions keyed by `MarketId`. Five repos: `rs-lending-xlm` (contracts),
`@xoxno/types`, `@xoxno/sdk-js`, `xoxno-az-functions`, `xoxno-api-v2`.

This file is the single source of truth for the change. Every task lists
**Change / Files / Depends / Verify / Executor**. All file:line refs were
captured by source review on 2026-06-24 and must be re-confirmed at edit time
(read-before-write).

---

## 0. The locked idea (end-to-end flow)

```
list market  ──► Controller allocates MarketId (u32, ++counter)
                 ├─ writes AssetConfig under ControllerKey::Market(MarketId)   (risk params + underlying field)
                 ├─ calls Pool.create_market(MarketId, MarketParams)           (own cash/index/rate ledger)
                 ├─ sets MarketOracleConfig under the same MarketId            (per-market oracle)
                 └─ appends MarketId to MarketsByUnderlying(underlying)        (registry)

user action  ──► supply/borrow/withdraw/repay take MarketId
                 ├─ Controller position keyed by (account, MarketId)
                 ├─ Pool ledger mutated for that MarketId only (isolation: own cash)
                 └─ events carry MarketId (+ underlying address kept as a field)

indexer/api  ──► position doc id = `${identifier}_${marketId}`
                 market doc id   = `${marketId}_MARKET_PROFILE`
                 token kept as a field for display/price; per-underlying rollup = SUM over MarketIds
```

Same underlying under two MarketIds = two independent ledgers + two position
docs, settled/liquidated per MarketId, rolled up by underlying for display.
This is Aave V4's multi-hub topology collapsed into one Pool + one Controller
keyed by MarketId (Morpho-singleton packaging). NO premium shares. NO per-market
pool *contract* (one Pool hosts all MarketIds; isolation comes from the key
namespace).

---

## 1. Global design decisions (settle BEFORE Phase 1)

| # | Decision | Recommendation | Owner sign-off? |
|---|---|---|---|
| G1 | `MarketId` type | `u32` plain alias; document the convention. (Newtype adds Soroban `contracttype`/`Map` friction.) | no |
| G2 | Allocator | Controller instance counter `ControllerKey::MarketCount`, mirrors `increment_emode_category_id` (`governance/config.rs:261`). Controller passes id into `pool.create_market`. | no |
| G3 | Registry | `ControllerKey::MarketsByUnderlying(Address) -> Vec<u32>` for discovery + same-underlying policy. | no |
| G4 | Pool multiplicity | Keep ONE Pool contract; key by MarketId. No per-market pool address. | no |
| G5 | Asset address | Demote from map-key to a stored FIELD on `MarketConfig`, `MarketOracleConfig`, pool `MarketParamsRaw`, and every asset-bearing event (sibling of `market_id`). Underlying never disappears — needed for transfers, oracle identity, display. | no |
| **G6** | **Oracle consistency for same-underlying markets** | **Option A: enforce all MarketIds sharing an underlying use one oracle config (registry invariant). Option B: per-market oracle but canonical per-underlying price in HF.** Recommend A. | **YES — required before A7** |
| **G7** | **Same-underlying collateral↔borrow loop** | forbid collateralizing USDC-m1 while borrowing USDC-m2, OR allow with spread cap. | **YES — required before A7** |
| G8 | Doc id scheme | Stellar position `${identifier}_${marketId}`; market `${marketId}_MARKET_PROFILE`; emode `${marketId}_${category}_TOKEN_EMODE_PROFILE`. MVX (shared `LendingAccountProfileDoc`) keeps `${identifier}_${token}` → `marketId` is OPTIONAL on the shared types, chain-branched in the id constructor. | confirm MVX unaffected |
| G9 | Migration | Contract: lazy-on-touch re-key of legacy Address-keyed positions (same pattern as dynamic-config snapshot refresh). Indexer: full cursor reset + reindex + purge/migrate old docs. | accept reindex window |
| **G10** | **Breaking API responses** | `/lending/market/indexes` and `/lending/market/prices` return `Record<token,…>` — keys collide; must re-key by `marketId` (breaking) or nest. | **YES — coordinate with UI** |
| G11 | Naming | az-functions already has a Cosmos id-formatter named `marketId(contractId)` (`stellar-lending-id.util.ts:81`). Rename it (e.g. `marketProfileDocId`) before introducing protocol `marketId` to avoid a footgun. | no |

G6, G7, G10 are the only true blockers needing the owner. The plan proceeds to
those points then pauses for sign-off.

---

## 2. Critical path (cross-repo DAG)

```
Phase 1  rs-lending-xlm (contracts)  ── A1..A10  [worktree-isolated]
                │  (final event + ABI + view shapes frozen here)
                ▼
Phase 2  @xoxno/types  ── B1..B6  ──► publish vTYPES (≥1.0.425)
                │
                ▼
Phase 3  @xoxno/sdk-js ── C1..C3  ──► publish vSDK (≥1.0.155, dep vTYPES)
                │
        ┌───────┴────────┐
        ▼                ▼
Phase 4a az-functions    Phase 4b api-v2     (both depend on vTYPES + vSDK)
   D1..D7                   E1..E7
        │                ▼
        └──► Phase 1 testnet redeploy ──► cursor reset + REINDEX (D7) ──► api cache flush (E5)
                                 ▼
Phase 5  xoxno-ui (downstream consumers of vTYPES/vSDK; NOT in primary scope — see §8)
```

Hard ordering rule (XOXNO multi-repo): contract → types → sdk → consumers. Do
NOT patch consumers with local types. Types is the gate everyone waits on, so
freeze the contract event/ABI/view shapes (A2/A4/A6 + the new views in A5) before
starting B.

---

## 3. Repo A — `rs-lending-xlm` (contracts). The big one.

Execute with **subagent-driven-development in a git worktree** (`feat/market-id`).
Edits are parallel-unsafe; run tasks sequentially, one implementer per task,
task-review after each. Models: standard for A1/A4/A7/A8 (judgment), cheap for
A6/A9 mechanical, most-capable for A7 (policies) review.

Verification bar (per task, scaled): `cargo check -p <crate> --tests` →
`cargo clippy -p <crate> --all-targets -- -D warnings` → targeted tests. Final
gate: workspace tests green (`-p controller --lib`, `-p governance --lib`, pool).
Never `--all-features` (breaks linking w/ certora `_CVT_assert`). Fuzz needs
`--sanitizer=thread -Zbuild-std` on macOS.

### A1 — MarketId primitive, storage keys, allocator, registry
- **Change:** add `pub type MarketId = u32`. Add `ControllerKey::MarketCount`,
  `ControllerKey::MarketsByUnderlying(Address)`; re-type `ControllerKey::Market(Address)` → `Market(MarketId)`.
  Add `storage::increment_market_id`, `get/set/has/try_get_market_config` by `MarketId`,
  `register_market_for_underlying`, `markets_for_underlying`. Add `PoolKey::Params/State(MarketId)`.
- **Files:** `common/src/types/pool.rs` (PoolKey:398-404, MarketParamsRaw:10-28 rename `asset_id`→`underlying`),
  `interfaces/controller/src/types/controller.rs` (ControllerKey:549-568, MarketConfig:268-275 add `underlying`+`market_id`),
  `contracts/controller/src/storage/{market,instance,mod}.rs`.
- **Depends:** —  **Verify:** `cargo check -p common -p controller-interface -p pool-interface --tests`
- **Executor:** general-purpose, standard model.

### A2 — Pool re-key (key, params field, ABI, pool events)
- **Change:** `PoolKey::{Params,State}(MarketId)`. `create_market(market_id, params)`. Every pool fn
  `asset: Address` → `market_id: MarketId` (`supply/borrow/withdraw/repay/update_indexes/add_rewards/
  flash_loan/create_strategy/seize_position/claim_revenue/update_params/update_caps`, +views
  `bulk_get_indexes(Vec<MarketId>)`, `get_sync_data(MarketId)`, etc.). `PoolAction.asset`→`market_id`.
  Transfers read `params.underlying`. Pool events `PoolMarketStateEvent`/`PoolMarketParamsEvent`/
  `StrategyFeeEvent` gain `market_id` (`contracts/pool/src/events.rs:11-63`).
- **Files:** `interfaces/pool/src/lib.rs` (ABI), `contracts/pool/src/{lib,cache,events,views}.rs`,
  `common/src/types/pool.rs` (PoolAction:410-416, bulk index types).
- **Depends:** A1.  **Verify:** `cargo check -p pool -p pool-interface --tests` → `cargo test -p pool --lib`.
- **Executor:** general-purpose, standard model.

### A3 — Controller re-key (market, positions, emode, oracle)
- **Change:** position maps `Account.{supply,borrow}_positions: Map<MarketId,_>`
  (`controller.rs:277-289`); `EModeCategoryRaw.{assets,usage}: Map<MarketId,_>` (`:116-123`);
  `MarketConfig`+`underlying`+`market_id`; oracle key by MarketId; emode-asset key
  `(category_id, MarketId)`. `storage/{account,emode}.rs` re-key.
- **Files:** `interfaces/controller/src/types/controller.rs`, `contracts/controller/src/storage/{account,emode,market}.rs`,
  `contracts/controller/src/cache/mod.rs` (market_indexes cache keyed by MarketId:38-198, put/cached_market_index).
- **Depends:** A1.  **Verify:** `cargo check -p controller --tests`.
- **Executor:** general-purpose, standard model.

### A4 — Listing flow (allocate + thread MarketId)
- **Change:** in `create_liquidity_pool` (`router.rs:153-205`): after dedup guards,
  `let market_id = storage::increment_market_id(env)`; dedup now allows same underlying
  (drop `AssetAlreadySupported` on underlying; keep per-MarketId uniqueness); `pool_create_market_call(market_id, params)`;
  `MarketConfig{ underlying: asset, market_id, ... }` stored by `market_id`;
  `MarketOracleConfig::pending_for(asset, decimals)` under `market_id`;
  `register_market_for_underlying(asset, market_id)`. Entrypoint signature: return `(Address pool, u32 market_id)`
  or just `market_id`. `set_market_oracle_config`/`set_oracle_tolerance`/`disable_token_oracle`
  (`governance/config.rs:454/538/550`) take `market_id`. emode add/edit (`config.rs:296-438`) take `market_id`.
- **Files:** `contracts/controller/src/router.rs`, `contracts/controller/src/governance/config.rs`,
  `contracts/controller/src/external/pool.rs:11-13` (`pool_create_market_call(market_id, params)`).
- **Depends:** A1,A2,A3.  **Verify:** `cargo check -p controller -p governance --tests`.
- **Executor:** general-purpose, standard model.

### A5 — Position flows + pool client + new views
- **Change:** `positions/{supply,borrow,withdraw,repay}.rs`, `strategies/*`, `positions/liquidation*.rs`
  resolve & pass `market_id`; `external/pool.rs` all calls keyed by MarketId. Add controller views:
  `get_market(market_id) -> {underlying, …}`, `markets_for_underlying(addr) -> Vec<u32>`,
  `get_all_market_indexes_detailed(markets: Vec<MarketId>)` returns rows carrying `market_id`
  (api-v2 + az-functions depend on these — freeze their shapes here).
- **Files:** `contracts/controller/src/positions/*`, `strategies/*`, `external/pool.rs`, `views/*`.
- **Depends:** A4.  **Verify:** `cargo test -p controller --lib` (supply/borrow/withdraw/repay).
- **Executor:** general-purpose, standard model.

### A6 — Events: add `market_id` to all asset-keyed events
- **Change:** add `market_id: u32` to controller events `CreateMarketEvent`(events.rs:253),
  `UpdateMarketParamsEvent`(:270), `FlashLoanEvent`(:388), `UpdateAssetConfigEvent`(:398),
  `UpdateAssetOracleEvent`(:405), `UpdateEModeAssetEvent`(:436), `RemoveEModeAssetEvent`(:444),
  `OracleDisabledEvent`(:526), `InitialMultiplyPaymentEvent`(:461); and into the vec deltas
  `EventDepositDelta`/`EventBorrowDelta` (events.rs:310-374) for `UpdatePositionBatchEvent`(:376)
  — **market_id goes on each delta, not top-level (easy to miss)**. Update all emit sites
  (router.rs:186/240, cache/mod.rs:200-244, strategies/*, governance/config.rs:235/356/451/489/543/559).
  Pool events done in A2.
- **Files:** `contracts/controller/src/events.rs` + emit sites; in-file test ctors (events.rs:619-903).
- **Depends:** A4,A5.  **Verify:** `cargo test -p controller --lib events`; `tests/controller/events.rs`.
- **Executor:** general-purpose, cheap model (mechanical), but review carefully (downstream depends on shapes).

### A7 — Policies (G6 oracle consistency, G7 loop guard) — GATED on owner sign-off
- **Change:** G6 invariant in `set_market_oracle_config`/listing: same underlying ⇒ shared oracle (Option A).
  G7 guard in borrow validation: reject/limit borrowing an underlying held as collateral under another MarketId
  (uses `MarketsByUnderlying`). Add error variants.
- **Files:** `contracts/controller/src/governance/config.rs`, `validation.rs`, `positions/borrow.rs`, `common/src/errors.rs`.
- **Depends:** A4; **G6+G7 owner decisions**.  **Verify:** new unit tests for both policies.
- **Executor:** general-purpose, standard model; **review on most-capable model**.

### A8 — Migration (lazy-on-touch)
- **Change:** on first touch of an account, detect legacy Address-keyed `SupplyPositions/BorrowPositions`
  entries and re-key to the MarketId allocated for that (underlying) at its base market; write registry.
  Bounded per-call (no unbounded loop). Governance one-shot helper for market docs.
- **Files:** `contracts/controller/src/helpers/account.rs`, `storage/account.rs`, a `migration.rs`.
- **Depends:** A3,A4.  **Verify:** regression test: legacy doc → touched → re-keyed, balances preserved.
- **Executor:** general-purpose, standard model.

### A9 — Tests / fuzz / certora / harness
- **Change:** fix the wiring choke point `tests/test-harness/src/setup/builder.rs` first
  (`create_liquidity_pool` + `set_market_oracle_config` now MarketId), then op helpers
  `tests/test-harness/src/ops/*` + `core/lending.rs`. Update unit (`contracts/{pool,controller}/src/tests.rs`,
  controller event ctors), integration (`tests/integration/lib/{protocol,core}.sh` + flows + scenarios),
  fuzz (`tests/fuzz/fuzz_targets/{flow_e2e,flow_strategy,pool_native,rates_and_index}.rs`,
  `tests/fuzz/src/{context,decode,invariants}.rs`), certora harness+specs
  (`certora/controller/harness/external/pool.rs`, `certora/{controller,pool}/spec/*`, confs).
- **Depends:** A1-A8.  **Verify:** `cargo test --workspace`; fuzz smoke; `make certora-*` green gate.
- **Executor:** split: cheap model for mechanical harness/test edits; standard for fuzz model + certora specs.

### A10 — Testnet redeploy + governance scripts + ABI dump
- **Change:** rebuild wasms (`stellar contract build`), redeploy controller+pool, re-list markets via
  the new MarketId flow, update `services/keeper` keys if market-keyed, update integration CLI scripts,
  refresh the SDK ABI/contract addresses. Record new addresses + start ledger (for reindex).
- **Depends:** A1-A9 green.  **Verify:** integration e2e green on testnet; capture redeploy ledger.
- **Executor:** general-purpose, standard model; human-gated deploy.

---

## 4. Repo B — `@xoxno/types` (currently 1.0.424). Source of truth; publish before C/D/E.

Verify bar: `pnpm typecheck && pnpm lint && pnpm test`. One implementer per task; mechanical.

- **B1 — Event DTOs:** add `marketId: number` to `StellarMarketStateSnapshot`(stellar-lending-events.dto.ts:13),
  `StellarEventPositionDelta`(:81), `StellarCreateMarketEvent`(:193), `StellarUpdateMarketParamsEvent`(:238),
  `StellarPoolMarketParamsUpdate`(:270), `StellarFlashLoanEvent`(:324), `StellarUpdateAssetConfigEvent`(:346),
  `StellarUpdateAssetOracleEvent`(:358), `StellarStrategyFeeEvent`(:426), `StellarOracleDisabledEvent`(:520),
  `StellarUpdateEModeAssetEvent`(:380)/`StellarRemoveEModeAssetEvent`(:395). Keep `asset`. **Depends:** A6 frozen.
- **B2 — Args:** `marketId` on `MarketParamsRawDto`(stellar-lending-admin-args.dto.ts:54), supply/borrow/withdraw/repay
  arg DTOs (stellar-lending-args.dto.ts). **Depends:** A5/A6 frozen.
- **B3 — Doc id constructors (collision fix):** `LendingAccountProfileDoc` (lending-account-profile.ts:159/163)
  + `LendingMarketProfileDoc` (lending-market-profile.doc.ts:267) + emode doc. Add OPTIONAL `marketId?: number`
  field; chain-branch the id: Stellar `${identifier}_${marketId}` / `${marketId}_MARKET_PROFILE`, MVX unchanged
  (G8). **Depends:** B1.  **Critical:** this is the data-collision fix.
- **B4 — Summary + indexes:** `marketId` on `SingleLendingAccountToken`(lending-account-summary.ts:27),
  `LendingIndexesDto`. **Depends:** B1.
- **B5 — Cache keys:** add `marketId` to the 13 token-derived builders in `cache/cache-keys.ts`
  (`LendingMarketProfileDoc`, `LendingAccountProfileDoc`, `LendingTokenEModeProfileDoc`, `LendingMarketIndexes`,
  `LendingBulkOraclePrice`, `LendingTokenPrice*`, `LendingTopMarketParticipants`, `LendingMarketParticipantsCount`,
  `LendingMarketStatsGraphData`, `LendingMarketAverageGraphData`, `UserLendingPositions`, …). **Depends:** B3.
- **B6 — Publish:** bump to ≥1.0.425, build dual ESM/CJS (watch the CJS-types dual-package gotcha), publish.
  **Depends:** B1-B5 green.  **Executor:** cheap model for B1/B2/B4/B5, standard for B3; human publish.

---

## 5. Repo C — `@xoxno/sdk-js` (currently 1.0.154, dep types ^1.0.424).

Bump dep to published vTYPES first. Verify: `pnpm typecheck && pnpm lint && pnpm test`.

- **C1 — Decoders:** in `src/sdk/stellar/events/decode.ts` read the new `market_id` slot per topic:
  `decodePositionDelta`(:132-146, deposit/borrow deltas), `decodeMarketSnapshot`(:161), `decodeMarketParams`(:123),
  and registry sites (:226/243/271/296/305/310/326/341/363/394). Output objects must match B1 shapes.
  **Depends:** B6, A6 frozen.
- **C2 — Builders:** `StellarTokenAmount` gains `marketId`; `tupleAddrAmountVec` re-encodes to the new
  contract leg (`Vec<(u32, i128)>` or `(u32, Address, i128)` — match A5 ABI); `buildStellarSupplyBatchTx`(:195)/
  borrow/withdraw/repay/liquidate/flash/multiply/swap* take `marketId`. **Depends:** A5 ABI frozen, B2.
- **C3 — Release:** `buildStellarLendingIdentifier`(id.ts:32) unchanged (account-only). Bump ≥1.0.155, publish.
  **Depends:** C1,C2 green.  **Executor:** standard model (ABI-encoding correctness); human publish.

---

## 6. Repo D — `xoxno-az-functions` (indexer). Depends on vTYPES+vSDK.

Verify: `pnpm typecheck && pnpm lint && pnpm test`. Then reindex on testnet.

- **D1 — Rename footgun:** rename the Cosmos id-formatter `marketId(contractId)`
  (`stellar-lending-id.util.ts:81-89`) → `marketProfileDocId`, before anything else. **Depends:** —
- **D2 — Doc ids:** `StellarLendingIdUtil.accountId`(:163-181) → `${identifier}_${marketId}`; market doc id → marketId.
  Must byte-match the B3 types constructor. **Depends:** B6, D1.
- **D3 — Decoder mapping:** read `update.marketId` in `stellar-position-update.event.ts:94-96,143,179-181`;
  set `marketId` field + id from it; keep `token`. **Depends:** C3 (sdk decodes market_id), B6.
- **D4 — Processor read-key:** `processPositionPatch` read-before-write `(accountId, assetContractId)`
  (`stellar-lending-event.processor.ts:343-346`) → `(accountId, marketId)`; merge/delete logic by MarketId. **Depends:** D2,D3.
- **D5 — Activity + multiply:** `buildMarketProfileMap`/`buildDecimalMap`/APY
  (`stellar-lending-activity.builder.ts:230-279`) key by MarketId; `processInitialMultiplyPayment`(:728-762)
  patch by MarketId not `existing.token`. **Depends:** D3.
- **D6 — Aggregations + Kusto columns:** any group/sum by token → `(token, marketId)`; ensure the Kusto tables
  `LendingMarketActivity`/`LendingPositionActivity`/`LendingRevenueSnapshot` EMIT a `MarketId` column
  (api-v2 KQL E4 reads it). **Depends:** D3.
- **D7 — Reindex:** reset `StellarLendingCursorDoc` to A10 redeploy ledger; purge/migrate legacy
  `${identifier}_${token}` docs; replay. **Depends:** D2-D6, A10.  **Executor:** standard model; human-run reindex.

---

## 7. Repo E — `xoxno-api-v2` (reads). Depends on vTYPES+vSDK. Uses raw `@stellar/stellar-sdk` for chain reads (hand-written args).

Verify: `pnpm typecheck && pnpm lint && pnpm test`.

- **E1 — Reconciler:** `stellar-reconciler.service.ts` side-map "keyed by asset" (:458-492,:539) → keyed by MarketId;
  resolve `marketId → underlying` (new A5 view) to keep `token`/decimals on docs; doc id (:649-650) → `${identifier}_${marketId}`;
  market doc id (:1064) + emode (:957) include marketId; `invalidateCaches`(:1192-1220) rebuild MarketId-aware keys. **Depends:** B6,C3,A5.
- **E2 — Service identity + joins:** `lending-data.service.ts` id reconstruction (:392,:3716,:3746) → marketId;
  position→market join `allMarketProfiles[doc.token]`(:2433, fill :2386) → by MarketId; point reads
  `getLendingAccountProfileDoc`(:3741)/patch(:3709); `WHERE token=@token` filters (:4024,:4072) → marketId.
  Per-underlying USD/EGLD rollups (:4280-4333) already sum across positions — keep, but group line items by MarketId. **Depends:** E1.
- **E3 — Soroban reads:** `stellar-oracle.service.ts:212-246` `get_all_market_indexes_detailed(assetVector)` →
  pass `Vec<MarketId>` / return rows carrying marketId; positional `result[token]` → re-key by marketId;
  `getAllStellarMarketsTokens`(:467)/`getCachedStellarMarketAddresses`(:3677) enumerate MarketIds. **Depends:** A5,C3.
- **E4 — Kusto:** `summarize ... by Token` / `join on Token` (service :611,:641-644,:654-657,:825,:834-835)
  → `by Token, MarketId`. **Depends:** D6 (columns emitted).
- **E5 — Cache keys + flush:** consume B5 MarketId-aware keys; hand-rolled `'stellar:lending:market-addresses'`(:3679);
  full lending cache flush after E1-E4 + reindex. **Depends:** B6, D7.
- **E6 — Endpoints + DTOs (G10 breaking):** `lending-data.controller.ts` `:token` params and `token` filters
  accept/disambiguate marketId; `/lending/market/indexes`(:196) & `/lending/market/prices`(:534) `Record<token,…>`
  responses re-key by marketId (breaking) or nest; add `marketId` to per-market/per-position response DTOs. **Depends:** E1-E3; **G10 owner+UI**.
- **E7 — Release:** bump sdk dep to vSDK, deploy. **Depends:** E1-E6 green.  **Executor:** standard model; human deploy.

---

## 8. Phase 5 — downstream (out of primary scope, flagged)

`xoxno-ui` consumes vTYPES + vSDK + the api-v2 responses. The breaking
`Record<token,…>` → `Record<marketId,…>` change (G10) and the new `marketId`
fields require UI work: market lists, position rows, the per-underlying rollup
view (group by token, show per-MarketId breakdown). Add a UI lane only after E6
shape is final. Not planned in detail here.

---

## 9. Execution method (agent orchestration)

- **Driver:** subagent-driven-development, this file as the plan. One implementer
  subagent per task; task-review (spec + quality) after each; broad review at
  end of each repo phase.
- **Isolation:** Phase 1 (contracts) in a git worktree `feat/market-id`
  (parallel-unsafe). Phases 2-7 are separate repos → can run B fully, then C,
  then D and E in parallel (independent repos, both gated on C publish).
- **Models:** cheap for mechanical re-keys/event-ctor/test edits (A6, A9-tests,
  B1/B2/B4/B5, D1); standard for logic (A1-A5, A8, C, D3-D7, E); most-capable
  reviewer for A7 (policies) and each end-of-phase broad review.
- **Per-task contract:** implementer reads only its task block here + the named
  files, runs the Verify command, reports test output. No task marked done
  without its Verify evidence.
- **Gates:** (1) G6+G7 owner sign-off before A7. (2) Contract event/ABI/view
  shapes frozen before B. (3) vTYPES published before C. (4) vSDK published
  before D/E. (5) G10 + UI coordination before E6. (6) A10 redeploy ledger
  before D7 reindex.

---

## 10. Risks / rollback

- **Data collision (highest):** until B3+D2 land, two markets of one underlying
  overwrite one Cosmos doc. Do not list a second market for any underlying on a
  live indexer before the indexer is MarketId-aware.
- **Reindex window:** D7 requires downtime/replay; schedule with E5 cache flush.
- **Breaking API (G10):** `Record<token,…>` consumers break; gate on UI.
- **Migration correctness (A8):** lazy re-key must preserve scaled balances and
  risk-param snapshots; covered by regression test.
- **Rollback:** Phase 1 is a worktree branch — abandon before merge if blocked.
  Types/sdk are additive (optional `marketId`) until the id-constructor branch
  (B3) flips; keep MVX path untouched so MVX never regresses.
- **Certora:** spec re-key (A9) is large; use `check_orphans.py` green gate, not
  raw cargo, for the controller certora build (known expected-broken under
  in-flight type refactors).

---

## 11. Open decisions to bring to the owner before starting

1. **G6** — oracle consistency policy for same-underlying markets (Option A vs B).
2. **G7** — same-underlying collateral↔borrow loop (forbid vs spread-cap).
3. **G10** — accept breaking `Record<token>`→`Record<marketId>` API responses, or nest.
4. Confirm we actually want N markets per underlying now, or only the *capability*
   (build the indirection, list 1:1 initially) — affects whether D7 reindex is urgent.
