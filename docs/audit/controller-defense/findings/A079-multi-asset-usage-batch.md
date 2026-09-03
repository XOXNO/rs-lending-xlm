# A079 — Multi-asset batch usage aggregation correctness

- Agent: A079
- Theme: T5
- Severity: info
- Status: defended
- Paths:
  - `contracts/controller/src/positions/mod.rs:70-141,148-188` (`for_each_leg`, `apply_leg_usage`, `merge_debt_leg`)
  - `contracts/controller/src/positions/supply.rs:126-155,192-301,308-417` (`settle_supply`, `apply_withdraw_batch`, `merge_supply_leg`, `merge_withdraw_leg`)
  - `contracts/controller/src/positions/debt.rs:128-212` (`settle_debt`, `apply_repay_batch`)
  - `contracts/controller/src/payments.rs:33-99` (`aggregate_payments` / `aggregate_positive_payments`)
  - `contracts/controller/src/spoke_usage.rs:61-160` (`SpokeUsageContext` map keyed by `HubAssetKey`)
  - `contracts/controller/src/context/spoke.rs:103-143` (`apply_spoke_entry` / `apply_spoke_exit` / `persist_spoke_usage`)
  - `contracts/pool/src/ops/mod.rs:49-73` (`run_batch` — order-preserving results)
  - Callers: liquidation `apply.rs` repay/seize batches; strategies via `process_deposit` / single-leg helpers; bad-debt per-key exits
- Defense: Multi-asset batches never fold usage into a single scalar. Each pool leg is zipped 1:1 via `for_each_leg` (length assert) and `apply_leg_usage` updates the **per-`HubAssetKey`** row in one in-memory `SpokeUsageContext`. User verbs and liquidation repayments **dedupe hubs before** building pool entries so each batch has at most one leg per hub (load-bearing for pool baselines and usage). Pool `run_batch` emits results in entry order. Caps are enforced per asset row at each entry; sides (`Supply` / `Borrow`) are independent fields on the same row.
- Gap: (1) `PoolPositionMutation` carries no `hub_asset`, so the controller cannot identity-check entry↔result beyond positional zip — trusts the pool contract. (2) `process_deposit` / `settle_*` do not re-aggregate; strategy callers must guarantee uniqueness (current callers do). (3) Explicit multi-asset **usage** Certora/harness rules are thinner than single-leg delta tracking (coverage → A085). Per-leg A080 missing-row exit no-op still applies inside multi-asset exit batches.
- Impact: A broken zip / shared-scalar / wrong-hub apply would mis-book spoke caps across markets (false rejects or over-admission up to per-asset cap headroom). Current design prevents that class on all inventoried production batch builders. No direct fund theft from aggregation alone.
- Evidence: INV-HALT-03; peers A076–A078, A080, A082, A084, A103 §7.1 provisional; unit `payments` aggregation tests; Certora `bulk_supply_two_assets_both_persisted` (positions); harness multi-asset verbs + `spoke_caps.rs`; pool `run_batch`.
- Opinion: A103’s provisional read is confirmed. Close A079 as **defended**; keep “one hub per batch leg” as a review checklist item for any new multi-leg builder that skips `aggregate_*`.

---

## 1. Scope and method

Mission: verify that when a controller flow mutates **several hub assets in one transaction**, spoke usage is aggregated correctly — each asset’s scaled delta lands on the right `(spoke, hub_asset, side)` row, without cross-asset summing, double-count, drop, or zip mis-attribution.

Method:

1. Read `shared/COORDINATION.md`, `SEED.md`, and peers A076 / A077 / A078 / A080 / A103 (A079 was coverage debt in A103 §7.1).
2. Trace every production path that builds a multi-leg pool batch or multi-asset usage loop into `apply_leg_usage` / `apply_spoke_*`.
3. Check pairing (`for_each_leg` vs pool result order), pre-batch uniqueness, per-row map semantics, and mid-batch cap visibility.
4. Distinguish intentional multi-persist / fee-only exits (A078 / A084) from aggregation bugs.

Out of scope as primary claims: persist-vs-pool timing (A078), missing-row exit (A080), supply-vs-borrow index choice (A081), Credit fee-only intent (A084).

No production Rust edited. No git operations.

---

## 2. Aggregation model (what “batch usage” means)

Spoke usage is **not** a single counter for the batch. It is:

```
SpokeUsageContext.usage: Map<HubAssetKey, SpokeUsageRaw>
  SpokeUsageRaw { supplied_scaled_ray, borrowed_scaled_ray }  // RAY shares
```

| Layer | Behavior |
|---|---|
| Key | Full `HubAssetKey` (`hub_id` + `asset`) |
| Side | `UsageSide::Supply` or `Borrow` — independent fields on the same row |
| Delta | `\|new_scaled − old_scaled\|` from pool `LegOutcome` via `apply_leg_usage` (A076 / A082) |
| Cap | `enforce_spoke_cap` on **that** row + side, using **that** leg’s returned index + decimals (A077) |
| Persist | One write of every touched row at finalize / bad-debt post-seize (A078) |

Correct multi-asset aggregation therefore means: **N legs → N independent row updates (possibly N distinct keys), never one summed occupancy across assets.**

---

## 3. Pairing: `for_each_leg` + pool `run_batch`

### Controller

```73:90:contracts/controller/src/positions/mod.rs
pub(crate) fn for_each_leg<E, R>(
    env: &Env,
    entries: &Vec<E>,
    results: &Vec<R>,
    mut f: impl FnMut(E, R),
) where
    ...
{
    assert_with_error!(
        env,
        results.len() == entries.len(),
        GenericError::InternalError
    );
    for (entry, result) in entries.iter().zip(results.iter()) {
        f(entry, result);
    }
}
```

Every merge site passes the **same** `entries` / `actions` vector that was sent to the pool, then reads `hub_asset` from the **entry** and scaled/index/decimals from the **result**:

| Batch builder | Merge | Hub source | Usage side / direction |
|---|---|---|---|
| `settle_supply` | `merge_supply_leg` | `entry.action.hub_asset` | Supply / Entry |
| `apply_withdraw_batch` | `merge_withdraw_leg` | `entry.action.hub_asset` | Supply / Exit |
| `settle_debt` borrow | `merge_debt_leg` | `entry.action.hub_asset` | Borrow / Entry |
| `apply_repay_batch` | `merge_debt_leg` | `entry.hub_asset` | Borrow / Exit |

`apply_leg_usage` always keys the spoke map with that hub:

```115:140:contracts/controller/src/positions/mod.rs
pub(crate) fn apply_leg_usage(..., hub_asset: &HubAssetKey, direction: LegDirection, old_scaled: Ray, outcome: &LegOutcome) {
    match direction {
        LegDirection::Entry { asset_decimals } => cache.apply_spoke_entry(..., hub_asset, outcome.new_scaled.checked_sub(env, old_scaled), ...),
        LegDirection::Exit => cache.apply_spoke_exit(..., hub_asset, old_scaled.checked_sub(env, outcome.new_scaled)),
    }
}
```

### Pool

```52:72:contracts/pool/src/ops/mod.rs
pub(crate) fn run_batch<E, R>(...) -> Vec<R> {
    ...
    for entry in entries.iter() {
        let (result, snapshot) = leg(env, &entry);
        results.push_back(result);
        ...
    }
    results
}
```

Results are **positional**: index `i` matches entry `i`. Length mismatch panics on the controller before any merge.

### Identity gap (residual, low)

`PoolPositionMutation` has `position`, `market_index`, `actual_amount`, `asset_decimals` — **no** `hub_asset`. The controller cannot assert “result belongs to this hub” beyond trusting the pool and the zip. Same-protocol trust boundary; not an aggregation logic bug. Defense-in-depth improvement would be embedding `hub_asset` on the mutation and asserting equality in `for_each_leg` / merge.

---

## 4. Uniqueness before the batch (load-bearing)

All pool entry vectors for a batch are built **before** the FFI, snapshotting each account position’s scaled amount into `PoolAction.position`. A second leg for the **same** hub in the same batch would therefore hand the pool a **stale baseline** for leg 2 (account map is only updated in the post-pool merge). Usage would also apply two deltas against evolving `old_scaled` after merges — numerically related to the wrong pool baselines.

So multi-asset correctness requires: **≤ 1 pool leg per `HubAssetKey` per batch.**

| Path | Uniqueness mechanism |
|---|---|
| `supply` / `borrow` / `repay` | `aggregate_positive_payments` — sum + first-seen order |
| `withdraw` | `aggregate_payments(..., MeansAll)` — sum / sticky withdraw-all sentinel |
| Liquidation repay | `calculate_repayment_amounts` → `aggregate_positive_payments` |
| Liquidation Transfer seize | `calculate_seized_collateral` iterates `supply_positions` map → one entry per hub |
| Liquidation Credit seize | Same seize list; fee exit per seize entry (unique hubs) |
| Bad debt | `iter_*_positions` map keys |
| `flash_position` deposits | `validate_collaterals` rejects repeated **asset** (stronger than hub key); `collect_collateral_deposits` one push per collateral |
| `migrate_blend` deposits | `push_unique_address` on withdraw list before deposit |
| `multiply` / `swap_collateral` | Single deposit hub |
| Strategy withdraw-all | `supply_positions.keys()` — unique |

`aggregate_payments` evidence (unit):

- Dedupes `(A,10)+(A,5)+(B,3)` → `[(A,15),(B,3)]` preserving first-seen order (`contracts/controller/tests/helpers/utils.rs`).
- Harness: `test_bulk_supply_duplicate_asset_counts_once` — duplicate USDC legs admit under `max_supply_positions = 1`.

**Footgun residual:** `process_deposit` → `settle_supply` does **not** re-call `aggregate_*`. It trusts the caller’s vector. Current production callers either aggregate (user verbs) or enforce uniqueness (strategies). A future caller that passes duplicate hubs into `process_deposit` would hit the stale-baseline class above. Checklist item, not a present bug.

---

## 5. Per-path batch walkthrough

### 5.1 Ordinary multi-asset verbs

```
aggregate_*  →  build entries[i] from current positions  →  pool_*_call
  → for_each_leg i: merge_* → apply_leg_usage(hub_i, Δscaled_i)  [RAM]
  → (solvency?) → finalize_position_flow → persist_spoke_usage (all touched rows)
```

- Supply A+B: two `apply_spoke_entry(Supply, …)` on distinct keys; each cap vs that asset’s config.
- Borrow A+B: two Borrow entries; same.
- Withdraw / repay multi-asset: Exit per hub; no cap consume (INV-HALT-03); A080 no-op if a row is missing for that hub.

Mid-batch: after merging asset A, the in-memory usage map holds A’s update before B’s `apply_entry`. Because hubs are unique in the batch, B never reads A’s row for its own cap. Independent caps — correct.

If B’s entry panics on cap after A’s pool leg succeeded: whole tx aborts; nothing durable (A078).

### 5.2 Liquidation (multi repay + multi seize)

1. `apply_liquidation_repayments` — measured transfers → `apply_repay_batch` → Borrow Exit usage per debt hub.
2. Transfer seize: `apply_withdraw_batch(Liquidation)` → Supply Exit per collateral hub.
3. Credit seize: account maps move shares; **only** `fee_scaled` `apply_spoke_exit(Supply)` per hub (A084); then fee seize pool call.
4. `finalize_position_flow` (victim; optionally receiver) persists the **same** Cache usage map.

Aggregation: repay hubs unique via `aggregate_positive_payments`; seize hubs unique via position map iteration. No double-apply of the same hub’s full scaled in one seize list.

### 5.3 Strategies (multi-leg over time, one Cache)

Strategies often alternate single-leg helpers (`borrow_into_controller`, `execute_withdrawal`, `execute_repayment`, `net_settle_*`) and occasional multi-asset `process_deposit` (flash_position, migrate_blend). All buffer into one `SpokeUsageContext` until `strategy_finalize` → one persist.

- Same asset touched twice across sequential steps (e.g. withdraw then later deposit): second `apply_*` sees the buffered row — correct cumulative usage.
- `net_settle`: one pool call → Supply Exit + Borrow Exit on the **same** hub (two sides) — both update the same usage row’s respective fields; not a cross-asset fold.
- `execute_withdraw_all`: sequential single-leg withdraws over unique keys.

### 5.4 Bad debt

Loop every supply key then every debt key → `apply_spoke_exit` full scaled → pool seize batch → `persist_spoke_usage`. Map iteration ⇒ unique hubs per side. Multi-asset cleanup drains each row independently (harness `test_closed_market_still_allows_bad_debt_cleanup` asserts both USDC supply and ETH borrow usage → 0).

---

## 6. Failure-mode matrix (aggregation-specific)

| Failure / mistake | Observed? | Effect if it were real |
|---|---|---|
| Zip length mismatch | Assert panics | No partial usage apply |
| Pool reorders results | Not in `run_batch` | Would mis-attribute scaled/index to wrong hub (catastrophic for caps + positions) — prevented by order-preserving batch |
| Sum all legs into one usage row | Not present | Cross-asset cap distortion |
| Skip `apply_leg_usage` for some legs | Not on inventoried merges | Under-count that hub only |
| Duplicate hub in one pool batch | Blocked by aggregate / uniqueness | Stale pool baseline + wrong usage (see §4) |
| Cap check uses another asset’s row | Impossible — keyed by `hub_asset` | — |
| Entry for A uses B’s `market_index` | Only if zip broken | Same as reorder |
| Mid-batch RAM then panic | Expected | Full abort; no durable skew (A078) |
| Missing usage row on multi-asset exit | A080 per leg | That hub’s exit no-ops; others still decrement |

---

## 7. Interaction with peers

| Peer | Relation to A079 |
|---|---|
| A076 | Per-leg entry/exit math; A079 shows those updates land on distinct map keys in a batch |
| A077 | Each leg’s own returned index for its cap — multi-asset does not share one index across hubs |
| A078 | One persist after all legs buffered — multi-asset batch is atomic with pool |
| A080 | Still the leading T5 residual; applies **per exit hub** inside a multi-asset withdraw/repay/seize |
| A082 | Deltas from pool scaled, not request amounts — holds per leg in the zip |
| A084 | Credit fee-only + strategy single finalize — not double-count of batch totals |
| A103 §7.1 | Provisional “per-asset row + length equality” — **confirmed** by this deep-dive |

No disagreement file required.

---

## 8. Evidence inventory

| Kind | What it covers |
|---|---|
| Unit | `aggregate_payments_dedups_and_preserves_order`; spoke `apply_entry_stores_single_add_not_dual_add` |
| Harness | Multi-asset supply/withdraw/borrow/repay; `spoke_caps` multi-market closed + bad-debt usage drain; bulk supply duplicate asset |
| Certora | `bulk_supply_two_assets_both_persisted` (positions both present); per-leg `usage_*_tracks_scaled_delta` / strategy / liq suites — **not** a dedicated “Σ usage after 2-asset supply equals ΔA+ΔB on two keys” rule |
| Pool | `run_batch` order + length parity with entries |

**Coverage residual (own with A085):** add an explicit multi-asset usage rule or harness that asserts after one `supply([(A,x),(B,y)])`, `get_spoke_usage(A).supplied` and `get_spoke_usage(B).supplied` each move by that leg’s scaled delta (and neither absorbs the other’s). Current evidence is compositional (pairing + map semantics + uniqueness) rather than a single end-to-end multi-key usage proof.

---

## 9. Verdict

**Defended.** Multi-asset batch usage aggregation is correct:

1. Aggregation is **per `HubAssetKey` row**, not a batch-wide scalar.
2. `for_each_leg` + pool `run_batch` preserve 1:1 positional pairing; hub identity for usage comes from the request entry.
3. Pre-batch **hub uniqueness** (aggregate or explicit) is the load-bearing guard that keeps pool baselines and usage deltas coherent.
4. Caps, indexes, and sides stay local to each leg’s asset; mid-batch RAM updates do not cross-contaminate distinct hubs.
5. Strategies and liquidations accumulate many legs into one Cache map and persist once (or idempotently) without double-counting deltas.

Residuals are **info / process**: positional trust without hub on `PoolPositionMutation`; `process_deposit` not re-aggregating (caller contract); thinner dedicated multi-asset usage proofs (A085). None overturn the defended status or introduce a novel medium beyond A080’s per-leg missing-row exit.
