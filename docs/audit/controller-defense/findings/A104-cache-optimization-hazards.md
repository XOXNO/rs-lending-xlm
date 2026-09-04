# A104 — Cache / optimization hazards (synthesis of A086–A100)

- Agent: A104 (synthesis)
- Theme: T6 / T7 / T8
- Severity: low (highest **in-wave** residual is A094 engineering footgun; inherited A080 capacity distortion is medium but owned by T5)
- Status: partial (Wave-6 now **15/15** filed; corpus mostly **defended**; residuals clustered and quantified below; original 4/15 snapshot kept in §0.2)
- Paths: synthesis over `findings/A086*`, `A087*`, `A094*`, `A099*`; `context/{mod,pool,market_index,oracle,spoke,events}.rs`; adjacency from A008, A032, A033, A034, A063, A072, A077, A078, A080, A084
- Defense: See §3 (defended Cache / optimization surfaces)
- Gap: See §4 (hazards with quantified blast radius) and §7 (coverage holes for unfiled A088–A093, A095–A098, A100)
- Impact: See §4–§5. No filed Wave-6 finding demonstrates silent fund theft, undercollateralized exit of a gated path, or bypass of a failed hub/oracle/solvency check via memoization. Leading live risk inside the wave is **wrong mid-tx index/price use if a future pool merge omits `put_market_index`** (A094). Leading related residual called out by A099 is **A080 spoke-usage exit no-op** (capacity distortion, not theft).
- Evidence: Peer findings A086, A087, A094, A099; PRELIMINARY “Cache memoization… defended” + residual row A094; SEED Cache facts; ADR-0005 / ADR-0020; INV-ORACLE / INV-RISK-01; adjacency A032/A033/A034/A063/A077/A078/A080
- Opinion: Treat Wave-6 Cache design as **substantially defended** for correctness under current call sites. Prioritize a review checklist / lint for `put_market_index` after every pool mutation merge (A094), document `pool_sync_data` invalidation rules (A086), and keep success-only `verified_hubs` + debt-free solvency skip as explicit policy (A099). Do not “fix” ADR-0005 price snapshots mid-tx. Re-run A104 when A088–A093 / A095–A098 / A100 land.

> **Corpus-complete addendum:** A088–A093, A095–A098, A100 later filed. All
> **defended** / info except A094 (**partial**, low footgun) and A100
> (optimization-note). No new Critical/High. In-tx index overlay confirmed
> sound (A098). Ranking: `synthesis/FINAL.md`.

---

## 0. Method and corpus coverage

### 0.1 Method

1. Read `shared/COORDINATION.md`, `SEED.md`, `synthesis/PRELIMINARY.md`, `shared/AGENT_MANIFEST.md` Wave 6, README finding format.
2. Read every finding file under `findings/` whose id is in **A086–A100** and that exists on disk.
3. Extract: status, severity, gaps, impact claims, peer cross-links.
4. Cluster residuals into hazard classes; quantify blast radius; cross-link IDs.
5. Prefer **agreement** unless peer evidence conflicts (none found — see §8).
6. For missing scopes, record **adjacency only** from peers (A008/A032/A033/A034/A063/A072/A077/A078) — do not invent Wave-6 findings.

No production Rust edited. No git operations (COORDINATION).

### 0.2 Coverage map (A086–A100)

| ID | Manifest scope | File present? | Peer status | Severity | Role in this synthesis |
|---|---|---|---|---|---|
| A086 | Cache field inventory / invalidation | **yes** | defended | info | Field map; `pool_sync_data` non-invalidation residual |
| A087 | Prefetch prices / market indexes batching | **yes** | defended | info | Warm-up matrix; fail-closed prices; budget residuals |
| A088 | `pool_address` / `pool_sync_data` memoization | **no** | — | — | Hole; A086 owns sync-data gap; `context/pool.rs` confirms memo |
| A089 | `spoke_config` / `spoke_assets` memoization | **no** | — | — | Hole; A086 pin/`reset_spoke_context`; A063 listing stack |
| A090 | `verified_hubs` memoization correctness | **no** | — | — | Hole; covered inside A099 + A063 |
| A091 | spoke_usage embedded in Cache lifecycle | **no** | — | — | Hole; A078 persist timing + A086 pin; A080 residual |
| A092 | Event update buffers coalesce | **no** | — | — | Hole; A033 event-after-persist defended |
| A093 | `new` vs `new_view` TTL side effects | **no** | — | — | Hole; A034 + A008 + A086 constructor fact |
| A094 | Avoided re-reads after pool — staleness | **yes** | **partial** | low | Primary Wave-6 correctness footgun |
| A095 | Cross-contract read savings vs correctness | **no** | — | — | Hole; A087/A094 tradeoff narrative |
| A096 | Account load shapes (borrow/supply/full) | **no** | — | — | Hole; no peer deep-dive filed |
| A097 | Write batching on `finalize_position_flow` | **no** | — | — | Hole; A032 defended (gates retained) |
| A098 | Market index cache vs live accrual races | **no** | — | — | Hole; A094 cross-ledger concurrency note |
| A099 | Optimizations that skip a security check | **yes** | defended | low | Memo short-circuits hunt; points at A080 |
| A100 | Dead cache paths / unused memo maps | **no** | — | — | Hole; A086 inventory implies all maps live |

**Present:** 4 / 15 scopes (**A086, A087, A094, A099**). **Missing:** 11 scopes. Synthesis is authoritative for the four filed agents; unfiled IDs contribute only adjacency (§7).

PRELIMINARY already listed A086/A094/A099 under defenses and called out A094 as a leading residual; A087 has since filed and **agrees** (defended batching; points back at A094).

---

## 1. Executive verdict

**In-memory `Cache` and related storage/read optimizations are a coherent correctness design, not a silent-bypass layer.**

Under ADR-0005 (one coherent valuation snapshot per tx) and ADR-0020 (flash_position finalize uses pre-callback prices):

| Layer | Verdict | Owning IDs |
|---|---|---|
| Price memo (`token_prices`) | **Defended** — hard batch; `cached_price` fail-closed; no mid-tx refresh by design | A087, A086 |
| Market-index memo | **Defended today** — mutation overwrite via `put_market_index`; bulk simulate for untouched hubs | A087, A094, A077 |
| Hub-active memo (`verified_hubs`) | **Defended** — success-only; failed check never sticky | A099, A063 |
| Spoke pin / config / assets | **Defended** (adjacency) — single-spoke pin; `reset_spoke_context` clears | A086, A089*, A063 |
| Finalize write batching | **Defended** (adjacency) — solvency before persist; events after storage | A032*, A097*, A033* |
| View vs mutator TTL | **Defended** (adjacency) — `new_view` skips instance renew (rent-grief defense) | A093*, A034, A008, A086 |
| Debt-free solvency skip | **Defended policy** — intentional; not a forgotten gate | A099, A072 |
| Credit fee-only usage | **Defended policy** — intentional under-count vs gross seize | A099, A084 |

**Residuals that survive** collapse into:

1. **Engineering footgun** — omit `put_market_index` on a new pool merge → stale simulated index for HF/caps in the same tx (**A094**).
2. **Incomplete invalidation** — `pool_sync_data` not cleared after pool mutations; low risk with today’s call sites (**A086**; deep-dive owed to **A088**).
3. **Budget / hygiene** — keeper N+1 lazy indexes; strategies prefetch prices not indexes; missing bulk-index unit test (**A087**).
4. **Inherited T5** — A099’s hunt redirects the only “skipped check” with material capacity impact to **A080** (exit no-op on missing usage row).

No Wave-6 filing elevates a Cache memo to Critical/High fund-loss under SAC + honest listing assumptions.

---

## 2. What labels mean here

| Label | Meaning |
|---|---|
| **Defended** | Memo / skip / batch is paired with a fail-closed or overwrite rule that preserves the intended invariant under stated assumptions. |
| **Partial** | Core design holds, but a documented footgun, incomplete invalidation, or inherited residual remains. |
| **Undefended** | An optimization removes a security check with no compensating control. **None claimed as novel in the filed Wave-6 corpus.** Closest live “skip” with impact is A080 (T5), not a Cache map bug. |
| **Optimization-note** | Budget / test / docs only — no correctness hole. |
| **Coverage hole** | Manifest agent file missing; adjacency may exist elsewhere. |

---

## 3. Defended Cache / optimization surfaces

These surfaces are consistently judged **defended** across filed peers (and strong adjacency). Synthesis agrees. Do not reopen without new evidence.

### 3.1 Field inventory and lifecycle (A086)

`Cache` (per invocation) memoizes:

| Field | Key | Invalidation / refresh today |
|---|---|---|
| `token_prices` | `Address` | First hard `fetch_prices`; no mid-tx refresh (ADR-0005) |
| `market_indexes` | `HubAssetKey` | Bulk/lazy simulate; **overwrite** via `put_market_index` after pool legs |
| `pool_address` | singleton | Immutable for tx (storage read once) |
| `pool_sync_data` | `HubAssetKey` | Fill-once; **not** cleared after mutations (residual §4.2) |
| `spoke_usage` / `spoke_config` / `spoke_assets` | spoke-scoped | Pin via `ensure_spoke_context`; clear via `reset_spoke_context` |
| `verified_hubs` | `u32` | Success-only insert (A099) |
| `supply_updates` / `debt_updates` | buffers | Drained at emit; not SoT (A033 adjacency) |

`Cache::new` renews instance TTL then `new_view`; `new_view` builds empty maps without renew — rent-grief defense for views (A008/A034).

### 3.2 Price / index batching (A087)

| Property | Control |
|---|---|
| Hard oracle batch | `fetch_prices` → aggregator `prices`; missing key → `OracleNotConfigured` panic |
| No silent lazy price | `cached_price` panics if unwarmed |
| Strategy early snapshot | `prefetch_strategy_prices` before movement / callback |
| Risk-gate warm-up | `calculate_account_risk_totals` → `load_markets` |
| Liquidation warm-once | Plan totals before any `cached_price` in math |
| Touched-hub indexes | Mutation return → `put_market_index` (A077/A094) |
| Untouched-hub indexes | Simulated `get_bulk_indexes` (pool simulate-only) |
| Dedup | `collect_uncached_keys` / `unique_hub_tokens` bound by position limits |

**What batching does not skip:** post-pool HF/LTV/min-borrow (`require_post_pool_risk_gates`), hub-active / listing / flash guards, Cap checks tied to mutation indexes (A077).

### 3.3 Memo short-circuits are not “failed-check skips” (A099)

| Optimization | Behavior | Why safe |
|---|---|---|
| `verified_hubs` | Skip repeat `require_hub_active` after **success** | Failure never recorded; next call re-runs storage |
| Debt-free skip of post-pool solvency | `account.debt_free()` → return early | No borrow risk; intentional (A072) |
| Credit seize fee-only usage | Skip supply-cap on share re-entry | Debit+credit cancel; documented (A084) |
| Ordinary debt-free supply | Often no oracle / no HF gate | No debt book to undercollateralize |

Flash / reentrancy guards (A007) bound the hypothetical “hub deactivated mid-call” story for `verified_hubs`.

### 3.4 Post-leg index truth for money math (A094 defense half + A077)

For current merge helpers (`merge_*_leg`, supply/debt paths):

1. Pool returns `PoolPositionMutation` with accrued indexes.
2. Controller builds `LegOutcome` from **pool outputs**.
3. Cap / usage apply uses that index (A077).
4. `put_market_index` refreshes Cache for later risk reads.

Skipping a re-fetch after mutation is therefore **safe when and only when** step 4 runs. That is the A094 partial.

### 3.5 Adjacent defended opts (unfiled Wave-6, peer-owned)

| Concern | Peer | Claim |
|---|---|---|
| Finalize write batch | A032 | Risk gates before `finalize_position_flow`; batching does not skip solvency |
| Event vs storage order | A033 | Persist usage → positions → emit; buffers observational |
| Persist after pool success | A078 | Durable spoke usage never commits ahead of failed pool leg |
| View TTL skip | A034 / A008 | `new_view` never bumps instance TTL |
| Cap uses mutation index | A077 | Prefetch/simulate cannot under-cap a write |

---

## 4. Residual hazard classes (quantified)

### 4.1 Ranked table

| Rank | Hazard class | Owning IDs | Status | Max quantified impact |
|---|---|---|---|---|
| 1 | Forgotten `put_market_index` after new pool FFI merge | **A094** (flagged by A087, PRELIMINARY) | partial | **Same-tx** wrong USD risk and/or wrong usage-cap scaling for the touched hub: can **false-reject** (availability) or **false-admit** relative to post-accrual index until tx ends. Blast radius = that account’s attempted leg / HF decision in **one transaction**. Not cross-market theft; not durable controller index SoT (pool remains SoT). |
| 2 | `pool_sync_data` never invalidated post-mutation | **A086** (deep-dive owed **A088**) | defended w/ residual | Today money paths after legs use mutation indexes + positions, not sync params. Practical impact ≈ **negligible** unless new code re-reads flags (e.g. `is_flashloanable`) **after** a same-tx pool param/state change. Upper bound if misused: wrong **boolean/param** gate for that hub within the tx (revert or unintended allow of a flash-like check) — still no share mint. |
| 3 | Inherited usage “skip”: `apply_exit` no-op on missing row | **A080** via **A099** | partial (T5) | Spoke cap **under-count** → temporary over-admission up to that spoke’s remaining headroom; **no direct theft**; supplier risk only if over-admission later goes bad (PRELIMINARY). |
| 4 | Keeper N+1 lazy `cached_market_index` before HF bulk load | **A087** | optimization-note | Fee/CPU budget waste; correctness OK (later `load_markets`). DoS only against fee payer / tx limits. |
| 5 | Strategies prefetch prices only (not indexes) | **A087** | optimization-note / by design | Finalize residual bulk fetch; not silent mis-valuation. Missing price still fail-closed. |
| 6 | Mid-tx price “staleness” vs post-callback positions | **A087** + ADR-0005/0020 | policy (defended) | Not a bug: coherent snapshot. Impact of “refreshing” would be oracle inconsistency across legs, not a fix. |
| 7 | Cross-ledger simulate vs other tx accrual | **A094** / **A098*** | accepted concurrency | Normal chain race; within one invocation mutation overwrite closes the gap. |
| 8 | `cached_price` without prior warm-up | **A087** future API footgun | fail-closed | Panic / whole-tx revert — availability, not wrong HF. |
| 9 | Debt-free / Credit fee usage “skips” | **A099**, A072, A084 | defended policy | No fund-theft vector identified from these short-circuits. |

\*A098 unfiled; concurrency claim is A094’s.

### 4.2 Impact model detail — A094 footgun

**Trigger:** New pool-mutation merge path ships without `put_market_index` (and possibly without `apply_leg_usage` — checklist pairs them).

**What stays correct anyway:**

- Share/cash amounts for the leg still come from pool mutation outputs (A082 adjacency).
- Cap check at `apply_entry` for **that** leg still uses the mutation’s index if `apply_leg_usage` is wired (A077).

**What goes wrong:**

- Later `load_markets` / `cached_market_index` for the same hub may still hold the **pre-leg simulated** index.
- Post-pool HF / LTV / liquidation post-totals / subsequent legs’ risk views for that hub use stale index → USD collateral/debt mis-scaled by the accrual delta between simulate and mutation.

**Bound:** Let \(I_{sim}\) be simulated index and \(I_{mut}\) post-accrual. Relative error on that hub’s contribution ≈ \(|I_{mut}/I_{sim} - 1|\). Over a short ledger gap this is typically tiny; over long idle markets it can be material **for that asset’s WAD contribution in that tx only**. Durable pool state is unaffected; next tx re-simulates or mutates fresh.

**Severity rationale (low, not medium):** Requires a **future code regression**; current merges call `put_market_index` (A094 evidence). PRELIMINARY correctly lists it as residual footgun, not a live exploit.

### 4.3 Impact model detail — A086 `pool_sync_data`

**Current production readers of `cached_pool_sync_data`:**

| Site | When | Hazard if stale |
|---|---|---|
| `flash_position` `is_flashloanable` | Pre-leg gate | Stale only if sync cached earlier **then** params change same tx before check — not the observed order |
| `views.rs` decimals | View path | Observability; `new_view` fresh Cache per call |
| Admin/config paths | Often direct `fetch_pool_sync_data` | Bypass Cache |

**Bound:** Mis-read of pool params/flags for one hub within one mutator invocation. No automatic path today re-validates flashloanable **after** mutating that hub’s pool state using a prior Cache hit.

### 4.4 Impact model detail — A099 → A080

A099’s hunt found **no** Cache memo that skips a **failed** security check. It explicitly elevates A080 as the leading “optimization / tolerance” hazard:

| Distortion | Direction | User-visible effect |
|---|---|---|
| Usage row missing while positions exist | Cap occupancy understated | Extra admissions until reconcile |
| Usage overstated vs positions | Cap occupancy overstated | False cap hits (availability) |

Neither moves tokens without pool mutations; both are **governance soft-cap** distortions.

### 4.5 What is **not** a Cache hazard (non-findings)

| Claim | Why rejected |
|---|---|
| Prefetch freezes wrong cap index on mutating leg | Caps use mutation index (A077) |
| Price memo skips oracle failure | Hard `prices` panics; strategies prefetch fail-closed (A087) |
| `verified_hubs` skips inactive hub forever after fail | Failures not memoized (A099/A063) |
| Finalize batching skips HF | Gates run before finalize (A032) |
| Soft view quotes poison money paths | Detailed views use soft status separately; money uses hard Cache (A087) |
| Certora `fetch_market_indexes` no-op | Feature-gated harness; not production WASM (A087) |

---

## 5. Loss-class summary (T8 quantification)

| Loss class | Can Cache opts cause it? | Filed evidence |
|---|---|---|
| Protocol-wide share mint / free cash | **No** | A086–A099; money synthesis A101 |
| Undercollateralized gated borrow/withdraw left on-chain | **No** (unless A094 footgun + extreme index drift fools HF **and** gates still “pass”) | A072 + A094; fail-closed oracle |
| Account-local swap slippage | **No** — T3 aggregator trust (A048/A056) | Out of Wave 6 |
| Market TVL desync from lying tokens | **No** — listing trust (A055) | Out of Wave 6 |
| Spoke capacity distortion | **Indirect** via A080 (A099 pointer), not via price/index maps | A080 |
| Same-tx wrong HF/cap decision | **Yes if** `put_market_index` omitted | A094 |
| Fee/CPU DoS | **Yes** (keeper N+1; uncapped Vecs are A062/A015 not Cache) | A087 |
| Instance rent grief via views | **Mitigated** by `new_view` | A086/A034/A008 |

**Single-account max from pure Cache bugs filed here:** incorrect accept/reject of that account’s action in one tx (revert or unintended pass of HF/cap), not extraction of another account’s balances.

**Market/protocol max from pure Cache bugs filed here:** none durable; pool remains index SoT; next successful mutation refreshes.

---

## 6. Cross-link matrix

### 6.1 Filed Wave-6 peers

| From → To | Relation |
|---|---|
| A086 → A094 | Inventory notes `put_market_index` overwrite; sync-data gap is sibling of index staleness |
| A086 → A099 | Verified hubs / spoke pin are memo surfaces A099 hunts |
| A087 → A086 | Batching is the price/index slice of the inventory |
| A087 → A094 | Touched hubs must mutation-refresh; forgotten put is shared footgun |
| A087 → A077 | Caps/usage trust mutation indexes, not prefetch |
| A087 → A099 | Memo short-circuits independent of price maps |
| A087 → A032 / A045 / A046 | Finalize / flash / multiply rely on early snapshot + late gates |
| A094 → A077 | Mutation index is SoT for writes; Cache must track it |
| A094 → A087 | Simulated bulk vs post-accrual overwrite |
| A099 → A080 | Leading skipped-check residual is usage exit no-op |
| A099 → A007 / A084 | Reentrancy bound; Credit fee-only usage intentional |
| A099 → A063 | `verified_hubs` success-only (A090 adjacency) |

### 6.2 PRELIMINARY alignment

| PRELIMINARY claim | A104 confirmation |
|---|---|
| Cache memoization with spoke pin + post-leg refresh is a strong defense | **Confirmed** (A086, A087, A094 defense half) |
| A094 future forgotten `put_market_index` → wrong HF/caps in-tx | **Confirmed** as top Wave-6 residual |
| A080 capacity distortion | **Confirmed** as A099’s redirect; not re-owned as Cache map bug |
| A087–A093, A095–A098, A100 still pending | **Confirmed** — only A087 among those has filed |

### 6.3 Themes

| Theme | How Wave 6 touches it |
|---|---|
| T6 read/write savings | Prefetch, finalize batching, avoided re-reads |
| T7 in-memory Cache | All A086–A100 maps and memos |
| T8 gap + impact | This file; feeds A106/A110 backlog |
| T5 spoke usage | Lifecycle embedding (A091 hole); A080 via A099 |
| T4 validation | Debt-free solvency skip; hub-active memo |
| T3 money | Prices/indexes feed HF; do not replace measured custody |

---

## 7. Coverage holes (unfiled A088–A093, A095–A098, A100)

Synthesis does **not** invent severity for missing agents. Adjacency only:

| Missing ID | Manifest intent | Best adjacency already on disk | What A104 still cannot close |
|---|---|---|---|
| A088 | `pool_address` / `pool_sync_data` deep-dive | A086 gap on sync-data invalidation; `context/pool.rs` fill-once | Exhaustive call-site timing for every sync-data read vs mutation |
| A089 | spoke_config / spoke_assets memo | A086 `reset_spoke_context`; A063 listing | Multi-spoke edge cases beyond liquidation receiver notes (A084) |
| A090 | verified_hubs correctness | A099 + A063 | Dedicated same-tx reentrancy matrix solely for hub memo |
| A091 | spoke_usage in Cache lifecycle | A078 persist; A086 pin; A080 | Full pin/reset/multi-account map of usage buffers |
| A092 | Event buffer coalesce | A033 order defended | Coalesce/dedup semantics of `supply_updates`/`debt_updates` |
| A093 | new vs new_view TTL | A034 inventory; A008 rent grief | Any mutator accidentally on `new_view` (A034 claims none) |
| A095 | Read savings vs correctness tradeoffs | A087/A094 narrative | Systematic “saved read” catalogue with invariant mapping |
| A096 | Account load shapes | — | Borrow-only / supply-only / full load omissions |
| A097 | Finalize write batching | A032 defended | Diff ordinary vs strategy vs liq finalize write sets under T7 |
| A098 | Index vs live accrual races | A094 concurrency paragraph | Formal same-ledger multi-tx race model |
| A100 | Dead cache paths | A086 all fields appear used | Dead-code / unused map proof |

**Recommendation:** When those files land, merge into this ranking; expect A088 to either **confirm** A086’s low sync-data residual or elevate it if a post-mutation sync-data safety check is found.

---

## 8. Agreements and disagreements

### 8.1 Agreements

- A086, A087, A099: Cache memos are **defended** for current production paths.
- A087 ↔ A094: Mutation overwrite is mandatory; simulate-only bulk is not post-leg truth.
- A099 ↔ A063: `verified_hubs` success-only.
- A094 ↔ A077: Pool mutation indexes gate usage/caps.
- A087 ↔ ADR-0005/0020: Frozen prices are policy, not accidental staleness.
- PRELIMINARY ↔ A104: A094 is the Wave-6 leading residual; A080 remains the sharper capacity issue via A099.

### 8.2 Disagreements

**None.** No filed Wave-6 peer claims a critical Cache bypass that another peer denies. Severity spread (info vs low on A094/A099) is consistent with “defended design + footgun/residual.”

### 8.3 Tension to watch (not a disagreement)

A086 rates overall **defended** while calling out sync-data non-invalidation; A094 rates **partial** for the index overwrite footgun. Synthesis treats both as **residuals under a defended architecture**, with A094 higher priority because risk/cap math **does** re-read `market_indexes` after legs, whereas money paths largely **do not** re-read `pool_sync_data` after legs today.

---

## 9. Remediation backlog (for A110)

Priority ordered for Cache/optimization only:

| P | Action | Closes / reduces | Effort shape |
|---|---|---|---|
| P1 | Code-review checklist + optional lint: every pool mutation merge calls `put_market_index` (+ `apply_leg_usage`) | A094 | Process / static check |
| P2 | Document Cache invalidation rules in `context/` module docs (prices immutable; indexes overwrite; sync-data fill-once; spoke reset API) | A086, A088* | Docs |
| P3 | Consider clearing `pool_sync_data` for touched hubs after mutations if any post-leg safety reads are added | A086 / A088* | Small code if needed |
| P4 | Keeper: prefetch supply hub indexes once before ParamUpd loop | A087 | Budget only |
| P5 | Optional strategy `fetch_market_indexes` before legs | A087 | Budget only |
| P6 | Unit test first-pass for `fetch_market_indexes` (mirror oracle tests) | A087 | Test hygiene |
| P7 | Do **not** mid-tx refresh oracle after legs | A087 / ADR-0005 | Anti-remediation |
| P8 | A080 reconcile / invariant (owned by T5, flagged by A099) | A080 | Separate backlog |

---

## 10. Verdict

Wave-6 filed evidence supports:

1. **Cache is a correctness amplifier** (fail-closed prices, mutation index overwrite, success-only hub memo, view TTL split), not a bypass layer.
2. **Highest in-wave residual** is the **A094** `put_market_index` footgun — same-tx HF/cap distortion if future merges regress; quantify as account/tx-local, not protocol drain.
3. **A086** sync-data non-invalidation is a real incomplete rule with **low practical impact** on today’s call graph.
4. **A087** residuals are budget/hygiene; batching itself is defended.
5. **A099** finds no memo that skips a failed check; it correctly escalates **A080** as the meaningful “skip” residual outside pure Cache maps.
6. **11/15 scopes unfiled** — re-run this synthesis after A088–A093 / A095–A098 / A100; do not treat adjacency as a substitute for those deep-dives.

**Bottom line:** No novel Critical/High Cache optimization hazard is established by A086–A100 filings present on disk. Ship checklist discipline for index overwrite; keep coherent price snapshots; finish the missing Wave-6 agents before freezing T7 in the final backlog (A110).
