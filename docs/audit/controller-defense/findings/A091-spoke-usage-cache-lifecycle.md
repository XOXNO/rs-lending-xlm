# A091 — `spoke_usage` embedded in Cache lifecycle (ensure / reset / persist)

- Agent: A091
- Theme: T7 (with T5 adjacency)
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/context/mod.rs:25–62` (`Cache` fields; `new` / `new_view` leave `spoke_usage: None`)
  - `contracts/controller/src/context/spoke.rs:12–143` (`ensure_spoke_context`, `reset_spoke_context`, `require_spoke_usage_context`, `apply_spoke_{entry,exit}`, `persist_spoke_usage`)
  - `contracts/controller/src/spoke_usage.rs:61–141` (`SpokeUsageContext::{new,persist,load_usage_row,apply_entry,apply_exit}`)
  - `contracts/controller/src/storage/spoke.rs:56–78` (`get_spoke_usage` / `set_spoke_usage` prune-on-zero)
  - `contracts/controller/src/positions/mod.rs:112–141,238–252` (`apply_leg_usage`, `finalize_position_flow`)
  - `contracts/controller/src/positions/liquidation/{mod,apply,bad_debt}.rs` (Credit double-finalize; bad-debt direct persist; no reset)
  - `contracts/controller/src/strategies/mod.rs:68–79` (`strategy_finalize` → single finalize)
  - `contracts/controller/src/keepers.rs:86–91` (**sole** production `reset_spoke_context`)
- Defense: Per-invocation `Cache` holds at most one `SpokeUsageContext`, created lazily by `ensure_spoke_context` and pinned to a single `spoke_id` (`SpokeMismatch` #310 on conflict). Usage rows are lazy-loaded into an in-memory `Map` only on `apply_{entry,exit}`; `persist_spoke_usage` writes every buffered row under that pinned id (or no-ops if context was never opened). Ordinary / strategy / liquidation account tails durability through `finalize_position_flow` after pool success; bad-debt calls `persist_spoke_usage` after `pool_seize_positions_call`. The only production reset is keeper threshold refresh, which never buffers usage. Multi-account Credit liquidation and post-liq bad debt intentionally reuse one Cache on one spoke and re-persist the same map (idempotent amplification).
- Gap: (1) **Engineering footgun** — `reset_spoke_context` drops unpersisted buffered usage; safe today because the sole caller never dirties usage, but any future mid-flow reset before persist would under-count occupancy (capacity distortion, same direction as A080). (2) **No dedicated unit test** that `ensure(spoke_a)` then `ensure(spoke_b)` panics without reset, nor that reset-before-persist loses deltas (A083 residual). (3) First listing/config touch pins an empty usage context even when no apply runs — harmless (`persist` iterates empty), but couples “I only wanted assets” to the usage Option. (4) Semantic missing-row exit no-op remains A080; lifecycle does not create that bug.
- Impact: Under current call sites, successful txs commit spoke usage consistent with the pooled deltas that produced them; failed txs roll back. Wrong-spoke attribution via Cache is fail-closed. Blast radius of the reset-before-persist footgun would be spoke-wide false headroom (over-admission until reconcile), not direct fund theft. No novel Critical/High from lifecycle embedding alone.
- Evidence: INV-HALT-03, INV-STOR-01 (lifecycle discipline adjacency), ADR-0009 / ADR-0015; peers A076, A078, A079, A080, A083, A084, A086, A103, A104; unit `contracts/controller/tests/spoke.rs` (`exit_sees_entry_cached_row_in_same_context`, `spoke_usage_context_preserves_spoke_id`, entry/exit persist); harness `spoke_caps.rs`, keeper mixed-spoke batch; Certora usage_* / A078 inventory.
- Opinion: Lifecycle embedding is **sound and defended**. Treat `ensure`/`reset`/`persist` as a three-state discipline: pin → buffer → durable write; reset only after persist (or when buffer is known empty). Keep `finalize_position_flow` as the ordinary durability chokepoint; keep keeper reset usage-free. Do not add mid-liquidation reset. Close residual with a Cache-pin panic unit and a static/comment gate against reset between apply and persist.

---

## 0. Scope and method

### 0.1 Mission

Prove or refute that embedding `spoke_usage: Option<SpokeUsageContext>` inside per-invocation `Cache` preserves spoke-cap occupancy across the full lifecycle:

1. **ensure** — lazy create / pin
2. **mutate** — RAM-only `apply_entry` / `apply_exit`
3. **persist** — durable `SpokeUsage` keys
4. **reset** — clear pin so another spoke (or a fresh same-spoke reload) can be loaded

Out of scope as primary claims: cap index selection (A077/A081), Credit fee-only intent (A084), missing-row exit semantics (A080), storage key domain (A028), event-vs-storage order beyond noting usage is first in finalize (A033). Those are cited only where they intersect lifecycle.

### 0.2 Method

1. Read COORDINATION (findings-only; no git) + AGENT_MANIFEST A091 + README format.
2. Read `Cache` constructors and every `context/spoke.rs` lifecycle API.
3. Inventory production callers of `ensure_spoke_context` (transitively), `reset_spoke_context`, `persist_spoke_usage`.
4. Trace ordinary verbs, strategies, liquidation (victim / Credit receiver / bad debt), and keeper threshold batch.
5. Cross-check peers A076–A086, A103/A104 adjacency for A091 “coverage hole”.
6. No production Rust edited.

---

## 1. Executive verdict

**`spoke_usage` in Cache is a coherent write-behind buffer with a hard one-spoke pin, not a silent stale-memo hazard.**

| Lifecycle phase | Property | Verdict |
|---|---|---|
| Construct (`new` / `new_view`) | `spoke_usage = None` | Defended |
| Ensure | Empty context + pin; mismatch → `#310` | Defended |
| Apply | Lazy row load; RAM map only; cap on entry | Defended (semantics A076; timing A078) |
| Persist | All map rows under pinned `spoke_id`; no-op if `None` | Defended |
| Reset | Clears usage + config + assets together | Defended at sole call site; footgun if misused |
| Multi-finalize same Cache | Same spoke; re-persist after further applies | Defended (intentional) |

Highest residual is **operational**: reset-before-persist would discard deltas. Not present in production graphs today (A078/A083 agree).

---

## 2. State machine (what “embedded” means)

```
Cache.spoke_usage: Option<SpokeUsageContext>
         │
         ▼
      None  ──ensure(spoke_id)──►  Some(ctx { spoke_id, usage: Map empty })
         ▲                              │
         │                              │ apply_entry / apply_exit
         │                              │   (lazy get_spoke_usage → Map row)
         │                              ▼
         │                         Some(ctx { … usage dirty/touched })
         │                              │
         │                    persist_spoke_usage()
         │                              │  for each Map row:
         │                              │    set_spoke_usage(spoke_id, hub, row)
         │                              ▼
         │                         Some(ctx) still held  ← RAM not cleared by persist
         │                              │
         └────── reset_spoke_context ───┘
                    usage=None; config=None; spoke_assets=empty Map
```

Critical embedding facts:

1. **Persist does not clear the Option.** After `finalize_position_flow`, the same in-memory map remains. A later `apply_*` continues from RAM (or loads missing hubs from storage). A later `persist` rewrites the same keys (idempotent for unchanged rows).
2. **Only reset (or end of Cache lifetime) drops the buffer.** There is no “dirty bit” separate from Map membership.
3. **Usage map is not a read-through memo for views of all hubs.** Rows enter the map only via `apply_entry` / `apply_exit` → `load_usage_row`. Listing/config use sibling fields (`spoke_config`, `spoke_assets`) that share the pin but not the usage Map.
4. **Empty `Some(ctx)` is possible.** Any first `cached_spoke_asset` / `spoke_config` call ensures a context with an empty usage Map. `persist_spoke_usage` then iterates zero keys → **no storage writes**. Harmless, but means “usage context loaded” ≠ “usage mutated”.

---

## 3. Ensure — pin and lazy create

```12:22:contracts/controller/src/context/spoke.rs
pub(crate) fn ensure_spoke_context(&mut self, spoke_id: u32) {
    if let Some(ctx) = &self.spoke_usage {
        assert_with_error!(
            &self.env,
            ctx.spoke_id() == spoke_id,
            SpokeError::SpokeMismatch
        );
        return;
    }
    self.spoke_usage = Some(SpokeUsageContext::new(&self.env, spoke_id));
}
```

```67:75:contracts/controller/src/spoke_usage.rs
pub(crate) fn new(env: &Env, spoke_id: u32) -> Self {
    Self {
        env: env.clone(),
        spoke_id,
        usage: Map::new(env),
    }
}
```

### 3.1 Properties

| # | Property | Implication |
|---|---|---|
| E1 | First ensure creates empty RAM buffer | No eager load of all hubs (gas); no stale bulk snapshot |
| E2 | Second ensure same id is idempotent | Multi-leg / multi-asset flows share one buffer (A079) |
| E3 | Second ensure different id panics `#310` | Fail-closed cross-spoke isolation (A083) |
| E4 | `require_spoke_usage_context` ensure then `as_mut().unwrap_or InternalError` | After ensure, Option is Some; `#34` only if invariant broken |
| E5 | Config/asset accessors also call ensure | Pin is shared across usage + listing memos |

### 3.2 Callers that open the pin (transitive)

Everything spoke-scoped goes through `ensure_spoke_context`:

| Accessor | Opens usage Option? | Touches usage Map? |
|---|---|---|
| `require_spoke_usage_context` | Yes | Yes (apply) |
| `apply_spoke_entry` / `apply_spoke_exit` | Yes | Yes |
| `cached_spoke_asset` / `require_spoke_asset*` / `require_listed_active_config` | Yes | No |
| `spoke_config` / `active_spoke` | Yes | No |

`apply_spoke_entry` loads **cap from `require_spoke_asset_config(spoke_id, …)`** then mutates **`require_spoke_usage_context(spoke_id)`** — same `spoke_id` argument, both ensure-pinned. Cap config and occupancy cannot silently come from different spokes in one Cache (A083).

### 3.3 Constructors

`Cache::new` renews instance TTL then `new_view`. Both set `spoke_usage: None`. Views that only read risk/config may ensure a pin via `spoke_config` / assets without ever calling `persist_spoke_usage` — correct (no durable side effect from ensure alone).

---

## 4. Reset — clear pin; sole production caller

```24:29:contracts/controller/src/context/spoke.rs
pub(crate) fn reset_spoke_context(&mut self) {
    self.spoke_usage = None;
    self.spoke_config = None;
    self.spoke_assets = Map::new(&self.env);
}
```

### 4.1 What reset clears

| Field | Cleared? | Why together |
|---|---|---|
| `spoke_usage` | Yes | Drop pin + any unpersisted rows |
| `spoke_config` | Yes | Untagged `Option<SpokeConfig>` — wrong without pin |
| `spoke_assets` | Yes | Map keyed by `HubAssetKey` **without** spoke id — correctness depends on pin |

There is **no** API that clears only usage and leaves a stale config pin, or vice versa. That coupling is load-bearing for A089 adjacency.

### 4.2 Production inventory

`rg` over `contracts/controller/src`: **one** call site.

```86:91:contracts/controller/src/keepers.rs
let mut cache = Cache::new(env);
for account_id in account_ids {
    cache.reset_spoke_context();
    sync_account_thresholds(env, account_id, scope, &mut cache);
}
```

`sync_account_thresholds` uses `cached_spoke_asset(account.spoke_id, …)` and risk restamp; it does **not** call `apply_spoke_*` or `persist_spoke_usage`.

| Why reset is required | Why reset is safe |
|---|---|
| Batch may mix spokes (`test_update_account_threshold_mixed_spokes_batch`) | No buffered usage to drop |
| Without reset: second spoke → `SpokeMismatch`, or weaker pin would serve wrong `SpokeAssetConfig` | Opposite of A078 mid-flow footgun |

Liquidation, strategies, supply/borrow/repay/withdraw, flash_position: **never** call reset.

### 4.3 Footgun formalization (not a present bug)

Unsafe sequence:

1. `apply_spoke_*` (RAM dirty for spoke A)
2. `reset_spoke_context()` **without** `persist_spoke_usage`
3. Later ensure/persist for A or B

Effect: A’s deltas discarded → durable occupancy **under-counts** relative to live positions → **false headroom** / over-admission until reconcile (capacity distortion; same *direction* as A080 missing-row under-count, different mechanism).

Safe sequences:

- Reset when buffer empty (keeper today).
- Persist then reset (would reload from storage on next apply — correct for continuing mutations).
- Never reset mid ordinary/strategy/liq flow (today’s code).

**After successful persist, reset is still correct** for subsequent applies: storage already holds truth; new context lazy-loads on next exit/entry.

---

## 5. Persist — durability chokepoint

### 5.1 Cache wrapper

```138:143:contracts/controller/src/context/spoke.rs
pub(crate) fn persist_spoke_usage(&self) {
    if let Some(ctx) = &self.spoke_usage {
        ctx.persist();
    }
}
```

```77:82:contracts/controller/src/spoke_usage.rs
pub(crate) fn persist(&self) {
    for (hub_asset, usage) in self.usage.iter() {
        storage::set_spoke_usage(&self.env, self.spoke_id, &hub_asset, &usage);
    }
}
```

| Property | Detail |
|---|---|
| P1 | No-op if never ensured | Views / paths with no spoke touch write nothing |
| P2 | Writes **every** Map row | Multi-asset batch atomic at finalize (A079) |
| P3 | Bound to `ctx.spoke_id` | No persist-time override (A083) |
| P4 | Zero both sides → key remove | `set_spoke_usage` prune |
| P5 | Does not clear Option/Map | Enables Credit double-finalize + bad-debt third write |

### 5.2 Production persist call sites (complete)

| Site | When | Preceding pool? |
|---|---|---|
| `finalize_position_flow` → `cache.persist_spoke_usage()` | Tail of supply / withdraw / borrow / repay / strategies / liquidation account batches | Yes — legs already returned (or empty context → no-op) |
| `execute_bad_debt_cleanup` → `cache.persist_spoke_usage()` | After `pool_seize_positions_call` | Yes — seize completed; exits buffered before call |

No other production writer of `SpokeUsage` keys (A078). Certora/unit tests may call `persist` directly.

### 5.3 Ordinary ordering (lifecycle view)

```
Cache::new
  → (gates / measured transfers)
  → pool_*_call SUCCESS
  → merge_*_leg → apply_leg_usage → ensure + apply_{entry|exit}   [RAM]
  → post-pool solvency? (borrow/withdraw/strategies)
  → finalize_position_flow
       1. persist_spoke_usage     ← first durable usage write
       2. persist_account_positions
       3. emit_position_batch
```

Cap failure after pool / solvency failure after apply: Soroban aborts → no durable usage (A078). Lifecycle embedding does not reorder that.

### 5.4 Strategy path

One `Cache` per strategy entry; all legs accumulate into one usage Map; `strategy_finalize` → one `finalize_position_flow`. No reset. Aligns with multi-asset batching (A079).

---

## 6. Multi-account / multi-persist on one Cache (liquidation)

Credit liquidation is the stress case for embedded lifecycle:

```
Cache::new
  resolve_seize_receiver (may ensure via create_account / require listing)   [same spoke]
  build plan / repay batch → apply_spoke_exit (debt)                        [RAM]
  Credit seize → fee apply_spoke_exit (supply) + pool seize fees            [RAM; pool after fee buffer]
  finalize_position_flow(victim)   → persist #1
  record_share_credit_updates + finalize_position_flow(receiver) → persist #2
  check_bad_debt → execute_bad_debt_cleanup
       apply_spoke_exit remaining positions                                 [RAM continues]
       pool_seize_positions_call
       persist_spoke_usage                                                  → persist #3
```

### 6.1 Why reuse is correct

- Usage is **spoke-scoped**, not account-scoped. Victim debit + receiver credit cancel except fee (A084); both accounts share `account.spoke_id` (asserted).
- Persist #1 writes fee exits + repay exits already buffered.
- Persist #2 typically rewrites the **same** Map (receiver finalize does not re-apply usage for share credit — fee already applied). Amplification, not double-count of deltas.
- Bad debt continues on the **same** Option: further exits decrement RAM that already matches storage after #1/#2, then #3 writes the residual. If cleanup were on a fresh Cache, exits would `load_usage_row` from storage — also correct **because** prior finalize already persisted. Sharing Cache is an optimization, not a correctness requirement after #1.

### 6.2 What would break

| Hypothetical | Effect |
|---|---|
| `reset_spoke_context` after fee apply, before victim finalize | Fee/repay exits dropped → overstated occupancy |
| `reset` between victim finalize and bad debt | Safe *if* finalize already persisted (reload from storage); still needless |
| Cross-spoke Credit receiver | Already rejected `SpokeMismatch` before usage moves |

No production reset on this path (A083 hazard matrix row covered).

Transfer seize uses withdraw-batch merge → normal leg usage into same Cache → single victim finalize (plus optional bad debt). Same lifecycle rules.

---

## 7. Coupling: ensure without mutate; persist without clear

### 7.1 Empty context pinned by listing reads

Keeper threshold refresh, liquidation planning, risk totals, and gate helpers often call `cached_spoke_asset` / `spoke_config` first. That sets `spoke_usage = Some(empty)`. Later applies fill the Map. If a path ensures but never applies and somehow called persist, storage is untouched — **safe**.

### 7.2 Persist leaves RAM hot

Design choice: persist is write-through of the Map, not consume-and-clear. Consequences:

| Positive | Negative / discipline |
|---|---|
| Multi-finalize without reload | Reviewers must not assume “persist ⇒ clean slate” |
| Bad-debt can continue mutating | Reset-before-persist remains the sharp edge |
| Idempotent re-write of unchanged hubs | Extra storage writes on double finalize (fee amplification; A078 residual) |

### 7.3 Interaction with A080

Missing-row `apply_exit` returns without inserting a Map row → persist cannot invent a zero key for that hub. Lifecycle correctly preserves “no row” rather than materializing zeros (unit `exit_without_usage_row_is_noop_and_does_not_persist`). Embedding does not amplify A080; it also does not heal under-recorded usage.

Same-context entry then exit without intermediate persist hits the Map (`exit_sees_entry_cached_row_in_same_context`) — lifecycle buffering works as designed.

---

## 8. Hazard matrix (lifecycle-specific)

| # | Scenario | Outcome today | Severity if introduced |
|---|---|---|---|
| H1 | Two spokes ensure without reset | `#310` panic | N/A (fail closed) |
| H2 | Reset before persist after apply | Not in prod; would drop deltas | Medium capacity (governance limit) |
| H3 | Persist before pool success | Not in prod (A078) | High desync if introduced |
| H4 | Credit double finalize | Idempotent rewrite | Info amplification |
| H5 | Bad debt third persist on shared Cache | Correct residual write | Info |
| H6 | Ensure-only then persist | Empty iter / no write | None |
| H7 | Keeper reset mid batch | Clears config/assets; no usage dirty | Defended |
| H8 | View `new_view` ensure, no persist | No durable usage | None |
| H9 | Strategy many legs one finalize | Single persist of full Map | Defended |
| H10 | Direct `set_spoke_usage` from entrypoint | Not exposed | Would forge occupancy |

---

## 9. Agreement with peers

| Peer | Claim relevant to A091 | A091 stance |
|---|---|---|
| A076 | One spoke per invocation; persist all touched rows | **Agree** — lifecycle implements that |
| A078 | Persist after pool; reset mid-flow footgun | **Agree** — expand call-site map |
| A079 | Multi-asset Map then one persist | **Agree** |
| A080 | Exit missing row no-op | **Agree** — orthogonal; not caused by ensure/reset |
| A083 | Pin + sole reset + writer provenance | **Agree** — A091 owns full lifecycle narrative |
| A084 | Credit fee-only; no mid-liq reset | **Agree** |
| A086 | Field inventory: pin via ensure, clear via reset | **Agree** — deepen usage-specific state machine |
| A103 / A104 | A091 listed as coverage hole | **This filing closes** the hole for ensure/reset/persist |

No disagreement filed. Residual shared with A083/A078: missing pin/reset regression tests.

---

## 10. Evidence and coverage gaps

### 10.1 Present evidence

| Evidence | What it shows |
|---|---|
| Unit `spoke_usage_context_preserves_spoke_id` | Context remembers id |
| Unit `exit_sees_entry_cached_row_in_same_context` | Buffer across apply without persist |
| Unit entry/exit persist + prune | Durable write / zero remove |
| Unit listing without usage | Config path independent of Map rows |
| Harness spoke caps | Cap enforcement end-to-end |
| Harness keeper mixed spokes | Reset required for multi-spoke batch |
| Certora usage_* (via A078/A085) | Scaled delta tracking on verbs |
| Source inventory | Exactly two persist sites; one reset site |

### 10.2 Missing tests (residuals)

| Gap | Suggested artifact |
|---|---|
| `ensure(1)` then `ensure(2)` panics `#310` without reset | Unit on `Cache` |
| Apply then reset then persist does not write first deltas | Unit documenting footgun |
| Static gate: no `reset_spoke_context` between `apply_spoke_*` and `persist_spoke_usage` | Comment-test / script (A085/A108 adjacency) |
| Liquidation Credit: assert usage after victim finalize equals fee-only delta before receiver finalize | Harness (A084/A085) |

---

## 11. Remediation / hygiene (non-blocking)

1. **Document** in `context/spoke.rs` module or `ensure`/`reset` docs: “reset drops unpersisted usage; call only when buffer empty or after persist.”
2. **Add** Cache-pin mismatch unit + reset-before-persist negative unit.
3. **Keep** `finalize_position_flow` as sole ordinary durability path; do not move persist earlier to avoid post-cap revert cost (A078 opinion).
4. **Reject** PRs that insert `reset_spoke_context` into liquidation/strategy/position flows without an accompanying persist-before-reset and multi-spoke justification.
5. **Do not** clear usage Map inside `persist` without auditing Credit double-finalize and bad-debt continuation — current “persist leaves hot” is load-bearing.

---

## 12. Final judgment

**Defended.** Embedding `SpokeUsageContext` in `Cache` correctly implements a per-tx, single-spoke write-behind buffer: lazy ensure, fail-closed pin, RAM apply, durable persist after pool success, and a sole usage-free reset for multi-spoke keeper batches. Multi-finalize liquidation reuses the hot buffer intentionally. Residuals are engineering discipline and test coverage around the reset-before-persist footgun — not a live fund-loss or silent wrong-spoke write under the current call graph.

Closes A104’s A091 coverage hole for pin/reset/multi-account usage-buffer mapping; defers capacity-desync remediation to A080 / reconcile tooling.
