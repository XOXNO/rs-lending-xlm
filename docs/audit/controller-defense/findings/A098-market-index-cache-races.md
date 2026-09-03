# A098 — Market index cache vs live accrual races within one tx

- Agent: A098
- Theme: T6 / T7 (in-memory `Cache` vs pool live accrual); T5 adjacency for cap/HF consumers
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/context/market_index.rs:11–42` (`put_market_index`, `fetch_market_indexes`, `cached_market_index`)
  - `contracts/controller/src/context/mod.rs:25–74` (`Cache.market_indexes`; `load_markets`)
  - `contracts/controller/src/context/pool.rs:19–28` (`cached_pool_sync_data` — **raw** indexes, fill-once)
  - `common/src/collections.rs:24–45` (`collect_uncached_keys` — hit skips re-simulate)
  - `contracts/pool/src/lib.rs:168–174,194–203,221–224,303–316` (`update_indexes`; `flash_loan`; `seize_positions`; simulate-only `get_bulk_indexes`)
  - `contracts/pool/src/ops/mod.rs:29–72` (`synced_market` / `run_batch` per-leg load→accrue→commit)
  - `contracts/pool/src/ops/flash.rs:40–70,76–87,134–140` (accrue in RAM **before** callback; commit **after**)
  - `contracts/pool/src/ops/seize.rs:18–35` (accrue then optional supply-index **writedown**)
  - `contracts/pool/src/interest.rs:16–91` (`global_sync` no-op at `elapsed_ms == 0`; `apply_bad_debt_to_supply_index`)
  - `contracts/pool/src/time.rs:15–20` (`now_ms` from ledger timestamp)
  - `common/src/rates/simulate.rs` (`accrue_step` shared with write path)
  - `contracts/controller/src/positions/{mod,supply,debt}.rs` (`merge_*_leg` → `put_market_index`)
  - `contracts/controller/src/strategies/legs.rs:154–217` (net-settle double-put same DTO)
  - `contracts/controller/src/strategies/{flash_loan,flash_position}.rs` (callback under flash guard)
  - `contracts/controller/src/positions/liquidation/{mod,plan,apply,bad_debt,math}.rs` (plan simulate → apply mutations → `post_totals` → then cleanup writedown)
  - `contracts/controller/src/risk/totals.rs:169–185` (`load_markets` then `cached_market_index`)
  - `contracts/controller/src/keepers.rs:16–24,207` (accrue-only; lazy index for ParamUpd events)
  - `contracts/controller/src/spec_hooks.rs:15–19` (Certora no-op bulk fetch)
  - `certora/controller/spec/solvency_rules.rs:419–429` (`index_cache_single_snapshot`)
- Defense: Inside one Soroban invocation the ledger timestamp is frozen, so time-based accrual is a **single** `now_ms` projection. Pool `get_bulk_indexes` and mutating `global_sync` share `accrue_step` (INV-IDX-04). After the first successful accrue+commit for a hub in this ledger, further pool legs see `elapsed_ms == 0` and leave indexes unchanged except **INV-IDX-03 writedown**. Controller `Cache.market_indexes` is an overlay: simulate-fill for untouched hubs, unconditional `put_market_index` from every production position/net-settle mutation DTO, and `fetch_market_indexes` **refuses** to overwrite a present key. Risk gates after legs therefore consume post-mutation indexes for touched hubs and isomorphic simulate for the rest. Untrusted callbacks cannot mutate the pool (`#[only_owner]` = controller) or re-enter position flows (flash guard).
- Gap: (1) **A094 process residual:** a future merge that mutates the pool without `put` leaves the pre-leg simulate in Cache; `load_markets` will not refresh it. Current helpers put. (2) **Seize / bad-debt FFI returns no `MarketIndexRaw`:** controller never `put`s those hubs. Safe on today’s graph because `post_totals` run **before** Borrow-side writedown, Credit fee absorb does not change indexes, and Credit sizing is share-denominated. (3) **Cash flash:** pool holds accrued indexes in RAM uncommitted across the callback; controller Cache is empty of indexes on that entrypoint and does not value positions afterward. (4) **`pool_sync_data` is a different number:** raw `State` indexes, fill-once, not used for HF. (5) Certora `index_cache_single_snapshot` proves memo **stability**, not simulate↔mutate isomorphism or put-after-leg (those live in pool/common rate rules + call-graph review). (6) Host time advancing mid-invocation is not a present Soroban model; if it ever were, Credit/seize no-put would become a real same-Cache split.
- Impact: **No in-tx cache-vs-accrual race that under-collateralizes, over-seizes, or forks durable indexes on production paths.** Blast radius of a forgotten `put` is the **same invocation’s** HF/cap/event stamps for that hub only (A094/A106 S10 class) — Cache dies at return; pool `PoolKey::State` remains SoT (A038). Bad-debt supply writedown is visible to the **next** transaction via simulate/mutations, not via this Cache. Cross-ledger “another tx accrued between my simulate and my mutate” is ordinary chain serialization (A094), **out of this scope**.
- Evidence: INV-IDX-01..05 (esp. 03 writedown, 04 zero-elapsed no-op, shared `accrue_step`); ADR-0003 (shares × index); ADR-0016 (chunked time); ADR-0020 (price freeze, not index freeze); formulas.md interest/index; pool README simulate vs raw sync; harness `liquidation_accrual_timing.rs`, `bulk_indexes.rs`, `bad_debt_netting_and_exit_timing.rs`; Certora pool isomorphism + `index_cache_single_snapshot`; peers A038, A077, A081, A086–A088, A094, A095, A104 §7 hole this file closes.
- Opinion: **Within one transaction, the market-index Cache is not racing live accrual; it is a simulate-then-overlay of a timestamp-frozen market.** Treat “race” as four distinct phenomena and keep them separate: (A) time accrual vs simulate — isomorphic; (B) second leg same ledger — no further time accrual, Cache `put` matches commit; (C) non-time index mutation (bad debt) — must not be read from Cache after the FFI without a put or a re-fetch, and current callers do not; (D) forgotten `put` after a position mutation — engineering footgun, not a live bug. Do not “fix” by re-fetching after every leg (budget) or by dual-writing indexes onto the controller. Document seize-without-put next to the overlay contract.

---

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, README format, `AGENT_MANIFEST` Wave 6 **A098**, and peers **A038** (persistence / deferred race taxonomy), **A094** (stale Cache after pool), **A077/A081** (caps use mutation DTO), **A086/A088** (sync fill-once vs index overwrite), **A087/A095** (prefetch / savings), **A072** (post-pool HF), **A007** (flash guard), **A104** (explicitly listed A098 as unfiled hole).
2. Fixed the **time model**: `now_ms` is `ledger.timestamp * 1000`, constant for the invocation.
3. Split **two caches**: pool per-leg RAM (`ops::Cache`, commit to `PoolKey::State`) vs controller per-invocation `Cache.market_indexes`.
4. Built a **race taxonomy** (simulate vs mutate, skip-refetch, second accrue, writedown, flash uncommitted window, sync-blob confusion, callback, Certora).
5. Walked every production entrypoint that both fills indexes and talks to the pool in one Cache lifetime.
6. No production Rust edited. No git operations.

Out of primary claim: IRM arithmetic (IDX suite), oracle snapshot policy (ADR-0005 / A087), durable SoT (A038), cap field selection (A081).

---

## 1. Scope vs peers

| Peer | Owns | This file adds |
|---|---|---|
| A038 | Durable SoT is pool; Cache is ephemeral | **Same-tx race taxonomy**; when simulate, put, and live commit can disagree *during* the invocation |
| A094 | Forgotten `put` after position FFI; cross-ledger concurrency note | Distinguishes that note from **intra-invocation** races; shows skip-refetch is *required* after a good `put` |
| A077 / A081 | Caps consume mutation `market_index`, not Cache re-read | Confirms entry caps are immune to Cache overlay races; HF/events are not |
| A086 / A088 | Sync blob fill-once, no invalidation | Sync **raw** indexes vs simulated Cache — a lookalike race that is not HF |
| A087 / A095 | Bulk simulate savings | Savings are safe **because** of (A)+(B) below, not despite them |
| A104 | Marked A098 unfiled; cited A094 concurrency | Closes the hole with a verdict |
| A007 / A045 | Flash guard / flash-position money | Callback cannot be an accrual writer |

**“Within one tx”** means one controller (or pool-as-callee) invocation: nested FFI, one frozen ledger timestamp, one controller `Cache`. It does **not** mean two user transactions in the same ledger close — that is A094’s ordinary concurrency paragraph.

---

## 2. Time and accrual model (why a race is even conceivable)

```
ledger.timestamp  ──×1000──►  now_ms     (constant for the whole invocation)
pool State.last_timestamp                 (durable; may be ≪ now_ms)
elapsed_ms = now_ms - last_timestamp
needs_accrual ⇔ elapsed_ms > 0
```

- **Simulate (no write):** `get_bulk_indexes` loads `get_sync_data` and runs `simulate_update_indexes(now_ms, sync)`.
- **Mutate:** `synced_market` → `global_sync` → same `accrue_step` chunks → later `commit` writes indexes **and** sets `last_timestamp` to `now_ms` (`mark_accrued`).
- **Second mutate, same hub, same invocation:** load from storage; `elapsed_ms == 0`; `global_sync` returns immediately. Books (cash, shares) still change; **indexes do not**, unless the op is Borrow-side seize writedown.

INV-IDX-04 is the protocol’s explicit answer to “does utilization change mid-tx reprice interest?” **No.** Cadence is per successful accrue to `now`. Finer *cross-tx* cadence can realise more interest (documented in invariants.md); that is not a Cache bug.

**Corollary (A):** For a hub that has not yet been mutated in this invocation,

`cached_market_index` after a miss **equals** the indexes a subsequent position mutation will commit **from time accrual**, at this `now_ms`, from the same stored books.

Utilization after the mutation differs; that does not move indexes until a **later ledger’s** accrue.

---

## 3. Two-layer caches (do not conflate)

```
Controller invocation
  Cache.market_indexes     Map<HubAssetKey, MarketIndexRaw>   empty at new/new_view
       │ fill:  get_bulk_indexes (simulate)
       │ put:   mutation DTO (post-accrue, post-leg)
       │ skip:  fetch skips keys already present
       └── dropped at return; never ControllerKey

  nested pool FFI (one market per load)
       pool Cache          RAM: indexes, books, last/current timestamps
       commit()            PoolKey::State  ← durable SoT
```

Pool comment (`cache/mod.rs`): there is **no** storage lock; callers must not interleave commits for the same market without reload. Production `run_batch` loads a **fresh** pool Cache per entry, so the pool side cannot hold a stale RAM index across controller logic. The controller overlay **can** — that is the only interesting race surface.

`pool_sync_data` on the controller Cache is a **third** number: raw committed `State` (possibly behind `now_ms`). Fill-once (A086/A088). Money valuation uses `cached_market_index`, not this blob. `flash_position` reads only `params.is_flashloanable` from it, **before** legs.

---

## 4. Race taxonomy

Each row is a hypothesised “Cache said X, live pool said Y” inside one invocation.

### 4.1 Simulate fill, then position mutation, then HF (`load_markets` hit)

| Step | Pool durable | Controller Cache |
|---|---|---|
| `load_markets` / lazy miss | unchanged | `I_sim = simulate(now, State0)` |
| `supply`/`borrow`/… | accrue + books; commit `I_sim` (time) + new books | still `I_sim` until `put` |
| `merge_*_leg` | — | `put(I_mut)` = mutation DTO = `I_sim` for time |
| `enforce_post_pool_solvency` → `load_markets` | `State1` at `last=now` | hit; **does not** re-simulate |

**Verdict: no race** on current merges. Indexes used at the gate match the committed post-accrue indexes. Scaled positions in `Account` already reflect the mutation; valuation is `scaled × index` with the overlay.

If `put` were omitted: Cache keeps `I_sim`. For **time-only** legs that is still `I_mut`. The skip-refetch would *accidentally* still be correct for interest. The A094 footgun is therefore **latent until a non-time index change** (writedown) or until someone assumes Cache tracks books-derived quantities that indexes do not represent.

### 4.2 Fetch-after-mutation without a prior fill

`cached_market_index` miss after a committed pool leg would `get_bulk_indexes` on `State1` with `elapsed=0` → returns stored post-leg indexes. **Safe.** The dangerous order is **fill then mutate then skip**, not mutate then fill.

### 4.3 Two position legs, same hub, same batch or sequential FFI

`run_batch` / two controller pool calls: first accrue+commit; second `elapsed=0`. Each `merge_*` `put`s. Second DTO’s indexes equal first’s (time). Utilization and cash differ; HF after the second leg uses the second `put` (same indexes, new scaled amounts).

**Verdict: no race.** This is INV-IDX-04, not a Cache bug. Harness `bad_debt_netting_and_exit_timing.rs` pins `elapsed_ms == 0` on the second leg.

### 4.4 Multi-asset: mutate A, value A and B

`load_markets([A,B])` fills both simulates. Mutate A, `put(A)`. `calculate_account_risk_totals` loads `[A,B]`; A hit (put), B hit (simulate). B was never mutated → simulate still matches B’s live accrue-if-touched.

Markets are independent (`INV-IDX-03` isolation: seize writedown is per `hub_asset`).

**Verdict: no cross-asset index race.**

### 4.5 Non-time index mutation — Borrow seize writedown (the real disagreement class)

`ops/seize.rs`: `synced_market` (time accrue, usually no-op if a prior same-tx accrue already committed `last=now`) then `apply_bad_debt_to_supply_index` **lowers** `supply_index`, then `commit`.

Controller **does not** receive `MarketIndexRaw` and **does not** `put`.

| Call graph today | Cache vs live after seize | Used afterward? |
|---|---|---|
| Liquidation `post_totals` **then** `check_bad_debt` → `execute_bad_debt_cleanup` | Cache still pre-writedown; pool State already written down **after** totals | Totals already snapshotted; account deleted; no second HF |
| Standalone `clean_bad_debt` / `force_socialize` | Totals from simulate **before** seize | Event USD only; account burned |
| Credit fee seize (`Deposit` absorb) | Accrue may commit; **indexes unchanged** by absorb | `post_totals` uses plan-time simulate ≡ accrue-at-same-now |

**Verdict: defended on the current graph; not a general “seize is fine.”** A future path that values remaining suppliers or the liquidated book **after** Borrow-side seize **with the same Cache** would overstate collateral (`supply_index` too high) until `put` or map-delete+refetch. That is A038’s hygiene note, now classified as **the only production-shaped intra-tx Cache/live index split**.

### 4.6 Cash `flash_loan`: uncommitted pool RAM vs storage during callback

`prepare` = `renewed_market` = accrue **in pool RAM**. Callback runs **before** `finalize`/`commit`. During the callback:

- Durable `State` still has old `last_timestamp` / old indexes.
- `get_bulk_indexes` (permissionless) **re-simulates** from that storage → `I_sim` = RAM accrued indexes (same math, books not yet fee-minted).
- Fee mint (`add_protocol_revenue`) after callback **does not change indexes**.
- Then commit.

Controller `process_flash_loan` never fills `market_indexes` and does not run HF. Receiver cannot call pool mutators (`#[only_owner]`). Receiver cannot re-enter controller money paths (`require_not_flash_loaning` / `with_flash_guard`).

**Verdict: no controller Cache race.** The uncommitted window is pool-internal; views during callback that simulate from storage still match RAM accrued indexes. Do not teach future code to `cached_pool_sync_data` *during* a flash for indexes — that blob would still be **pre-accrual raw** if it had been filled before `prepare` (it is not, on this entrypoint).

### 4.7 `flash_position`: mutate → put → callback → deposit → HF

`create_strategy` **commits** before the token send (`strategy::accounting` → `commit`). Controller `merge_debt_leg` `put`s. Callback is under flash guard. Collateral `process_deposit` merges + `put`s. `strategy_finalize` → `load_markets` hits puts for touched hubs.

Prices are frozen (ADR-0020); indexes of touched hubs are mutation-fresh. Untouched portfolio hubs still have simulate-at-`now` if warmed, or lazy-simulate at finalize.

**Verdict: no race.** Callback cannot accrue the pool.

### 4.8 Liquidation plan vs apply (the “stale price + fresh index” worry)

Harness `liquidation_accrual_timing.rs` states the design: plan already reads accrued-to-`now` via `get_bulk_indexes`; repay/seize legs then accrue with `elapsed_ms == 0`. Pre-keeper `update_indexes` then liquidate is bit-identical to liquidate directly at the same ledger time.

Transfer seize: `merge_withdraw_leg` `put`s (same indexes). Credit seize: share math; events stamp **plan** `entry.market_index`; no put required for correctness of share moves (`apply.rs` comment).

**Verdict: no liquidator-payoff race from Cache vs live accrual.** Asymmetry vs prices is oracle policy, not index policy.

### 4.9 `pool_sync_data` vs `market_indexes` (lookalike, different invariant)

| | `cached_market_index` | `cached_pool_sync_data` |
|---|---|---|
| Source | simulate or mutation put | raw `get_sync_data` |
| After a position mutation | put overwrites | **stale raw** if filled before the leg |
| HF / liq math | yes | no |
| Production money-path use of indexes in the blob | none | none (`is_flashloanable` only, pre-leg) |

Using sync `state.borrow_index` for valuation would be a **raw-vs-simulated** race with live accrual even **without** a later mutation. Production does not.

**Verdict: not an index-cache race today; A088 residual if a new reader appears after legs.**

### 4.10 Keeper `update_indexes` / `update_params` / claim / recap

`update_indexes`: pool accrue+commit; controller Cache unused for indexes. **N/A.**

`update_account_threshold`: **no** pool mutate; lazy `cached_market_index` then possibly `load_markets` for HF floor. Simulate-only. **N/A.**

Claim/recap: do not put indexes; they do not revalue a book from Cache afterward as a solvency gate (recap is pool cash). **N/A.**

### 4.11 Views

`new_view` + simulate. No mutation in the same Cache. **N/A.**

### 4.12 Certora / harness (epistemology, not WASM)

| Distortion | What it can hide |
|---|---|
| `fetch_market_indexes` no-op | Bulk warm empty; lazy `cached_market_index` still ghost-fetches in some summaries |
| `index_cache_single_snapshot` | Two hits agree **without** a `put` — forbids silent map corruption, **not** “Cache tracks pool after seize” |
| Harness raw sync helper (A035) | Reads unsimulated State |
| Ghost mutation indexes | May hide forgotten-put (A094/A108) |

**Verdict: do not cite snapshot rules as a proof that overlay stays isomorphic to pool after every FFI.**

### 4.13 Cross-ledger (explicitly out of scope, contrast only)

Another transaction in the same or later ledger can accrue and commit before this invocation’s simulate. This invocation then simulates from **that** State. There is no shared Cache. A094 already labelled this normal serialization.

---

## 5. Overlay rules (the actual contract)

Derived from `market_index.rs` + `collect_uncached_keys` + merge helpers:

1. **Cold start:** map empty.
2. **Miss:** one-key or bulk `get_bulk_indexes` → insert simulate projection.
3. **Hit:** return map; **never** re-talk to the pool.
4. **`put_market_index`:** unconditional overwrite; wins over any later `fetch` for that key.
5. **Therefore** post-leg freshness is **only** via `put` (or never having filled). `load_markets` after a mutation is not a refresh.
6. **Caps:** `apply_spoke_entry` takes the DTO index argument, not a Cache lookup (A077). Overlay races do not under-cap entries.
7. **HF / liq USD / ParamUpd events / detailed views:** Cache overlay.

This is the opposite of the oracle map (no put, freeze on purpose). Mixing the two policies is the usual design footgun (A095 three-way split).

---

## 6. Per-entrypoint timeline (controller Cache lifetime)

| Entrypoint | Index fill | Pool index writers | Post-leg Cache readers | Overlay race? |
|---|---|---|---|---|
| `supply` | none until merge put; supply-only finalize does not HF | `supply` batch | events from DTO | none |
| `withdraw` | none until withdraw merge put | `withdraw` | HF via `load_markets` (hit puts; other collaterals/debt lazy or bulk simulate) | none (time iso) |
| `borrow` | none until debt merge put | `borrow` | HF | none |
| `repay` | put; **no** HF | `repay` | events | none |
| `multiply` / swaps / RWC / migrate | strategy price prefetch; indexes via leg puts + finalize `load_markets` | borrow/supply/repay/net_settle | `strategy_finalize` HF | none |
| `net_settle` (via strategy) | double put same DTO | `net_settle` | later HF | none |
| `flash_loan` | none | pool flash accrue+fee | event fee only | none (§4.6) |
| `flash_position` | put debt; put deposits | strategy + supply | HF after callback | none (§4.7) |
| `liquidate` Transfer | plan `load_markets`; repay/seize puts | repay + withdraw | `post_totals` then cleanup | none; cleanup writedown **after** totals |
| `liquidate` Credit | plan fill; **no put** on collateral hubs | optional Deposit seize (no index change) | `post_totals` on plan simulate | none at same `now` |
| `clean_bad_debt` | totals simulate then writedown | Borrow (+Deposit) seize | none (account gone) | none |
| `update_indexes` | unused | accrue | none | n/a |
| views | simulate | none | return | n/a |

`validate_position_entry_gates` does **not** pre-value HF from indexes (limits/flags). First bulk index read on many risk-increasing paths is **after** the pool, at the solvency gate — so those paths never even hold a pre-leg simulate for the touched hub unless something else warmed it (liquidation plan, or an account that already had other assets loaded).

---

## 7. What can change indexes without `put` (complete list)

From pool commit paths (A038 table, race-filtered):

| FFI | Index effect beyond time accrue | Controller `put`? | Same-tx Cache reader after? |
|---|---|---|---|
| deposit/withdraw/borrow/repay/strategy/net_settle | none (fees mint **shares**, not indexes) | **yes** via merge | HF/events |
| `update_indexes` | time only | no (Cache unused) | no |
| `update_params` | accrue then new IRM | no | no (admin path) |
| `seize` Deposit | time only; absorb shares | no | Credit `post_totals` — iso at frozen `now` |
| `seize` Borrow | **supply_index ↓** | no | only if a later HF — **not today** |
| `flash_loan` | time; fee = shares | no | no controller HF |
| `claim_revenue` / `recapitalize` | no index (recap is cash) | no | no |

Protocol fee paths (`add_protocol_revenue`, liquidation withhold, flash/strategy fee) are **not** index races.

---

## 8. Invariants mapping

| Invariant | Intra-tx Cache implication |
|---|---|
| INV-IDX-01/02 | Bounds on write path; Cache stores copies, cannot exceed what pool returned or simulated |
| INV-IDX-03 | The unique live vs Cache split; must not HF after Borrow seize on stale overlay |
| INV-IDX-04 | Zero elapsed ⇒ simulate identity and second mutate identity; **closes** time-race class |
| INV-IDX-05 | Accrue assignment is pool/common; Cache does not re-split interest |
| INV-RISK-01 | Gate still runs; it consumes overlay. Overlay correctness is this file + A094 |
| INV-ORACLE-03 | Prices freeze; indexes **must not** freeze across a mutation without put — different policy |
| Share formulas | `scaled × index`; scaled from mutation/account, index from overlay |

---

## 9. Tests and rules that actually constrain this scope

| Artifact | What it shows | What it does not |
|---|---|---|
| `liquidation_accrual_timing.rs` | Plan simulate + in-call accrue + optional pre-keeper accrue are payoff-identical at one ledger time | Forgotten `put` |
| `bulk_indexes.rs` | View bulk = `simulate_update_indexes` | Controller overlay |
| `bad_debt_netting_and_exit_timing.rs` | Second same-tx leg `elapsed_ms == 0` | Cache put |
| Pool `interest.rs` unit iso simulate vs `global_sync` | Bit-identical at same stamp | Controller |
| Certora pool isomorphism / rate-index | Simulate body vs accrue | Controller `put` |
| `index_cache_single_snapshot` | Memo stable | Post-FFI freshness |

A108 already proposed a static `merge_*` contains `put_market_index` gate rather than a runtime “skip put” hook. That remains the right regression for 4.1’s latent case, not a new runtime test that would require weakening production.

---

## 10. Gaps, non-findings, disagreements

| Item | Severity | Owner |
|---|---|---|
| Forgotten `put` on new position merge | low (future) | A094 / A104 / A110 RB-08 |
| HF after Borrow seize on same Cache | info — no such path | A038 hygiene; **this file’s 4.5** |
| Sync blob vs simulated indexes | info if new reader | A086/A088 |
| Certora snapshot ≠ overlay proof | info | A035 / A108 |
| Mid-invocation clock change | not in Soroban model | re-open A098 if host model changes |
| Dual-write `ControllerKey::MarketIndex` | **anti-pattern** | A038 |

**Non-findings:** Cache does not lose a time-accrual race to the pool inside one tx; multi-leg utilization does not silently reprice indexes; flash callbacks are not accrual writers; liquidation plan is not a stale-index attack; skip-refetch after `put` is a feature.

**Agrees with A038/A094/A095/A077.** Does **not** upgrade A094 to a live production bug. **Disagrees** with reading A104’s “cross-ledger simulate vs other tx” as an A098 finding — that stays A094; A098 is same-Cache, same-timestamp.

Optional disagreement file vs A094 is unnecessary: complementary, not conflicting.

---

## 11. Remediation / hardening (optional)

1. Rustdoc on `put_market_index` / `fetch_market_indexes`: overlay contract from §5, plus “Borrow `seize_positions` does not update this map; do not `cached_market_index` for that hub after that FFI.”
2. Keep A094 static checklist on merge helpers. Do **not** add `put` from seize unless a DTO exists — prefer “no later Cache read” which is already true.
3. If a composite flow ever HFs after socialization in one Cache: `market_indexes.remove(hub)` then lazy fetch, or return snapshot from pool seize. Not required now.
4. Do not re-simulate after every position leg “to be safe” — that fights `collect_uncached_keys` and the put overlay, and costs FFI.
5. Do not use `cached_pool_sync_data.state.*_index` for valuation.

---

## 12. Verdict

**Market-index Cache vs live accrual within one transaction is defended.** The ledger clock does not move; simulate and the first accrue are the same function; later legs do not time-accrue; production position merges overwrite the overlay from pool DTOs; the only live index mutation that bypasses `put` (bad-debt supply writedown) is ordered after the last Cache-backed valuation, or is share-absorb that does not change indexes.

The word “race” should be reserved for (1) the A094 forgotten-put footgun and (2) a hypothetical post-writedown Cache read. Neither is a present money-path defect. Wave-6 A104 may treat this scope as **filed, status defended, severity info**.
