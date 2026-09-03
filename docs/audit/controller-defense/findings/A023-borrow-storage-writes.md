# A023 — Borrow path storage writes (`process_borrow` / `merge_debt_leg` / finalize)

- Agent: A023
- Theme: T2
- Severity: info
- Status: defended
- Paths: `contracts/controller/src/lib.rs:112-121`; `contracts/controller/src/positions/debt.rs:35-76,128-160`; `contracts/controller/src/positions/mod.rs:52-67,73-90,96-104,112-188,206-252,275-287,306-327`; `contracts/controller/src/account.rs:209-221`; `contracts/controller/src/risk/params.rs:37-60`; `contracts/controller/src/risk/validation.rs:12-61`; `contracts/controller/src/context/mod.rs:39-44`; `contracts/controller/src/context/spoke.rs:103-143`; `contracts/controller/src/context/market_index.rs:12-15`; `contracts/controller/src/context/events.rs:29-62`; `contracts/controller/src/spoke_usage.rs:61-160`; `contracts/controller/src/storage/account.rs:71-103,247-270`; `contracts/controller/src/storage/spoke.rs:65-79`; `contracts/controller/src/external/pool.rs:33-40`; `common/src/types/controller.rs:346-355`
- Defense: Borrow mutates durable controller state only at `finalize_position_flow` after pool settle + post-pool solvency. In-leg work is buffered in `Account` + `Cache` (`merge_debt_leg`). Debt map always persists; supply map persists iff LTV restamp changed any listed collateral (`restamped → PositionSides::Both`). Spoke usage and position maps commit before the batch event. Instance + account TTLs renew on the path.
- Gap: none on ordinary `process_borrow` durable accounting. Residual (cross-ref A007): plain borrow does not set `with_flash_guard` around `pool_borrow_call`; a governance-listed token with a transfer hook could re-enter monetary entrypoints against still-unpersisted in-memory state. Listing trust + tx atomicity bound that residual. `put_market_index` is Cache-only (pool remains SoT) — by design (A094/A038).
- Impact: A broken `restamped → Both` coupling would leave solvency-gated LTV in memory while disk kept the pre-cut LTV, so later loads would overstate collateral vs the gate observation (Certora `assert_gate_observation_is_final` / TOB-AAVE-7 class). Current code + harness regressions close that. Failed solvency or cap checks revert the whole tx (pool + controller). Successful borrow cannot orphan empty account maps (`remove_if_empty: false` is correct — borrow cannot empty).
- Evidence: INV-AUTH-02, INV-RISK-01, INV-RISK-04, INV-STOR-01, INV-HALT-02 (`BlockOnEntry`); STRIDE Tamper.5 residual; Certora `controller_borrow_persists_pool_returned_position`, `post_gate_borrow_totals_are_final`, spoke-usage borrow rules; harness `security_audit.rs` (`regression_borrow_restamps_ltv_*`); unit/harness borrow position-limit tests. Agrees with A033 (persist-before-emit), A072 (post-pool gate), A076/A077/A082 (usage from pool outcomes), A032 (buffer-then-finalize pattern), A003 (owner-or-delegate before money move), A040 (listed hubs only).
- Opinion: Borrow’s storage story is sound and deliberately asymmetric with withdraw: withdraw already writes `PositionSides::Supply` so in-memory LTV restamps land on disk “for free”; borrow must widen to `Both` when restamp mutates supply, and it does. Do not “optimize” that branch away.

## Scope and method

1. Read `shared/COORDINATION.md` + `SEED.md` (no git ops; findings-only).
2. Trace `Controller::borrow` → `process_borrow` → `settle_debt(Borrow)` → `merge_debt_leg` → `enforce_post_pool_solvency` → `finalize_position_flow` / `persist_account_positions`.
3. Enumerate every durable write (`persistent` / instance TTL / temporary flash flag) reachable on that path; separate Cache buffers from storage keys.
4. Cross-check peer findings A003, A007, A032, A033, A036, A040, A072, A076, A077, A082, A094 and invariants INV-RISK-01 / INV-STOR-01.
5. Related helper `borrow_into_controller` uses `merge_debt_leg` but **not** this finalize — strategy callers own persistence (A032); noted only as a shared merge primitive.

Out of scope for A023: repay (`process_repay`), pool-internal cash books, strategy finalize batching beyond the shared tail.

---

## Call graph (ordinary borrow)

```
Controller::borrow                          #[when_not_paused]  lib.rs:112-121
  └─ process_borrow                         debt.rs:35-76
       ├─ require_authorized_caller         auth + !flash_loaning
       ├─ storage::get_account              load meta + supply + debt maps
       ├─ require_owner_or_delegate         INV-AUTH-02
       ├─ Cache::new                        renews controller instance TTL
       ├─ require_external_recipient        reject pool / controller as `to`
       ├─ aggregate_positive_payments       dedupe hubs; reject ≤0 amounts
       ├─ validate_position_entry_gates     limits + require_can_borrow
       ├─ settle_debt(DebtFlowKind::Borrow) debt.rs:138-160
       │    ├─ get_or_create_debt_position  read-only seed (no map insert)
       │    ├─ pool_borrow_call             pool durable writes (other contract)
       │    └─ for_each_leg → merge_debt_leg
       │         ├─ apply_leg_usage Entry   Cache spoke-usage buffer + cap
       │         ├─ put_market_index        Cache only
       │         ├─ record_debt_position_update  event buffer
       │         └─ update_or_remove_debt_position  Account RAM only
       ├─ enforce_post_pool_solvency
       │    ├─ restamp_listed_supply_ltv    may mutate supply map in RAM
       │    └─ require_post_pool_risk_gates INV-RISK-01
       └─ finalize_position_flow(..., sides, remove_if_empty=false)
            ├─ persist_spoke_usage          SpokeUsage keys
            ├─ persist_account_positions
            │    ├─ set_supply_positions    iff sides ≠ Debt
            │    ├─ set_debt_positions      iff sides ≠ Supply  (always here)
            │    ├─ renew_user_account      TTL on existing account keys
            │    └─ cleanup skipped         remove_if_empty=false
            └─ emit_position_batch          after durable writes (A033)
```

`sides = Both` if restamp changed any listed supply LTV; else `Debt` only (`debt.rs:69-75`).

---

## Durable writes inventory

| Phase | Storage class | Key / target | Writer | Notes |
|---|---|---|---|---|
| Entry | Instance | controller instance TTL | `Cache::new` → `renew_controller_instance` | Every mutating path; INV-STOR-01 |
| Mid (pool FFI) | Pool contract | pool market/user books | `pool_borrow_call` | Trusted sibling; rolls back with tx on later panic |
| Mid (controller) | — | none | — | `merge_debt_leg` is **not** a durable write |
| Finalize | Persistent shared | `ControllerKey::SpokeUsage(spoke_id, hub)` | `SpokeUsageContext::persist` → `set_spoke_usage` | Removes key if both scaled sides are 0 |
| Finalize | Persistent user | `BorrowPositions(account_id)` | `set_debt_positions` → `write_side_map` | Full map snapshot; remove if empty map |
| Finalize (conditional) | Persistent user | `SupplyPositions(account_id)` | `set_supply_positions` | Only when `PositionSides::Both` |
| Finalize | Persistent user TTL | AccountMeta / Supply / Borrow / Delegates if present | `renew_user_account` | Extends TTL; no value rewrite |
| Finalize | Events | `UpdatePositionBatchEvent` | `emit_position_batch` | After persists; observational |
| Never on this path | Persistent user | `AccountMeta`, `Delegates` | — | Spoke/mode/delegates unchanged |
| Never durable | Cache maps | `market_indexes`, event vecs, spoke_assets, … | `put_market_index`, buffers | Dropped at end of invocation |

Temporary `FlashLoanOngoing` is **not** set on ordinary borrow (only on `borrow_into_controller` / strategy windows — A007).

---

## 1. Pre-write gates (no storage mutation)

1. **Pause** — `#[when_not_paused]` on `borrow` (risk-increasing).
2. **Auth** — `require_authorized_caller` (`caller.require_auth` + flash-loan gate).
3. **Account authority** — `require_owner_or_delegate` before any pool call (A003 / INV-AUTH-02).
4. **Recipient** — `require_external_recipient` blocks pool and controller addresses (stranded funds / false balance deltas).
5. **Inputs** — `aggregate_positive_payments` rejects empty/non-positive legs and collapses duplicate hubs so each hub appears once in the pool batch.
6. **Listing / halt / borrowable** — `validate_position_entry_gates` → `require_can_borrow` → hub active, listed on account spoke, `BlockOnEntry` (not paused/frozen), `is_borrowable` (A040).
7. **Position count** — `validate_bulk_position_limits` (INV-RISK-04).

No `set_*` / `persist_*` runs in this phase. `Cache::new` only renews instance TTL.

---

## 2. `settle_debt` / Borrow — pool then in-memory merge

### 2.1 Position seeding

`get_or_create_debt_position` (`common/.../controller.rs:348-355`) returns the existing debt or a **zero** `DebtPosition` without inserting into `account.borrow_positions`. The map is updated only later in `update_or_remove_debt_position`. Opening a new hub therefore does not leave a zero-share durable row if the tx later reverts.

Pool actions are built from that snapshot **before** the FFI; all legs see pre-borrow scaled amounts. After the single `pool_borrow_call`, `for_each_leg` asserts `entries.len() == results.len()` then merges in order against the evolving in-memory map (safe because aggregation deduped hubs).

### 2.2 `merge_debt_leg` (Entry) — buffer only

```148:188:contracts/controller/src/positions/mod.rs
pub(crate) fn merge_debt_leg(
    env: &Env,
    account: &mut Account,
    action: events::PositionAction,
    hub_asset: &HubAssetKey,
    direction: LegDirection,
    outcome: &LegOutcome,
    cache: &mut Cache,
) {
    let old_scaled = match direction {
        LegDirection::Entry { .. } => account
            .borrow_positions
            .get(hub_asset.clone())
            .map_or(Ray::ZERO, |p| Ray::from(p.scaled_amount)),
        // ...
    };
    let position = DebtPosition {
        scaled_amount: outcome.new_scaled,
    };
    apply_leg_usage(/* UsageSide::Borrow, Entry, delta = new - old */);
    cache.put_market_index(hub_asset, &outcome.market_index);
    cache.record_debt_position_update(/* ... */);
    account::update_or_remove_debt_position(account, hub_asset, &position);
}
```

| Step | Effect | Durable? |
|---|---|---|
| `old_scaled` | Entry: missing hub → `Ray::ZERO` | No |
| `LegOutcome` | From pool `PoolPositionMutation` (`new_scaled`, index, `actual_amount`) | No (trust pool SoT — A082/A077) |
| `apply_leg_usage` / `apply_spoke_entry` | Buffer usage; cap via `calculate_scaled_cap` on **returned** borrow index + decimals | No until finalize |
| `put_market_index` | Refresh Cache for later risk math | No |
| `record_debt_position_update` | Queue `EventBorrowDelta` | No until emit |
| `update_or_remove_debt_position` | Upsert debt; remove if `scaled_amount == 0` | No until finalize |

Cap failure or math panic aborts before finalize → no controller usage/position writes; Soroban rolls back the pool leg.

---

## 3. Post-pool solvency and `PositionSides` coupling

```96:104:contracts/controller/src/positions/mod.rs
pub(crate) fn enforce_post_pool_solvency(...) -> bool {
    let restamped = risk::restamp_listed_supply_ltv(cache, account);
    validation::require_post_pool_risk_gates(env, cache, account);
    restamped
}
```

`restamp_listed_supply_ltv` walks supply hubs still listed on the spoke and overwrites **only** `loan_to_value` when it differs from config. Liquidation threshold / bonus / fees are intentionally **not** touched on this path (harness `regression_borrow_restamps_ltv_only`).

`require_post_pool_risk_gates` (INV-RISK-01): if debt exists, require LTV-collateral ≥ debt, HF ≥ 1 WAD, and min-borrow-collateral floor when configured. Valuation uses the in-memory (possibly restamped) supply map plus post-merge debt.

### Why `Both` matters

```69:75:contracts/controller/src/positions/debt.rs
    let restamped = enforce_post_pool_solvency(env, &mut cache, &mut account);
    let sides = if restamped {
        PositionSides::Both
    } else {
        PositionSides::Debt
    };
    finalize_position_flow(env, account_id, &account, &mut cache, sides, false);
```

- **`Debt` only when `!restamped`:** supply bytes on disk already match memory; skipping the supply write saves rent without valuation drift.
- **`Both` when `restamped`:** persists the LTV the gate just used. Without this, disk would retain pre-governance (usually higher) LTV while the gate admitted the borrow under the cut LTV — later reads would overstate collateral relative to the gate observation.

Withdraw can discard the bool and still persist sibling LTV because it always writes `PositionSides::Supply`. Borrow cannot; the explicit branch is load-bearing. Certora `health_rules.rs` documents the `restamped -> PositionSides` coupling as the generalized TOB-AAVE-7 fence (`assert_gate_observation_is_final`).

Harness evidence:

- `regression_borrow_restamps_ltv_after_governance_cut` — blocked oversized borrow under cut LTV; successful smaller borrow persists LTV 5_000 on supply storage.
- `regression_borrow_restamps_ltv_only` — LTV updates; LT/bonus/fees stay vintage.

---

## 4. `finalize_position_flow` / `persist_account_positions`

```241:252:contracts/controller/src/positions/mod.rs
pub(crate) fn finalize_position_flow(...) {
    cache.persist_spoke_usage();
    persist_account_positions(env, account_id, account, sides, remove_if_empty);
    cache.emit_position_batch(account_id, account);
}
```

### 4.1 Spoke usage

`persist_spoke_usage` writes every row cached in `SpokeUsageContext` (lazy-loaded rows that `apply_entry` touched). `set_spoke_usage` deletes the key when both supplied and borrowed scaled RAY are 0; otherwise `set_shared` (shared TTL class). Caps already enforced per entry leg (A076/A077). Multi-asset borrow accumulates usage in one context then one persist (batch-friendly; A032).

### 4.2 Account position maps

```218:236:contracts/controller/src/positions/mod.rs
pub(crate) fn persist_account_positions(...) {
    if sides != PositionSides::Debt {
        storage::set_supply_positions(env, account_id, &account.supply_positions);
    }
    if sides != PositionSides::Supply {
        storage::set_debt_positions(env, account_id, &account.borrow_positions);
    }
    storage::renew_user_account(env, account_id);
    if remove_if_empty {
        account::cleanup_account_if_empty(env, account, account_id);
    }
}
```

`write_side_map` (`storage/account.rs:90-103`): empty map → `persistent.remove`; else `persistent.set` of the **entire** side map (no partial hub patch). First debt on a supply-only account creates `BorrowPositions`. Sibling debt hubs not in this borrow are preserved because the in-memory map was loaded fully at `get_account`.

TTL: `write_side_map` does not call `set_user`’s bump; `renew_user_account` immediately extends every existing account key (meta, supply, borrow, delegates). On this path the pair always runs together — safe. Calling `set_debt_positions` alone elsewhere without renew would skip bump (footgun for future call sites; not reachable here).

`remove_if_empty: false` — borrow cannot clear both sides; skipping cleanup avoids accidental NFT burn. Empty-map removal still applies if a side map became empty (should not on Entry borrow with positive scaled debt). Agrees with A036’s “flag false leaves rent until a later exit path” note; not a fund risk on borrow.

### 4.3 Events after storage

`emit_position_batch` publishes buffered borrow (and any supply) deltas then clears buffers. Order matches A033: durable spoke usage + positions commit before events. Event amounts use pool `actual_amount`; scaled debt storage uses pool `new_scaled`.

---

## 5. What is intentionally not written

| Item | Reason |
|---|---|
| `AccountMeta` | Spoke/mode immutable on borrow |
| `Delegates` | Unrelated; TTL still renewed if key exists |
| Controller-persisted market indexes | Pool is SoT; Cache `put_market_index` is invocation-local (A094) |
| Flash temporary flag | Ordinary borrow has no untrusted callback window by design (A007) |
| Supply map when `!restamped` | Bit-identical to disk; rent optimization with gate-final fence |

---

## 6. Related: `borrow_into_controller`

`debt.rs:248-297` validates entry gates, wraps `pool_create_strategy_call` in `with_flash_guard`, asserts `balance_delta_since == result.amount_received`, then `merge_debt_leg` (Entry). **No** `finalize_position_flow` here — strategies must `strategy_finalize` (A032). Storage-write responsibility for that debt mint is the strategy batch, not `process_borrow`. Same merge primitive, different persistence owner.

---

## 7. Failure / atomicity matrix

| Failure point | Controller durable writes | Pool writes |
|---|---|---|
| Auth / gates / aggregate before pool | None (except instance TTL renew at `Cache::new`) | None |
| Pool borrow panics | None | Rolled back |
| Cap / merge panic after pool returns | None (finalize not reached) | Rolled back with tx |
| Post-pool solvency panic | None | Rolled back |
| Finalize succeeds | SpokeUsage + BorrowPositions [+ Supply if Both] + TTLs + event | Committed |

There is no “pool committed / controller skipped” window visible outside a failed transaction.

---

## 8. Cross-checks

| Peer / INV | Relation |
|---|---|
| A003 | Owner-or-delegate before settle — foreign borrow cannot rewrite another account’s maps |
| A007 | Flash guard residual on listed-token hooks; ordinary borrow unset by design |
| A032 / A033 | Same finalize order; events not SoT |
| A036 | `remove_if_empty: false` appropriate |
| A040 | Only listed hubs reach merge/persist |
| A072 | Solvency gate before persist |
| A076/A077/A082 | Usage/caps from pool outcomes, not caller amounts |
| A094 | `put_market_index` required after pool merge — present |
| INV-RISK-01 | Re-prove after pool; bind restamped supply into gate then disk |
| INV-STOR-01 | Instance renew at start; account key renew at finalize; empty maps removable |

Disagreements: none filed. If a future agent claims borrow fails to persist restamped LTV, that contradicts `debt.rs:69-75` and `security_audit.rs` regressions — escalate via `disagreements/`.

---

## 9. Verdict

Borrow-path controller storage writes are **defended**: single finalize tail, pool-true scaled debt, spoke-usage caps on entry deltas, solvency-before-persist, and the `restamped → PositionSides::Both` branch that keeps gate-observed supply LTV on disk. Residual risks are listing-trust reentrancy (A007) and Cache-only indexes (by design), not missing or reordered durable position writes.
