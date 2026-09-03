# A092 — Event update buffers (supply/debt) coalesce behavior

- Agent: A092
- Theme: T7 (also T6 batching / T2 observational adjacency)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/context/mod.rs:25-61` (`supply_updates` / `debt_updates` fields; constructors)
  - `contracts/controller/src/context/events.rs:9-62` (`record_supply_position_update`, `record_debt_position_update`, `emit_position_batch`)
  - `contracts/controller/src/events/mod.rs:49-151` (`PositionAction`, `EventDepositDelta`, `EventBorrowDelta`)
  - `contracts/controller/src/events/position.rs:8-22` (`UpdatePositionBatchEvent` contract comments)
  - `contracts/controller/src/positions/mod.rs:148-188,241-252` (`merge_debt_leg` record; `finalize_position_flow`)
  - `contracts/controller/src/positions/supply.rs:348-416` (`merge_supply_leg` / `merge_withdraw_leg` record)
  - `contracts/controller/src/positions/liquidation/{mod,apply}.rs` (Credit two-batch drain; `record_share_credit_updates`)
  - `contracts/controller/src/payments.rs:34-73` (payment **sum** coalesce upstream of buffers)
  - `contracts/controller/src/strategies/{legs,mod,swap_debt,repay_debt_with_collateral,migrate_blend,flash_position}.rs`
  - `contracts/controller/src/keepers.rs:208-236` (`ParamUpd` + emit)
  - Docs: `docs/reference/events.md`, `docs/reference/endpoints.md` §6 / flash_position observer note, ADR-0019
- Defense: Event buffers are **append-only leg queues**, not keyed maps. They do **not** merge/dedup by `HubAssetKey`. Upstream payment aggregation **does** sum duplicate payment legs per hub before pool calls, so ordinary multi-asset verbs emit **one delta per hub per side** from those batches. Many buffered legs are **coalesced into one** `UpdatePositionBatchEvent` at `emit_position_batch`, which then **clears** both Vecs. Credit liquidation relies on that clear between victim and receiver batches. Buffers are not source of truth (A033).
- Gap: (1) Indexer footgun if a consumer “coalesces” by hub and keeps a single `scaled_amount` while also summing `amount` without action awareness — migrate borrow+repay and close-after-partial-withdraw intentionally emit **multiple same-hub legs** in one batch. Docs state one entry per leg and multi-batch Credit order, but do not spell “never merge by hub without reading `PositionAction` / post-state semantics.” (2) Silent `restamp_listed_supply_ltv` before strategy/ordinary finalize can change stored LTV for untouched supply slots with **no** buffered `ParamUpd` leg (observational completeness; not a coalesce bug). (3) No unit test that asserts migrate or `close_position` produce two same-hub deposit/borrow deltas in one batch (harness covers Credit two-batch order). None of these move funds or corrupt durable maps.
- Impact: **No fund theft, share mint, undercollateralized exit, or durable SoT corruption** from buffer coalesce semantics. Worst case is indexer / off-chain reconstructor mis-reading multi-leg same-hub batches (wrong net movement or wrong final share if treating each `scaled_amount` as additive). On-chain positions and spoke usage follow account maps + `persist_*`, not the event Vecs. Blast radius = observability only; severity **info**.
- Evidence: Exhaustive `record_*` / `emit_position_batch` grep under `contracts/controller/src`; `payments::aggregate_*` + A062 duplicate policy; events.md wire tables; unit `contracts/controller/tests/events.rs` (Credit two batches, LiqSeize vs LiqCredit); peers A033, A032, A022–A027, A062, A086, A104 §7 A092 hole; SEED Cache facts; endpoints.md flash_position `FlashPos`+`Supply` observer note.
- Opinion: **Defended by design.** Do not add keyed coalesce inside `record_*` — that would erase intentional multi-leg history (Migrate borrow/repay, RpColNet dual-side, FlashPos+Supply, LiqRepay×N + LiqSeize×M). Keep append + single-batch emit + clear. Optional doc/test remediation only: indexer recipe for same-hub multi-leg batches; assert migrate refund double-borrow-delta; note silent LTV restamp vs `ParamUpd`.

---

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, README finding format, `AGENT_MANIFEST` Wave 6 (A092), peers **A033**, **A032**, **A062**, **A086**, **A104** (A092 listed as coverage hole), adjacency A022–A027 / A078.
2. Read `context/{mod,events}.rs` end-to-end; event payload types; `finalize_position_flow`; every production `record_*` and `emit_position_batch` call site.
3. Distinguished three layers often conflated as “coalesce”:
   - **Payment coalesce** (sum duplicate input legs per hub) — `payments.rs`
   - **Buffer coalesce** (merge deltas in Cache by hub) — **absent**
   - **Emit coalesce** (many legs → one `UpdatePositionBatchEvent`, then clear) — present
4. Enumerated intentional same-hub multi-delta paths and cross-account buffer drain (Credit).
5. Checked whether buffers can leak legs across accounts or survive emit; whether empty emit is a no-op; clone-then-replace clear semantics.
6. No production Rust edited. No git operations (COORDINATION).

No novel Critical/High. Fills A104’s A092 hole: order defended (A033) **and** append/batch/clear coalesce contract is sound.

---

## 0. What “coalesce” means in this scope

| Layer | Mechanism | Coalesce? | Security role |
|---|---|---|---|
| Caller payment Vec | `aggregate_positive_payments` / `aggregate_payments` | **Yes** — sum per `HubAssetKey`, first-appearance order | Prevents double pool apply from duplicate legs (A062) |
| Cache `supply_updates` / `debt_updates` | `push_back` only | **No** — append-only | Preserve one observational row per **leg** |
| Emission | `emit_position_batch` | **Yes** — one contract event containing both Vecs; then clear | Indexer-friendly batch; isolates Credit’s second account |

A092 owns the **Cache buffer + emit** layers. Payment sum is upstream context (A062 owns the duplicate-input policy).

---

## 1. Buffer primitives

### 1.1 Fields and lifecycle

```25:37:contracts/controller/src/context/mod.rs
pub(crate) struct Cache {
    // ...
    supply_updates: Vec<EventDepositDelta>,
    debt_updates: Vec<EventBorrowDelta>,
}
```

| Property | Behavior |
|---|---|
| Construction | Both Vecs empty in `new_view` (and thus `new`) |
| Persistence | **None** — RAM only for the invocation |
| Invalidation API | Only via `emit_position_batch` replace-with-empty |
| Cross-tx | Impossible — Cache is per entrypoint |

SEED: Cache “buffers supply and debt event deltas.” Module doc: drained by `emit_position_batch`.

### 1.2 Record — append, never merge

```11:46:contracts/controller/src/context/events.rs
pub(crate) fn record_supply_position_update(...) {
    self.supply_updates.push_back(EventDepositDelta::new(...));
}
pub(crate) fn record_debt_position_update(...) {
    self.debt_updates.push_back(EventBorrowDelta::new(...));
}
```

| Property | Behavior |
|---|---|
| Keying | None — no `Map<HubAssetKey, _>` |
| Dedup | None |
| Amount fold | None — each call is a new row |
| Payload stamp | Post-leg `scaled_amount` + leg `amount` + `PositionAction` + index (+ supply risk tuple) |

There is **no** code path that looks up an existing buffer entry for the same hub and updates it in place.

### 1.3 Emit — batch coalesce + clear

```48:62:contracts/controller/src/context/events.rs
pub(crate) fn emit_position_batch(&mut self, account_id: u64, account: &Account) {
    if self.supply_updates.is_empty() && self.debt_updates.is_empty() {
        return;
    }
    UpdatePositionBatchEvent {
        account_id,
        account_attributes: account.into(),
        deposits: self.supply_updates.clone(),
        borrows: self.debt_updates.clone(),
    }
    .publish(&self.env);
    self.supply_updates = Vec::new(&self.env);
    self.debt_updates = Vec::new(&self.env);
}
```

| Property | Behavior |
|---|---|
| Empty | No-op (keeper always calls emit; no spurious event) |
| Multiplicity | One event with **all** buffered deposit legs + **all** buffered borrow legs |
| Clear | Both Vecs replaced with fresh empty Vecs after publish |
| Account attrs | Snapshot of `Account` **at emit time** (post merges / restamps) |
| Clone | Payload owns a clone; clearing does not mutate published data |

Contract comment on `UpdatePositionBatchEvent`: one operation may publish **more than one** batch (Credit liquidation) — key on `account_id`, never one-batch-per-tx.

### 1.4 Finalize order (A033 adjacency)

```241:252:contracts/controller/src/positions/mod.rs
cache.persist_spoke_usage();
persist_account_positions(...);
cache.emit_position_batch(account_id, account);
```

Durable spoke usage + position maps commit **before** events. Buffers are observational. Agrees A033 / A032 / A022–A025.

---

## 2. What each buffered row means (anti-coalesce contract)

### 2.1 Deposit delta (`EventDepositDelta`)

Wire: 10-tuple — `(action, hub_id, asset, scaled_amount, index_ray, amount, lt, bonus, ltv, fees)`.

| Field | Semantics for coalesce |
|---|---|
| `action` | Which verb/leg produced this row — **must not be discarded** if merging |
| `scaled_amount` | Account supply shares **after this leg** (absolute), not a delta |
| `amount` | This account’s **movement** for the leg (raw units) |
| Risk u32s | Stamped from the position object passed at `record_*` time |

### 2.2 Borrow delta (`EventBorrowDelta`)

Wire: 6-tuple — `(action, hub_id, asset, scaled_amount, index_ray, amount)`.

Same absolute-vs-movement split: `scaled_amount` is post-leg debt shares; `amount` is the leg’s debt movement.

### 2.3 Why keyed coalesce would be wrong

If `record_*` overwrote by hub and summed `amount`:

| Scenario | Broken observable |
|---|---|
| Migrate borrow then repay refund | Loses two-step history; net `amount` may look like “zero borrow” while intermediate risk existed |
| LiqSeize vs LiqCredit (different accounts) | Different accounts — but same Cache sequentially; clear separates them |
| FlashPos debt + Supply collateral | Different sides/actions; keyed map would still need two sides |
| RpColWd then CloseWd same collateral | Two supply legs; merge would hide partial withdraw vs close-out |

Keeping append-only preserves ADR-0019 / events.md fee reconstruction (`LiqSeize.amount − LiqCredit.amount` across **two** batches) and endpoints.md flash_position observer note (`FlashPos` + `Supply` tags in one batch).

---

## 3. Call-site matrix

### 3.1 Who records

| Site | Side | Typical `PositionAction` | Notes |
|---|---|---|---|
| `merge_supply_leg` | supply | `Supply` | Uses `action.amount` (caller/measured deposit input on that path) |
| `merge_withdraw_leg` | supply | `Withdraw`, `SwColWd`, `RpColWd`, `CloseWd`, `LiqSeize` (Transfer), `RpColNet`, … | Uses `outcome.amount` (pool) |
| `merge_debt_leg` | debt | `Borrow`, `Repay`, `LiqRepay`, `Multiply`, `FlashPos`, `SwDebtR`, `Migrate`, `RpColR`, `RpColNet`, … | Uses `outcome.amount` |
| Credit debit in `apply_liquidation_share_credit` | supply | `LiqSeize` | Direct record (not via merge_withdraw) |
| `record_share_credit_updates` | supply | `LiqCredit` | **After** victim emit cleared buffers |
| Keeper threshold update | supply | `ParamUpd` | `amount = 0`; no usage move |

### 3.2 Who emits

| Path | Emit via |
|---|---|
| Ordinary supply / withdraw / borrow / repay | `finalize_position_flow` |
| Strategies (multiply, swaps, repay-with-collateral, migrate, flash_position) | `strategy_finalize` → `finalize_position_flow` |
| Liquidation victim | `finalize_position_flow` after `LiquidationEvent` |
| Liquidation Credit receiver | Second `finalize_position_flow` after `record_share_credit_updates` |
| Keeper `update_account_threshold` | Direct `emit_position_batch` (after optional position persist) |
| Bad-debt cleanup | **No** `UpdatePositionBatchEvent` (A027 / events.md) |

### 3.3 Ordinary multi-asset: one delta per hub (because of payment coalesce)

Flow:

1. Entrypoint builds payment Vec (may contain duplicate hubs).
2. `aggregate_*` → **one** `(hub, summed_amount)` per hub.
3. Pool batch + `for_each_leg` / merge → **one** `record_*` per aggregated hub.
4. Single `emit_position_batch`.

So for `supply` / `borrow` / `repay` / `withdraw` / liquidation repayments, **buffer multiplicity equals unique hubs**, not raw Vec length. That is payment coalesce, not buffer coalesce. Confirmed by A062 + harness duplicate-payment tests.

### 3.4 Intentional multi-row / multi-tag batches (buffer stays append-only)

| Flow | Buffer contents (same emit) | Same hub twice? |
|---|---|---|
| Multi-asset liquidation | N `LiqRepay` + M `LiqSeize` | No per side (plan aggregates debt; seize from map) |
| Credit liquidation | Victim batch then receiver batch | Cross-batch only |
| `swap_debt` | Borrow `SwDebtR` + repay `SwDebtR` | **No** — assets must differ |
| `repay_debt_with_collateral` (cross asset) | `RpColWd` + `RpColR` | No (different hubs) |
| `RpColNet` | Supply `RpColNet` + debt `RpColNet` | **Same hub, different Vecs** |
| `close_position` after partial withdraw | `RpColWd`/`RpColNet` + per remaining asset `CloseWd` | **Yes on supply** if residual of withdrawn collateral remains |
| `migrate_from_blend` debt refund | `Migrate` borrow then `Migrate` repay | **Yes on debt** for refunded assets |
| `flash_position` | `FlashPos` debt + `Supply` deposits | Debt vs supply; collateral hubs distinct by validation |
| `multiply` | `Multiply` debt + `Supply` deposit(s) | Different sides |
| Keeper multi-asset ParamUpd | One `ParamUpd` per **changed** supply hub | No (loop continues on unchanged) |

These are the cases where a naive “coalesce by hub” indexer breaks.

---

## 4. Cross-account drain (Credit) — clear is load-bearing

```122:147:contracts/controller/src/positions/liquidation/mod.rs
finalize_position_flow(... victim ...);  // emit + clear
if let Some((receiver_id, receiving_account)) = &receiver {
    apply::record_share_credit_updates(...);  // fill supply_updates again
    finalize_position_flow(... receiver ...); // second batch
}
```

| Step | `supply_updates` / `debt_updates` |
|---|---|
| After repay + seize / share-credit debit | Victim LiqRepay + LiqSeize (+ Transfer withdraw path) |
| After victim finalize | **Empty** |
| After `record_share_credit_updates` | Only `LiqCredit` rows for receiver |
| After receiver finalize | **Empty** |

If emit did **not** clear, the receiver batch would republish victim legs under the wrong `account_id` — a severe indexer / reconstructor bug. Clear-after-publish defends that. Unit proof: `contracts/controller/tests/events.rs` asserts two batches, victim first, receiver deposits length 1, fee = seize − credit.

Agrees A026 / A052 / ADR-0019.

---

## 5. Security / correctness analysis

### 5.1 Can buffers affect money or caps?

**No.** Cap / usage / HF use account maps, spoke usage, pool mutation DTOs, and price/index Cache fields — not `supply_updates` / `debt_updates`. Emit is after persist. Panic after persist still reverts the whole Soroban tx (including events).

### 5.2 Can legs leak across accounts?

Only if a path buffered for account A then emitted with B’s id **without** clear between. Production Credit path clears via victim finalize first. No other multi-account mutator shares one Cache’s event buffers across ids without that pattern. Single-account strategies use one `account_id` end-to-end.

### 5.3 Reentrancy

Monetary reentry constructs a **new** `Cache::new` (fresh empty buffers). Inner finalize emits inner legs only. Outer buffers remain until outer finalize. Observational interleaving possible under listing-hook / flash-callback reentry (A007 class); storage still atomic per tx. Event buffers do not weaken flash guards.

### 5.4 Empty emit / no-change keeper

Keeper always calls `emit_position_batch`; if no ParamUpd was recorded, both Vecs empty → no-op. Correct.

### 5.5 Clone cost / DoS

Buffer length bounded by position limits and strategy structure (≤ few legs; migrate ≈ 2× debt assets with refunds; liquidation ≤ borrow slots + supply slots). Not an attacker-growable unbounded event queue beyond already-budgeted position work. No security finding.

### 5.6 Silent LTV restamp vs buffer contents (gap 2)

`strategy_finalize` / post-pool solvency may call `restamp_listed_supply_ltv`, mutating in-memory (then persisted) LTV **without** `record_supply_position_update(ParamUpd)`. Deposit deltas already in the buffer keep the risk tuple from merge time. For legs just merged with `FullTuple` refresh, listing config is unchanged mid-tx → same LTV. For **untouched** collateral slots, storage LTV can change with **no** delta in the batch.

| Class | Verdict |
|---|---|
| Coalesce / append correctness | Unaffected |
| Observability completeness | Partial — intentional ParamUpd is keeper path; silent restamp is strategy/solvency hygiene |
| Funds | Unaffected |

Owned as residual completeness note; not elevated.

### 5.7 Amount field asymmetry (supply entry)

`merge_supply_leg` records `action.amount`; withdraw/debt merges record `outcome.amount`. Peer A022 notes observational divergence if those ever disagree. Orthogonal to coalesce; buffers still append one row per leg.

---

## 6. Comparison table: coalesce policies in the controller

| Surface | Duplicate same hub | Policy |
|---|---|---|
| Payment batches (supply/borrow/repay/withdraw/liq repay) | Allowed in input | **Sum** (A062) |
| Flash collaterals / refunds | Rejected | Hard `InvalidPayments` |
| Migrate `debt_caps` | Rejected | `AssetsAreTheSame` |
| Event buffers | Multiple legs allowed | **Append** (this finding) |
| Event emission | Many legs | **One batch event** + clear |
| Spoke usage map | One row per hub/side | Keyed overwrite in Cache usage context (A076/A091 adjacency) |
| Account position maps | One entry per hub/side | Keyed update/remove |

Event buffers deliberately **do not** mirror position-map keyed semantics: events are a **journal of legs**; maps are **final state**.

---

## 7. Tests and docs evidence

| Evidence | What it shows |
|---|---|
| `tests/events.rs` Credit two-batch ordering / fee gap | Clear between accounts; distinct actions |
| `tests/events.rs` LiqRepay amount vs delivered | Borrow buffer carries measured movement |
| endpoints.md flash_position observer note | One batch may mix `FlashPos` + `Supply` |
| events.md “one entry per … leg”; multi-batch Credit | Append + multi-emit contract |
| endpoints.md §6 payment sum | Upstream coalesce, not buffer coalesce |
| A033 / A032 coordinator fills | Order + strategy write batching |
| Harness duplicate payment tests (A062) | Unique hubs into merges → unique event rows for those verbs |

**Missing (gap 3):** dedicated assert that migrate refund emits two `Migrate` borrow deltas for one hub, or that `close_position` after partial `RpColWd` emits two supply deltas for that hub. Would lock the anti-keyed-coalesce contract for indexers.

---

## 8. Peer cross-links

| Peer | Relation |
|---|---|
| **A033** | Event-after-persist order; buffers not SoT — **agree**; A092 adds append/batch/clear semantics |
| **A032** | Strategy single finalize drains one batch — **agree** |
| **A062** | Payment sum coalesce — complementary upstream |
| **A022–A025** | Per-verb record+finalize diagrams — **agree** |
| **A026 / A052** | Credit two-batch + clear — **agree**; A092 owns why clear matters for buffers |
| **A027** | Cleanup skips position batch — **agree** |
| **A086 / A104** | Inventory listed buffers; A104 marked A092 hole — **this filing closes it** |
| **A078** | Usage persist timing adjacent; event drain after usage persist |

No disagreement file warranted.

---

## 9. Residuals and remediation (optional)

| ID | Residual | Priority |
|---|---|---|
| R1 | Indexer docs: never keyed-merge deltas without action + absolute `scaled_amount` rules; last-wins for final shares; sum `amount` only with signed/action awareness | P3 docs |
| R2 | Unit/harness: migrate refund double `Migrate` debt delta; close-after-partial double supply delta | P3 tests |
| R3 | Document silent `restamp_listed_supply_ltv` vs `ParamUpd` event surface | P4 docs |
| R4 | Do **not** implement Map-keyed coalesce in `record_*` | Anti-remediation |

---

## 10. Verdict

**Event update buffers are defended.**

- **No** in-buffer coalesce/dedup by hub — correct for a leg journal.
- **Yes** emit coalesce — one `UpdatePositionBatchEvent` per drain, then clear.
- **Yes** upstream payment coalesce — ordinary verbs still get one row per hub from aggregated batches.
- Clear-after-emit is **required** for Credit’s second account batch.
- Not SoT; ordering after durable writes (A033).

Highest residual is **off-chain misinterpretation** of intentional same-hub multi-leg batches, not an on-chain safety hole. Severity **info**, status **defended**.
