# A097 — Write batching on `finalize_position_flow`

- Agent: A097
- Theme: T6 / T7 (storage write savings via deferred durable commits; adjacency T2 write-set inventory, T5 usage persist timing)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/positions/mod.rs:206-252` (`PositionSides`, `persist_account_positions`, `finalize_position_flow`)
  - `contracts/controller/src/context/spoke.rs:139-143` (`Cache::persist_spoke_usage`)
  - `contracts/controller/src/spoke_usage.rs:77-82` (`SpokeUsageContext::persist`)
  - `contracts/controller/src/context/events.rs:9-62` (event buffers + `emit_position_batch`)
  - `contracts/controller/src/storage/account.rs:71-103,256-269` (`set_*_positions` / `write_side_map` / `renew_user_account`)
  - `contracts/controller/src/storage/spoke.rs:65-78` (`set_spoke_usage` prune-on-zero)
  - `contracts/controller/src/account.rs:170-176` (`cleanup_account_if_empty`)
  - Ordinary callers: `positions/supply.rs:69-76,181-188`, `positions/debt.rs:69-75,104-111`
  - Strategy tail: `strategies/mod.rs:68-79` (`strategy_finalize` → `Both` + `remove_if_empty=true`)
  - Liquidation: `positions/liquidation/mod.rs:126-147` (victim `Both`; Credit receiver `Supply`)
  - Exception (no finalize): `positions/liquidation/bad_debt.rs:15-60` (direct `persist_spoke_usage` then account delete)
  - Keeper adjacency (not finalize): `keepers.rs:217-236` (direct `set_supply_positions` + `emit_position_batch`)
  - Unit: `contracts/controller/tests/positions/flags.rs:293-387` (`persist_account_positions_*`)
- Defense: Every ordinary verb, every account-touching strategy, and liquidation’s account commits funnel durable controller accounting through one ordered tail — `persist_spoke_usage` → `persist_account_positions` → `emit_position_batch`. Legs mutate only in-memory `Account` maps, `SpokeUsageContext`, market-index memo, and event buffers. `PositionSides` skips rewriting the untouched position map; empty maps remove keys; `remove_if_empty` optionally pairs NFT/meta cleanup. Multi-leg strategies pay **one** finalize after **all** pool legs and after `require_post_pool_risk_gates`. Batching is a rent/CPU footprint optimization under Soroban SAC atomicity; it does **not** replace or reorder solvency, caps, auth, or listing gates.
- Gap: Residuals only (none novel critical): (1) Account **create** writes `AccountMeta` (+ NFT mint) before finalize — intentional identity bootstrap, not mid-leg position churn. (2) Liquidation Credit can finalize **twice** in one tx (victim + receiver), rewriting the same Cache usage map twice; bad-debt after liq may persist usage a third time — idempotent amplification (A078). (3) Keepers’ threshold sync writes supply + emits outside finalize (ParamUpd path; no usage mutate). (4) Bad-debt intentionally skips `finalize_position_flow` / position-batch emit and deletes residual maps that liquidation may have just written (A027). (5) Persist writes **whole** side maps / **all** cached usage rows for touched hubs — one write per key per finalize, not a per-hub delta patch API; still O(touched keys) not O(legs × keys mid-flow). (6) `PositionSides` / `remove_if_empty` / load-shape triad is load-bearing (especially repay borrow-only + Debt + false) — a wrong sides flag is a wipe/orphan hazard, not a batching bypass.
- Impact: No fund-theft, solvency-bypass, or silent book desync from write batching itself. Counterfactual mid-leg `set_*` after every merge would multiply persistent writes and TTL renewals without changing committed end-state under atomicity, and would risk durable controller state ahead of post-pool gates if a future author moved gates after those writes. Blast radius of a **buggy** sides/cleanup choice is one account’s maps / NFT pairing (INV-STOR-03), not protocol-wide theft. Double-finalize on Credit/bad-debt is fee/rent waste, not inconsistent usage (same in-memory map).
- Evidence: INV-STOR-01 (lifecycle renew/remove on write), INV-STOR-03 (empty cleanup pairing), INV-RISK-01 (post-pool gates before durable commit on risk-increasing paths), INV-HALT-03 (usage caps at apply), docs/reference/events.md `UpdatePositionBatchEvent`; peers A022–A027, A032, A033, A034, A072, A078, A079, A084, A087, A092*, A096*, A104; unit `persist_account_positions_writes_both_sides` / `_removes_empty_account`; Certora usage delta rules (via A078).
- Opinion: Finalize write batching is a **defended** T6/T7 storage-saving pattern with gates retained. Treat A032 as the strategy-focused sibling; this file owns the full ordinary / strategy / liquidation write-set matrix. Keep `finalize_position_flow` as the sole ordinary durability chokepoint; do not “optimize” by persisting mid-leg before solvency. Guard `PositionSides` / `remove_if_empty` on any new caller with the same care as A025’s repay triad.

---

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, README finding format, Wave-6 slot A097 in `AGENT_MANIFEST.md`, and A104’s coverage hole (“Diff ordinary vs strategy vs liq finalize write sets under T7”).
2. Read peers A032 (strategy batch short form), A033 (event order), A078 (usage persist timing), A022–A025 (per-verb write sets), A026–A027 (liq / bad-debt), A072 (post-pool risk), A034 (TTL), A079 (multi-asset usage), A104 adjacency.
3. Read production sources for `finalize_position_flow`, `persist_account_positions`, spoke-usage persist, event emit, storage `write_side_map`, and every production caller.
4. Inventory early writes that intentionally sit **outside** finalize (create meta/NFT, keepers, bad-debt).
5. Contrast storage footprint: batched end-of-flow vs hypothetical per-leg durable writes.
6. No production Rust edited. No git operations.

Out of scope as primary claims: measured token custody (T3), auth TOCTOU (T1), A080 exit no-op, oracle/index prefetch (A087), event coalesce semantics deep-dive (A092), account load-shape omissions (A096) except where they couple to `PositionSides`.

---

## 1. What “write batching” means here

Soroban persistent storage charges for each `persistent.set` / `remove` and for TTL bumps. The controller’s accounting keys for a user flow are:

| Key class | Writer in finalize |
|---|---|
| `SpokeUsage(spoke_id, hub)` | `SpokeUsageContext::persist` via `cache.persist_spoke_usage` |
| `SupplyPositions(account_id)` | `persist_account_positions` when `sides != Debt` |
| `BorrowPositions(account_id)` | `persist_account_positions` when `sides != Supply` |
| Existing meta / supply / debt / delegates (TTL only) | `renew_user_account` after side writes |
| Optional full account delete + NFT burn | `cleanup_account_if_empty` when `remove_if_empty` |

Plus one observational publish: `UpdatePositionBatchEvent` from buffered `supply_updates` / `debt_updates`.

**Write batching** = accumulate all leg effects in RAM (`Account`, `SpokeUsageContext.usage`, Cache event vecs, `put_market_index`), then perform the durable writes **once per account finalize** in a fixed order. It is not a cross-transaction batcher and not a pool-side optimization (pool still runs its own batched FFI per verb).

```241:252:contracts/controller/src/positions/mod.rs
pub(crate) fn finalize_position_flow(
    env: &Env,
    account_id: u64,
    account: &Account,
    cache: &mut Cache,
    sides: PositionSides,
    remove_if_empty: bool,
) {
    cache.persist_spoke_usage();
    persist_account_positions(env, account_id, account, sides, remove_if_empty);
    cache.emit_position_batch(account_id, account);
}
```

```218:236:contracts/controller/src/positions/mod.rs
pub(crate) fn persist_account_positions(
    env: &Env,
    account_id: u64,
    account: &Account,
    sides: PositionSides,
    remove_if_empty: bool,
) {
    if sides != PositionSides::Debt {
        storage::set_supply_positions(env, account_id, &account.supply_positions);
    }
    if sides != PositionSides::Supply {
        storage::set_debt_positions(env, account_id, &account.borrow_positions);
    }
    // Every variant writes at least one side.
    storage::renew_user_account(env, account_id);
    if remove_if_empty {
        account::cleanup_account_if_empty(env, account, account_id);
    }
}
```

### 1.1 In-memory merge (no mid-leg durable account maps)

Representative debt merge (supply/withdraw mirrors):

| Step inside `merge_*_leg` | Durable? |
|---|---|
| Compute `old_scaled` from in-memory account | No |
| `apply_leg_usage` → `apply_spoke_entry` / `exit` | No (RAM map + cap check) |
| `put_market_index` | No (Cache only; pool remains SoT — A038/A094) |
| `record_*_position_update` | No (event buffer) |
| `update_or_remove_*_position` | No (in-memory map) |

Only after **all** legs for the flow (and after solvency where required) does finalize flush.

### 1.2 Side-selective persistence (second savings axis)

`PositionSides` is an explicit write-set selector:

| Variant | Supply map | Debt map | Typical caller |
|---|---|---|---|
| `Supply` | write / remove-if-empty | **unchanged on disk** | supply, withdraw, Credit receiver |
| `Debt` | **unchanged on disk** | write / remove-if-empty | repay; borrow without LTV restamp |
| `Both` | write | write | strategies; liquidation victim; borrow with restamp |

Skipping the untouched map avoids a redundant full-map rewrite and avoids accidentally persisting an incomplete in-memory view of the other side (critical for repay’s borrow-only load — A025).

Empty maps call `persistent.remove` (`write_side_map`) — INV-STOR-01 empty-state cleanup without orphaning the other side’s key.

### 1.3 Usage row batching

`SpokeUsageContext::persist` iterates **every row present in the in-memory map** (lazily loaded on apply) and `set_spoke_usage`s once. Zero/zero both sides removes the key. Multi-asset batches therefore pay **one write per touched hub**, not one write per intermediate apply inside nested loops (A079).

`Cache::persist_spoke_usage` is a no-op if no usage context was pinned this invocation.

### 1.4 Event batching (observational twin)

`record_supply_position_update` / `record_debt_position_update` push deltas; `emit_position_batch` publishes **one** `UpdatePositionBatchEvent` then clears buffers (no-op if both empty). Order is persist-then-emit (A033). Keepers reuse the emit helper without the finalize wrapper.

---

## 2. Canonical ordering vs gates

```
pre-pool gates (auth, pause/flash, listing, flags, limits, measured transfers)
    → pool_*_call(s)                    // cross-contract mutation(s)
    → merge_*_leg / apply_leg_usage     // RAM only; entry enforces caps
    → enforce_post_pool_solvency?       // borrow, withdraw; strategies via strategy_finalize
    → finalize_position_flow
         1. persist_spoke_usage
         2. persist_account_positions (+ renew; optional empty cleanup)
         3. emit_position_batch
```

**Security claim of batching:** durability is deferred until after risk-increasing post-pool checks. Under Soroban transaction atomicity, a panic after a successful pool call still rolls back pool + controller together — so mid-leg durable writes would not create a lasting half-applied ledger state. The defended property is therefore:

1. **Footprint / rent** — fewer persistent ops and fewer TTL renew storms on multi-leg flows.
2. **Ordering discipline** — no durable controller books before gates that must see the full post-leg portfolio.
3. **Indexer coherence** — one batch event after storage matches committed maps (A033).

Batching does **not** claim to make failed txs cheaper on the pool side (pool work still ran before revert).

---

## 3. Write-set matrix (ordinary vs strategy vs liquidation)

### 3.1 Ordinary verbs

| Verb | Load shape | Post-pool solvency | `PositionSides` | `remove_if_empty` | Usage direction | Notes |
|---|---|---|---|---|---|---|
| `process_supply` | Full / create | No | `Supply` | `false` | Entry (+cap) | Create may write meta+NFT **before** deposit; finalize still sole position/usage commit (A022) |
| `process_withdraw` | Full | Yes | `Supply` | `true` | Exit | Empty supply can delete account if debt also empty (A024) |
| `process_borrow` | Full | Yes | `Debt` or `Both` if restamp | `false` | Entry (+cap) | Restamp may mutate supply LTV stamps → must persist supply (A023) |
| `process_repay` | **Borrow-only** | No | `Debt` | `false` | Exit | Triad prevents wiping live supply / burning NFT from empty supply view (A025) |

All four call `finalize_position_flow` exactly once at the tail.

### 3.2 Strategies (`strategy_finalize`)

```68:79:contracts/controller/src/strategies/mod.rs
pub(crate) fn strategy_finalize(
    env: &Env,
    account_id: u64,
    account: &mut Account,
    cache: &mut Cache,
) {
    let _ = risk::restamp_listed_supply_ltv(cache, account);
    validation::require_post_pool_risk_gates(env, cache, account);
    finalize_position_flow(env, account_id, account, cache, PositionSides::Both, true);
}
```

Callers (each ends in one finalize): `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, `migrate_blend`, `flash_position`.

| Property | Strategy batch |
|---|---|
| Legs before finalize | Multiple pool FFI + merges (and often swap / callback) |
| Mid-leg `set_supply` / `set_debt` / `persist_spoke_usage` | **None** |
| Risk gates | Always `require_post_pool_risk_gates` before finalize (A032 / A072) |
| Sides | Always `Both` (supply+debt may both move; restamp may touch supply stamps) |
| Cleanup | `remove_if_empty=true` (full close / migrate / flash unwind) |
| Savings vs naïve | N legs → 1 usage flush + ≤2 position map writes + 1 renew + 1 event |

`flash_loan` never opens an account and never calls finalize (pool-internal settle) — out of this defense’s write set.

`borrow_into_controller` / `process_deposit` used as **legs** do not finalize; ownership of durability stays with `strategy_finalize` (A023/A022 note).

### 3.3 Liquidation

| Step | Finalize? | Sides / cleanup |
|---|---|---|
| Repay + seize / Credit share apply (in-memory + usage buffer) | No mid-leg durable maps | — |
| Victim `finalize_position_flow` | Yes | `Both`, `remove_if_empty=false` |
| Credit receiver `finalize_position_flow` (optional) | Yes (second) | `Supply`, `false` |
| `check_bad_debt_after_liquidation` → `execute_bad_debt_cleanup` | **No** finalize | Direct `persist_spoke_usage` after seize; `remove_account_and_burn_nft`; no position-batch |

Liquidation intentionally **skips** `require_post_pool_risk_gates` at finalize (plan HF pre-check; A072 / A105). Write batching still applies: victim positions/usage commit once after all seize/repay legs.

**Credit double-finalize:** both calls share one `Cache`; second `persist_spoke_usage` rewrites the same buffered rows (receiver may have added fee exit / credit legs). Idempotent amplification of storage ops — A078 residual (2).

**Bad-debt after liq:** may rewrite residual position maps in victim finalize, then delete those keys in cleanup — rent waste on the success path that immediately socializes leftovers (A027). Still consistent: cleanup is authoritative deletion.

### 3.4 Production callers of `finalize_position_flow` (complete)

| Site | Args |
|---|---|
| `process_supply` | `Supply`, `false` |
| `process_withdraw` | `Supply`, `true` |
| `process_borrow` | `Debt` or `Both`, `false` |
| `process_repay` | `Debt`, `false` |
| `strategy_finalize` | `Both`, `true` |
| `process_liquidation` (victim) | `Both`, `false` |
| `process_liquidation` (Credit receiver) | `Supply`, `false` |

No other production call sites.

### 3.5 Intentional non-finalize writers (adjacency)

| Path | Writes | Why not finalize |
|---|---|---|
| `load_or_create_account` create | NFT mint + `set_account_meta` | Identity must exist before deposit; positions still wait for finalize |
| `execute_bad_debt_cleanup` | `persist_spoke_usage` + account remove | Positions deleted; no batch event by contract (events.md) |
| `keepers` threshold sync | Conditional `set_supply_positions` + `emit_position_batch` | Param-only refresh; no spoke-usage mutation; renew elsewhere on path |
| Liquidation apply Credit fee exits | Usage buffer only until finalize/persist | Same Cache lifecycle |

These do not undermine the batching defense; they are separate write policies that A022/A027/A015 already scope.

---

## 4. Storage-savings quantification (engineering model)

Let:

- \(L\) = number of pool legs that mutate controller positions in the flow
- \(H_u\) = distinct hubs with usage rows touched (loaded into `SpokeUsageContext`)
- \(S \in \{1,2\}\) = number of position side maps written (`Supply`/`Debt`/`Both`)

**With finalize batching (current):**

\[
W_{\text{ctrl}} \approx H_u + S + R
\]

persistent ops for usage + side maps, plus \(R\) renews of existing account keys (`renew_user_account` walks up to 4 keys that `has`), plus ≤1 event publish. Strategies with \(L \gg 1\) still pay \(W_{\text{ctrl}}\) once.

**Without batching (counterfactual mid-leg persist after each merge):**

\[
W'_{\text{ctrl}} \approx L \cdot (H_{\text{leg}} + S_{\text{leg}} + R)
\]

and event publishes would either N+1 or still need a separate coalesce design. Under atomicity the **committed** end-state matches, but fee budget and TTL churn scale with \(L\).

**Additional savings from `PositionSides`:** repay/borrow-without-restamp write \(S=1\) instead of rewriting an untouched (or, for repay, unloaded) supply map — avoids both rent and the INV-STOR-03 footgun of persisting an empty supply view.

**What is not saved:** pool cross-contract storage, token transfers, oracle reads, or Cache instance renew on `Cache::new`. Batching is controller-account/usage write coalescing only.

---

## 5. Threat / invariant cross-check

| Claim | Holds? | Notes |
|---|---|---|
| Batching skips solvency | **No** | Gates run before finalize on borrow/withdraw/strategies (A072/A032) |
| Batching skips usage caps | **No** | Caps at `apply_entry` pre-persist (A077/A078) |
| Mid-leg durable position maps | **No** on inventoried paths | Only create meta/NFT early |
| Events as SoT | **No** | Emit after persist (A033); buffers cleared |
| INV-STOR-01 renew/remove | **Yes** | Renew after side writes; empty map remove; cleanup on flag |
| INV-STOR-03 NFT↔account | **Yes** when `remove_if_empty` | Repay keeps `false` with borrow-only load |
| Liquidation HF gate at finalize | N/A (by design) | Pre-plan check; not a batching defect |
| Credit / bad-debt multi-persist | Amplification | Same map / delete supersedes — not desync |
| Keeper bypass of finalize | Separate path | No usage books; ParamUpd only |

**STRIDE (storage tamper):** Batching does not enlarge who can write keys. Auth and ownership gates remain on entrypoints. A wrong `PositionSides` on a new caller is a developer footgun (integrity of one account’s maps), not an auth bypass.

---

## 6. Failure modes and residuals

### 6.1 Defended / accepted

| Scenario | Outcome |
|---|---|
| Cap or solvency panic after pool success | Full tx revert; no durable controller usage/positions from finalize |
| Multi-leg strategy fails on last leg | No finalize; all prior controller RAM + pool legs roll back |
| Empty event buffers | `emit_position_batch` no-op |
| No spoke usage loaded | `persist_spoke_usage` no-op |

### 6.2 Residuals (info)

1. **Create-before-finalize meta write** — necessary; positions still batched.
2. **Credit double finalize + optional bad-debt third usage persist** — rent/CPU; A078.
3. **Liq finalize then bad-debt delete** — wasted position write on socialize path; A027.
4. **Whole-map rewrite** — not a sparse patch API; acceptable under position limits.
5. **Sides / load-shape coupling** — repay triad is the sharpest; document on any new `Debt`-only or borrow-only caller (A096 adjacency).
6. **Keepers outside finalize** — consistent for ParamUpd; do not route usage mutations through keepers without adopting finalize or an equivalent persist order.

### 6.3 What would undefend this pattern

- Persisting positions or usage **before** `require_post_pool_risk_gates` / `enforce_post_pool_solvency` on a risk-increasing path “to save a revert”.
- Calling `set_supply_positions` inside `merge_*_leg`.
- Strategy path that returns after legs without `strategy_finalize`.
- `finalize_position_flow(..., Debt, true)` after a borrow-only load (would cleanup from empty supply view).
- Emitting the batch **before** persist and treating events as commit confirmation.

None of these appear in current production callers.

---

## 7. Tests and rules

| Evidence | Covers |
|---|---|
| `persist_account_positions_writes_both_sides` | `Both` writes supply+debt keys |
| `persist_account_positions_removes_empty_account` | `remove_if_empty` + empty maps → account gone |
| Per-verb harness / unit suites (A022–A025 peers) | End-to-end finalize sides flags |
| Certora usage delta rules (A078) | Usage tracks scaled delta through finalize persist |
| `zz_storage_sizing` / INV-STOR-01 | Lifecycle / sizing discipline |

**Coverage gap (hygiene, not severity upgrade):** no single unit test named for “N merges → exactly one `set_spoke_usage` per hub” counting storage ops; behavior is structural (single `persist` call). Optional future assert via storage snapshot counters in harness.

---

## 8. Peer linkage

| Peer | Relationship |
|---|---|
| **A032** | Strategy-focused short form of the same defense; A097 expands write-set matrix |
| **A033** | Persist-before-emit ordering inside finalize |
| **A078** | Usage durable only at finalize (or bad-debt post-seize) |
| **A022–A025** | Per-verb write sets / sides / cleanup |
| **A026 / A027** | Liq finalize + bad-debt exception |
| **A034** | TTL renew co-located with persist |
| **A072** | Post-pool risk before strategy finalize |
| **A079** | Multi-asset usage aggregation feeds one persist |
| **A087** | Read batching sibling; finalize reuses Cache |
| **A092\*** | Event buffer coalesce (unfiled); emit is the flush half |
| **A096\*** | Load shapes couple to safe `PositionSides` |
| **A104** | Listed A097 as Wave-6 hole; adjacency already “defended” via A032 |

**Agreement with A104:** Finalize write batching is defended; solvency is not skipped. This filing closes the A097 hole with the ordinary/strategy/liq diff A104 requested.

**Disagreement:** none with filed peers.

---

## 9. Opinion / remediation

**Verdict: defended.** Write batching on `finalize_position_flow` is the controller’s intentional storage-saving and ordering chokepoint for account + spoke-usage commits. It reduces persistent write amplification on multi-leg flows, pairs correctly with `PositionSides` / empty cleanup, and keeps risk gates ahead of durability on paths that need them.

**Do not change** for “optimization” reasons:

- Move `persist_spoke_usage` earlier than pool success + (where required) solvency.
- Persist mid-leg to “fail faster” after caps — atomicity already reverts; early durable writes only add footprint and policy risk.

**Checklist for new callers:**

1. All legs + merges complete before finalize.
2. Choose `PositionSides` to match what this invocation’s in-memory maps actually represent.
3. Set `remove_if_empty` only when a full empty account should burn (never with an incomplete side load).
4. Prefer `strategy_finalize` for multi-leg account strategies rather than ad-hoc persist.
5. Keep emit after persist.

**Optional hygiene:** storage-op count harness for multi-leg strategy; document repay triad next to `PositionSides` enum docs.

---

## 10. Summary table (agent A097 deliverable)

| Question | Answer |
|---|---|
| Is finalize a storage-saving defense? | **Yes** — coalesce usage + side maps + one event after all legs |
| Does it skip security checks? | **No** — gates remain pre-finalize |
| Ordinary vs strategy vs liq? | 1× finalize each ordinary; 1× `Both`+cleanup strategies; liq 1–2× finalize then optional bad-debt non-finalize |
| Status | **defended** (severity info; residuals are rent/hygiene) |
