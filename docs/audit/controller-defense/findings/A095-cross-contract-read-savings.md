# A095 — Cross-contract read savings vs correctness tradeoffs

- Agent: A095
- Theme: T6 / T7
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/context/mod.rs:1-84` (`Cache`; `load_markets`; `require_hub_active`)
  - `contracts/controller/src/context/oracle.rs:19-38` (`fetch_prices`, `cached_price`)
  - `contracts/controller/src/context/market_index.rs:12-42` (`put_market_index`, `fetch_market_indexes`, `cached_market_index`)
  - `contracts/controller/src/context/pool.rs:8-28` (`cached_pool_address`, `cached_pool_sync_data`)
  - `contracts/controller/src/external/price_aggregator.rs:20-45` (hard `prices` vs soft `quotes`)
  - `contracts/controller/src/external/pool.rs:147-162` (`get_sync_data`, `get_bulk_indexes`)
  - `contracts/controller/src/risk/totals.rs:69-178` (`load_markets` at risk totals)
  - `contracts/controller/src/risk/validation.rs:31-60` (`require_post_pool_risk_gates`)
  - `contracts/controller/src/strategies/mod.rs:57-78` (`prefetch_strategy_prices`, `strategy_finalize`)
  - `contracts/controller/src/strategies/flash_position.rs:80-158` (sync gate → prefetch → callback → finalize)
  - `contracts/controller/src/positions/{mod,supply,debt}.rs` (`merge_*_leg` → `put_market_index`)
  - `contracts/pool/src/lib.rs:299-316` (simulate-only bulk indexes)
  - Deliberate non-saves: `storage/account.rs` / `external/position_nft.rs` (`owner_of` live); `payments.rs` / strategy balance snapshots (token FFI live)
- Defense: Every saved **cross-contract** read is paired with an explicit correctness policy: (1) hard oracle prices are a one-shot fail-closed snapshot (ADR-0005 / INV-ORACLE-01..03); (2) pool market indexes are simulate-batched then **overwritten** from mutation returns via `put_market_index` (A077/A094); (3) `get_sync_data` is fill-once and only consumed **before** pool legs on the sole money-path reader (`flash_position`); (4) authority and custody reads that must observe mid-tx / mid-ledger truth (`owner_of`, token balances, pool mutation results, swap/Blend FFI) are **never** memoized in `Cache`. Savings therefore do not substitute for risk gates, measured receipts, or live ownership.
- Gap: (1) Index savings are safe only while every pool-merge helper keeps calling `put_market_index` — engineering footgun owned by **A094**. (2) Sync-data savings have **no** invalidation API — incomplete rule owned by **A086/A088**; low practical risk on today’s call graph. (3) Strategies prefetch prices not indexes (budget asymmetry, **A087**). (4) Keepers can N+1 lazy `cached_market_index` before bulk `load_markets` (**A087**). (5) Aggregator address is re-read from storage on every `fetch_prices` batch (tiny storage cost, not a correctness hole). (6) Bulk zip uses `get_unchecked` by request order — trusts pool length contract (honest pool under SAC).
- Impact: **No fund-theft, undercollateralized exit, or skipped solvency/oracle failure** from preferring memoized cross-contract reads under current production graphs. Blast radius of a mistaken save is account/tx-local mis-valuation or wrong accept/reject inside one invocation; durable SoT remains pool storage + account shares + live NFT ownership. Leading residual is still A094 (forgotten `put`), not “too aggressive caching” of prices. Practical impact of current tradeoffs ≈ **negligible** for safety; positive for Soroban CPU/read budget on multi-asset HF paths.
- Evidence: ADR-0005, ADR-0020; INV-ORACLE-01..03, INV-RISK-01, INV-IDX-*; threat-model “never caches `owner_of`”; Certora `price_cache_consistency`, `index_cache_single_snapshot`; unit `tests/context/oracle.rs`; peers A086, A087, A088, A094, A099, A104 §7 hole closure; SEED Cache facts.
- Opinion: **Saved cross-contract reads are a correctness-preserving budget layer, not a silent trust shortcut.** Keep the three-way split: freeze prices; mutate-overwrite indexes; leave authority/custody live. Do not “fix” oracle mid-tx after legs. Document the sync fill-once rule next to `put_market_index`. Optional budget-only work (prefetch indexes in keepers/strategies) must not become a correctness requirement.

---

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, README finding format, `AGENT_MANIFEST` Wave 6 (**A095**), peers **A086, A087, A088, A094, A099, A104** (A104 explicitly owed this file a “systematic saved-read catalogue with invariant mapping”).
2. Enumerated every `Cache` memo that avoids a **cross-contract** FFI vs those that only avoid **controller storage** reads (boundary §1).
3. For each saved FFI: source contract, miss/hit behavior, invalidation, what correctness property the save encodes or risks, which INV/ADR owns it.
4. Catalogued deliberate **non-saves** (live FFI) that the threat model and money paths require.
5. Built a savings×correctness tradeoff matrix and per-entrypoint profile.
6. No production Rust edited. No git operations (COORDINATION).

No novel Critical/High. Agrees with A087/A094 tradeoff narrative and A104’s “substantially defended” Wave-6 posture.

---

## 1. Scope boundary: what counts as a cross-contract read save

`Cache` mixes three concern classes. **A095 owns only class A** for the tradeoff verdict; B/C are cited so savings claims are not inflated.

| Class | Examples | Cross-contract? | Primary owners |
|---|---|---|---|
| **A. Saved remote reads** | Aggregator `prices`; pool `get_bulk_indexes`; pool `get_sync_data` | **Yes** | This file; deep dives A087 / A088 / A094 |
| **B. Saved local storage reads** | `pool_address` (`ControllerKey::Pool`); spoke config/assets; `verified_hubs`; spoke usage rows | No (instance/persistent storage) | A086, A088, A089*, A090*, A091* |
| **C. Write buffers** | `supply_updates` / `debt_updates`; spoke usage dirty rows until `persist` | N/A | A033, A078, A092* |

`cached_pool_address` is **not** a cross-contract save: first fill is `storage::get_pool`. It *enables* cheaper subsequent pool FFI by avoiding repeated storage lookups when constructing `LiquidityPoolClient`. Correctness of the address memo is A088 (immutable after `deploy_pool`).

---

## 2. Catalogue of saved cross-contract reads

### 2.1 Price aggregator — hard `prices` (primary valuation save)

| Property | Behavior |
|---|---|
| Miss path | `fetch_prices` → `collect_uncached_keys` → one `PriceAggregatorClient::prices` → insert into `token_prices` |
| Hit path | `cached_price` returns memo; **no** lazy re-fetch |
| Failure | Missing key → panic `OracleNotConfigured` **before** Cache is updated from that call’s return map |
| Invalidate / refresh | **None** by design for the Cache lifetime |
| Batching | Single FFI for the uncached subset; strategies warm early via `prefetch_strategy_prices` |

**Savings:** Multi-asset risk walks (HF, liquidation math, post-pool gates) would otherwise N× aggregator calls. Liquidation plan warms once; math and post-totals reuse. Strategy finalize’s `load_markets` → `fetch_prices` is a no-op for already-prefetched keys.

**Correctness tradeoff:** Mid-tx oracle updates (including during `flash_position` callback) are **ignored**. That is not accidental staleness — it is INV-ORACLE-03 / ADR-0005 (“one coherent valuation snapshot”) and ADR-0020 (“Prices used at finalize are the pre-callback snapshot”).

| Invariant / ADR | How the save supports it |
|---|---|
| INV-ORACLE-01 | Hard path only; soft `quotes`/`PriceStatus` never feed money gates |
| INV-ORACLE-02 | Enforced inside aggregator before hard `prices` returns |
| INV-ORACLE-03 | Memo forbids split snapshots across legs/gates in one mutation |
| ADR-0005 | Fail-closed complete set before risk decisions |
| ADR-0020 | Pre-callback freeze for flash_position solvency |
| INV-RISK-01 | Gates still run; they consume the snapshot, not a bypass |

**Counterfactual (re-fetch every `cached_price`):** Better “live” oracle freshness mid-tx, worse MEV/oracle-race surface inside one user tx, breaks INV-ORACLE-03 proofs (`price_cache_consistency`), and increases budget. **Rejected by design.**

**Asymmetry with indexes:** `cached_price` never lazy-fetches (panic if cold). Forces callers to warm deliberately — fail-closed engineering constraint (A087).

---

### 2.2 Pool — `get_bulk_indexes` (simulate projection save)

| Property | Behavior |
|---|---|
| Miss path (bulk) | `fetch_market_indexes` → one `get_bulk_indexes` for uncached hubs → zip by order into `market_indexes` |
| Miss path (lazy) | `cached_market_index` → single-element bulk fetch |
| Hit path | Return memoized `MarketIndexRaw` |
| Post-mutation | `put_market_index` **overwrites** from pool mutation DTO |
| Pool semantics | `get_bulk_indexes` is **simulate-only** (no pool storage write) — `pool/src/lib.rs:303-316` |

**Savings:** Portfolio HF / LTV / liquidation planning needs one index per open hub. Without Cache: one bulk (or N singles) per walk, and again after each gate. With Cache: one simulate batch for untouched hubs; touched hubs pay the mutation FFI anyway and refresh via `put` without a second simulate.

**Correctness tradeoff:** Simulated indexes can diverge from post-accrual mutation indexes **and** from concurrent ledgers’ accruals. Within one invocation the design requires:

1. Pre-leg / untouched hubs → simulate is acceptable (same `now_ms` within the tx).
2. Touched hubs → mutation return is truth → **must** `put_market_index` (A094).
3. Cap/usage on entry use mutation DTO indexes directly (A077/A081), not a stale Cache hit.

| Invariant / ADR | How the save supports it |
|---|---|
| INV-IDX-01..05 | Durable monotone/accrual truth stays on **pool** storage; Cache is ephemeral projection (A038) |
| INV-ORACLE-03 (index half) | Certora `index_cache_single_snapshot` — two lazy reads agree without intervening `put` |
| INV-RISK-01 / INV-RISK-02 | Post-leg gates see mutation-fresh indexes for touched hubs + coherent simulate for remainder |
| ADR-0003 | Share↔amount math uses the index the protocol chose for that step |

**Counterfactual (re-simulate after every pool leg):** Would paper over a forgotten `put` but double pool reads, still miss another tx’s concurrent accrual between simulate calls, and could disagree with the mutation DTO the leg just applied. **Current design prefers mutation overwrite over re-simulate.**

**Certora note:** `spec_hooks` no-ops `fetch_market_indexes`; harnesses seed via `put_market_index`. Snapshot rules prove memo stability, not pool isomorphism (A035 epistemology).

---

### 2.3 Pool — `get_sync_data` (params/state blob save)

| Property | Behavior |
|---|---|
| Miss path | `cached_pool_sync_data` → `LiquidityPoolClient::get_sync_data` → fill map |
| Hit path | Return blob; **no** re-fetch |
| Invalidate | **None** (no `put_pool_sync_data` / clear-on-leg) |
| Money-path consumer | Sole mutator: `flash_position` pre-leg `is_flashloanable` |
| Other consumers | Views’ decimals unscale (`new_view`); admin listing uses **direct** `fetch_pool_sync_data` (bypass) |

**Savings:** Avoids a second sync FFI if the same hub’s sync were read twice. Today’s money path reads once — savings are mostly structural (shared helper + future reuse) plus view paths that also touch indexes.

**Correctness tradeoff:** After any accruing mutation or `update_params`, a Cache hit would be stale. Production money paths **do not** re-read sync after legs (A088 exhaustive timing). Views use a fresh `new_view` Cache per call.

| Invariant / control | Mapping |
|---|---|
| Flashloanable policy (ADR-0020) | One-shot pre-leg check via memoized sync; cash `flash_loan` re-checks **live** on pool |
| Interest / decimals domain | Admin caps use live fetch (A073); risk valuation uses indexes+prices, not sync state |
| A086 residual | Incomplete invalidation documented; do not add post-leg sync safety reads without clear/put |

**Counterfactual (always live `get_sync_data`):** Negligible safety gain on current graph; small budget cost. Worth it only if a post-leg sync gate is introduced.

---

## 3. Catalogue of deliberate non-saves (correctness requires live FFI)

These are cross-contract (or token) reads the controller **must not** fold into `Cache` memos. Saving them would be the wrong tradeoff.

| Live read | Why not memoized | Invariant / threat-model anchor |
|---|---|---|
| NFT `owner_of` / `try_owner_of` | Ownership can change; auth must see current holder | Threat model: “reads it live on every account access and never caches it”; INV-STOR-03 / INV-AUTH-02 |
| Token `balance` before/after | Measured custody; fee-on-transfer / hooks | INV-ACCT-02..04; ADR-0011 balance-delta settlement |
| Pool mutation returns (supply/borrow/withdraw/repay/seize/net_settle/strategy) | SoT for shares, amounts, post-accrual indexes | INV-ACCT-*; A041–A045 measure paths |
| Pool `flash_loan` prepare path | Live `is_flashloanable` + exact repay | ADR-0010 / A044 |
| Swap aggregator quotes/execution | Untrusted router; settle on balance deltas | ADR-0011; A056 |
| Blend `submit` | External pool; guarded measured path | A050 / A071 |
| Soft aggregator `quotes` / `PriceStatus` | Observability only; must not replace hard `prices` | INV-ORACLE-01; views detailed path (A087) |

**Contrast lesson:** Cache saves are allowed when the protocol **defines** a snapshot or when the remote value is immutable for the invocation under SAC. Cache saves are forbidden when the remote value is an **authority or custody oracle** that attackers or users can change.

---

## 4. Tradeoff matrix (savings × correctness)

| Saved FFI | Budget win | Correctness stance | Failure mode if policy broken | Severity today |
|---|---|---|---|---|
| Hard `prices` memo | High on multi-asset HF / liq / strategies | **Required** snapshot (ADR-0005/0020) | Mid-tx price chase / split snapshot | Defended |
| Bulk/lazy indexes | High on portfolio walks | Safe iff `put` after mutations | Stale simulate on touched hub → wrong HF/caps | Partial via **A094** footgun |
| Sync data memo | Low–medium | Safe iff no post-mutation safety read | Wrong flag/decimals decision in-tx | Info residual **A086/A088** |
| (Non-save) `owner_of` | Would save NFT reads | **Must stay live** | Auth on stale owner | Correctly not saved |
| (Non-save) balances | Would save token reads | **Must stay live** | Share mint ≠ cash | Correctly not saved |

**Net judgment:** The design spends budget on **repeat valuation reads** and refuses to spend correctness on **authority/custody repeats**. That is the right axis for a lending controller on Soroban.

---

## 5. Per-entrypoint savings profile

| Entrypoint family | Cross-contract reads saved | Relies on | Risk if save wrong |
|---|---|---|---|
| Ordinary borrow / risky withdraw / risky supply | Post-pool `load_markets` prices+indexes; mutation `put` | A087/A094 | Wrong HF if `put` omitted |
| Debt-free supply / full repay | Often skips valuation entirely | A099 intentional | N/A (no HF) |
| Strategies (multiply, swaps, repay-with-collateral, migrate, flash_position) | Early price snapshot; finalize no-op re-fetch; mutation puts | ADR-0005/0020 | Stale prices are **policy**; missing prefetch panics |
| `flash_position` specifically | Sync once pre-leg + prices pre-callback | A088 timing | Sync after leg unused today |
| `flash_loan` (cash) | Pool address storage memo only; **no** oracle | Pool live checks | N/A |
| Liquidation | One portfolio `load_markets`; math reuses; apply puts via merges | A087 §2.3 | Plan vs post-totals share price snapshot |
| Keepers threshold sync | Lazy index N+1 then bulk prices at FullTuple | A087 budget note | Correct; wasteful |
| Views | Fresh `new_view`; soft quotes for detailed; hard path for risk views | A008 | Observability skew only |

---

## 6. Interaction with security checks (savings must not skip gates)

A099 hunts optimizations that skip checks. Mapping those hunts onto **cross-contract** saves:

| Optimization | Skips a remote read? | Skips a security check? | Verdict |
|---|---|---|---|
| Price memo hit | Yes (aggregator) | No — gate still runs on snapshot | Defended |
| Index memo hit | Yes (pool simulate) | No — unless forgotten `put` poisons inputs | A094 residual |
| Sync memo hit | Yes (`get_sync_data`) | Only if a second safety read were intended | Not present today |
| `verified_hubs` | **No** (storage) | Skips repeat hub-active after **success only** | A099 defended |
| Debt-free solvency skip | Skips `load_markets` entirely | Intentional — no borrow risk | A099 / INV-RISK-01 scope |
| Exit usage no-op | Storage/usage | Capacity distortion | **A080** (T5), not a remote-read save |

**Conclusion:** Cross-contract read savings do not implement “skip the oracle if inconvenient” or “skip solvency if Cache says so.” Incomplete prefetch **fails closed**. Debt-free skip is policy, not memo corruption.

---

## 7. Budget mechanics that preserve correctness

### 7.1 Dedup before FFI

`collect_uncached_keys` / `unique_hub_tokens` ensure one remote key per distinct Address or HubAssetKey. O(n²) scan is documented safe only under position-limit bounds — attacker-growable unbounded vecs must not call it (common module docs). Boundedness is a **DoS** property of the save layer, not a valuation bias.

### 7.2 Atomic fill vs panic

`fetch_prices` builds a local map and panics on missing aggregator entries **before** assigning into `token_prices`. A failed hard batch does not leave a partial success set from that call for later `cached_price` consumption on a rolled-back path (Soroban tx abort). Prior successful prefetches in the same Cache would only matter if code caught panics and continued — it does not on these paths.

### 7.3 Price vs hub keying

Prices keyed by underlying `Address`; indexes by `HubAssetKey`. One oracle snapshot can value the same token on two hubs while indexes remain market-local — correct savings, not conflation (A087 §3.3).

### 7.4 What is *not* memoized (small leftover costs)

- `storage::get_price_aggregator` on every `fetch_prices` / `fetch_prices_status` (local storage).
- Building `LiquidityPoolClient` after address hit (address memo helps; client construction is cheap).
- Token and NFT live reads (intentional).

---

## 8. Residuals (owned elsewhere; restated for the tradeoff narrative)

| Residual | Tradeoff angle | Owner |
|---|---|---|
| Forgotten `put_market_index` | Saved simulate read becomes wrong post-leg truth | **A094** (low, partial) |
| No sync invalidation | Saved sync read unsafe if future post-leg consumer appears | **A086 / A088** (info) |
| Strategy price-only prefetch | Saves oracle early; indexes paid later at finalize / mutation | **A087** (info / budget) |
| Keeper lazy index N+1 | Prefers correctness-on-demand over bulk save | **A087** |
| A080 exit no-op | Not a cross-contract save; capacity skip | **A080** via A099 |
| Certora fetch no-op | Verification ≠ WASM savings model | **A035** |

None of these elevate “read savings vs correctness” to undefended fund loss under SAC + honest listing.

---

## 9. Peer agreements / disagreements

### Agreements

- **A087:** Batching is defended; frozen prices are policy; index overwrite mandatory.
- **A094:** Skipping re-fetch after mutation is safe **only** with `put_market_index`.
- **A088:** Sync memo defended on current timing; incomplete invalidation is real but low blast radius.
- **A086:** Inventory matches; sync residual unchanged.
- **A099:** Memo short-circuits are not the leading “skipped check”; A080 is.
- **A104:** This file closes the A095 coverage hole with the requested catalogue; does not overturn “substantially defended.”
- **A038:** Durable index SoT is pool; Cache savings cannot fork persistence.

### Disagreements

None. No disagreement file warranted.

---

## 10. Remediation / hygiene (non-blocking)

1. **Checklist (with A094):** every new pool mutation merge → `put_market_index` (+ usage apply). This protects the index **save**.
2. **Docs (with A086/A088):** state fill-once sync contract beside `put_market_index` overwrite contract — when savings are allowed vs forbidden.
3. **Do not** add mid-tx `fetch_prices` refresh after strategy legs or flash callbacks.
4. **Optional budget:** prefetch indexes in keepers / strategies; must remain additive, not a new correctness assumption.
5. **Optional:** `remove` sync map entry if a future composite flow re-reads sync after `update_params` on the same Cache (no such flow today).
6. **Tests:** keep `tests/context/oracle.rs` first-pass; consider analogous bulk-index unit (A087 gap).

---

## 11. Executive verdict

**Cross-contract read savings in controller `Cache` trade repeated remote reads for an explicit snapshot/overwrite discipline that matches the protocol’s valuation invariants.**

- Saving **oracle** reads → correctness **feature** (one snapshot).
- Saving **simulate index** reads → correctness **conditional** on mutation overwrite.
- Saving **sync** reads → correctness **conditional** on pre-mutation-only consumption.
- Saving **ownership / balances / mutation results** → would be a correctness **bug**; the code does not.

Status **defended** at severity **info**. Re-open only if a new production path: (a) reuses `cached_price` across intentional mid-tx oracle refresh needs, (b) reads `cached_market_index` after a pool leg without `put`, or (c) gates safety on `cached_pool_sync_data` after mutating that hub.
