# A025 — Repay path storage writes (`process_repay` / `settle_debt(Repay)` / finalize)

- Agent: A025
- Theme: T2
- Severity: info
- Status: defended
- Paths: `contracts/controller/src/lib.rs:137-141`; `contracts/controller/src/positions/debt.rs:78-212`; `contracts/controller/src/positions/mod.rs:52-67,112-188,206-252`; `contracts/controller/src/storage/account.rs:80-103,160-168,247-270`; `contracts/controller/src/account.rs:172-221`; `contracts/controller/src/context/{mod,spoke,events,market_index}.rs`; `contracts/controller/src/spoke_usage.rs:77-141`; `contracts/controller/src/storage/spoke.rs:65-79`; `contracts/controller/src/external/pool.rs:67-76`; `contracts/controller/src/payments.rs`; `contracts/controller/src/risk/validation.rs:12-24`; `common/src/token.rs:19-34`; `common/src/types/controller.rs:315-365`; `scripts/permissionless_entrypoints.txt:64`
- Defense: Permissionless repay mutates durable controller state only in one `finalize_position_flow` tail after measured token pull + pool repay + in-memory `merge_debt_leg`. Load shape is **borrow-only**; persist sides are **Debt only**; `remove_if_empty` is **false**. That triad prevents wiping live supply maps or burning the NFT from an empty in-memory supply view. Debt shares and spoke-usage borrow RAY decrease from **pool** `new_scaled` deltas, not caller request amounts. Empty debt maps remove the `BorrowPositions` key; meta / supply / delegates are not rewritten. Events emit after persists (A033).
- Gap: (1) Shared A080 residual — `apply_exit` no-ops when no `SpokeUsage` row exists, so a repay that burns debt may leave spoke borrow usage unchanged (capacity overstated until reconcile). (2) Shared A007/A023 residual — ordinary repay does not wrap token `transfer` / `pool_repay_call` in `with_flash_guard`; a governance-listed token with a transfer hook could re-enter monetary verbs against still-unpersisted in-memory state (listing trust + tx atomicity bound this). (3) Known A021/A036 rent residual — `remove_if_empty=false` leaves AccountMeta (+ Delegates) + NFT after debt map deletion even if supply were already empty; ordinary repay-then-withdraw cleans via withdraw’s `remove_if_empty=true`.
- Impact: Successful repay can only (a) decrease/remove per-account debt shares, (b) decrease (or remove-if-both-zero) `SpokeUsage` borrow RAY for touched hubs, (c) extend instance + live user TTLs, (d) emit observational batch events. Cannot mint debt, open supply slots, rewrite foreign supply/meta/delegates, retarget pool/oracle/NFT, or auto-burn the position NFT. A mistaken `PositionSides::Both` or `remove_if_empty=true` on this path would be Critical (wipe supply / burn NFT while collateral still live on disk). Current pairing closes that. Failed pool/token/assert steps revert the whole tx (pool + controller + transfers).
- Evidence: INV-AUTH-03, INV-ACCT-03/05, INV-HALT-01/03 (`AllowOnExit`), INV-STOR-01/03; Certora `usage_repay_tracks_scaled_delta`, `usage_repay_reachable`, solvency repay sanity; harness `tests/test-harness/tests/controller/repay.rs`, `account.rs` (`test_account_auto_removed_after_full_repay_withdraw`), `oracle/redstone_bulk.rs` (`test_full_repay_fires_zero_redstone_calls`); peers A002, A007, A021, A023, A024, A033, A036, A076, A080, A082.
- Opinion: Repay’s storage story is the deliberate mirror of borrow: risk-reducing, permissionless, borrow-only load, debt-only persist, no post-pool solvency restamp. Do not “optimize” by loading full account + writing `Both`, and do not flip `remove_if_empty` to true without also loading supply — either change alone is a fund-control / NFT-pairing bug.

## Scope and method

1. Read `shared/COORDINATION.md` + `SEED.md` (no git ops; findings-only).
2. Trace `Controller::repay` → `process_repay` → `settle_debt(Repay)` → measured transfers → `apply_repay_batch` → `merge_debt_leg(Exit)` → `finalize_position_flow` / `persist_account_positions`.
3. Enumerate every durable write (`persistent` / instance TTL / temporary flash flag) reachable on that path; separate Cache buffers from storage keys.
4. Cross-check peer findings A002, A007, A021, A023, A024, A033, A036, A076, A080, A082 and invariants INV-AUTH-03 / INV-STOR-01 / INV-STOR-03.
5. Related helpers `apply_repay_batch` / `execute_repayment` share the merge+pool primitive with liquidation; their **finalize owner** differs (A026). Documented here only as the shared write buffer, not as liquidation’s persist policy.

Out of scope for A025 as primary claims: pool cash/index books (A043), liquidation repay legs’ finalize (A026), strategy repay-with-collateral batching (A032/A049), lying-token measurement beyond noting `transfer_amount_measured` feeds the pool action amount (A055/A082).

---

## Call graph (ordinary repay)

```
Controller::repay                           # no #[when_not_paused]  lib.rs:137-141
  └─ process_repay                          debt.rs:81-112
       ├─ require_authorized_caller         auth + !flash_loaning (caller only)
       ├─ aggregate_positive_payments       dedupe hubs; reject ≤0 / empty
       ├─ get_account_borrow_only           meta + debt + NFT owner; supply=∅ in RAM
       ├─ Cache::new                        renews controller instance TTL
       ├─ settle_debt(DebtFlowKind::Repay)  debt.rs:161-183
       │    └─ for each aggregated hub:
       │         ├─ enforce_spoke_asset_flags AllowOnExit   (paused blocks; frozen OK;
       │         │                                           missing listing = no-op)
       │         ├─ get_debt_position_or_panic              must already owe this hub
       │         ├─ transfer_amount_measured                caller → pool (measured Δ)
       │         └─ make_pool_action(position, amount_in)
       │    └─ apply_repay_batch
       │         ├─ pool_repay_call                         pool burns debt; refunds overpay
       │         └─ for_each_leg → merge_debt_leg(Exit)
       │              ├─ apply_leg_usage Exit               Cache spoke-usage buffer
       │              ├─ put_market_index                   Cache only
       │              ├─ record_debt_position_update        event buffer
       │              └─ update_or_remove_debt_position     Account RAM only
       └─ finalize_position_flow(..., Debt, remove_if_empty=false)
            ├─ persist_spoke_usage                          SpokeUsage keys
            ├─ persist_account_positions
            │    ├─ (skip) set_supply_positions             CRITICAL: borrow-only load
            │    ├─ set_debt_positions                      WRITE/REMOVE BorrowPositions
            │    ├─ renew_user_account                      TTL on existing account keys
            │    └─ cleanup skipped                         remove_if_empty=false
            └─ emit_position_batch                          after durable writes (A033)
```

No `enforce_post_pool_solvency` / LTV restamp / oracle price prefetch on this path — repay only shrinks liabilities (HF non-decreasing). Harness `test_full_repay_fires_zero_redstone_calls` anchors the “no price read” property.

---

## Durable writes inventory

| Phase | Storage class | Key / target | Writer | Notes |
|---|---|---|---|---|
| Entry | Instance | controller instance TTL | `Cache::new` → `renew_controller_instance` | Every mutating path; INV-STOR-01 |
| Mid (token) | SAC balances | caller → pool | `transfer_amount_measured` | Not controller storage; measured Δ becomes pool action amount |
| Mid (pool FFI) | Pool contract | debt shares, cash, indexes | `pool_repay_call` | Trusted sibling; overpay refunded to payer; rolls back with tx on later panic |
| Mid (controller) | — | none | — | `merge_debt_leg` is **not** a durable write |
| Finalize | Persistent shared | `ControllerKey::SpokeUsage(spoke_id, hub)` | `SpokeUsageContext::persist` → `set_spoke_usage` | Borrow RAY ↓; remove key if both scaled sides are 0 |
| Finalize | Persistent user | `BorrowPositions(account_id)` | `set_debt_positions` → `write_side_map` | Full map snapshot; **remove key** if map empty after full repay |
| Finalize | Persistent user TTL | AccountMeta / Supply / Borrow / Delegates if present | `renew_user_account` | Extends TTL; no value rewrite. After empty debt remove, renew still hits meta/supply/delegates that remain |
| Finalize | Events | `UpdatePositionBatchEvent` | `emit_position_batch` | After persists; observational |
| Never on this path | Persistent user | `AccountMeta`, `SupplyPositions`, `Delegates` | — | Spoke/mode/collateral/delegates unchanged |
| Never on this path | Cleanup | all four account keys + NFT burn | — | `remove_if_empty=false` |
| Never durable | Cache maps | `market_indexes`, event vecs, spoke_assets, … | `put_market_index`, buffers | Dropped at end of invocation |
| Never set | Temporary | `FlashLoanOngoing` | — | Read-only check via `require_authorized_caller` |

---

## 1. Pre-write gates (no storage mutation)

1. **No global pause gate** — `repay` lacks `#[when_not_paused]` (INV-HALT-01 exit liveness; A001/A002). Storage may change during pause; policy, not a write bug. Harness: `test_repay_allowed_when_paused`.
2. **Auth** — `require_authorized_caller` (`caller.require_auth` + flash-loan gate). No `require_owner_or_delegate` on the target account (INV-AUTH-03: anyone may pay down debt).
3. **Inputs** — `aggregate_positive_payments` rejects empty vectors and non-positive legs; collapses duplicate hubs so each hub appears once.
4. **Account existence** — `get_account_borrow_only` requires meta + NFT owner (`AccountNotInMarket` / `AccountNotFound`); cannot invent an id.
5. **Debt slot existence** — `get_debt_position_or_panic` before transfer; cannot invent a debt hub to repay against.
6. **Listing / halt** — `FreezePolicy::AllowOnExit`: rejects `paused`, tolerates `frozen`; missing spoke-asset config is a no-op so delisted debt stays repayable (INV-HALT-03).

No `set_*` / `persist_*` runs in this phase. `Cache::new` only renews instance TTL.

---

## 2. Money then pool then memory merge (pre-persist)

### 2.1 Measured pull, then pool batch

For each aggregated hub, controller transfers from `payer` to the pool **before** `pool_repay_call`, using the observed balance delta as `PoolAction.amount` (`debt.rs:172-180`). Pool `ops/repay.rs` documents that expectation: funds must already sit in the pool; accounting burns scaled debt from the **controller-supplied** prior `position.scaled_amount`, credits cash by net repay, and refunds overpayment to the payer.

Implications for storage:

- Caller request amounts never enter `BorrowPositions` or `SpokeUsage` directly.
- Fee-on-transfer / shortfall tokens shrink `amount_in`; pool burns fewer shares; controller then merges `outcome.new_scaled` (A082 / A055 theme).
- All hubs are transferred first, then one `pool_repay_call` batch, then per-leg merges — a mid-batch panic reverts transfers + pool + (still-unwritten) controller state together.

### 2.2 `merge_debt_leg(Exit)` — buffered only

```148:188:contracts/controller/src/positions/mod.rs
pub(crate) fn merge_debt_leg(...) {
    let old_scaled = /* Exit: existing debt or panic */;
    let position = DebtPosition { scaled_amount: outcome.new_scaled };
    apply_leg_usage(... Exit, old_scaled, outcome);  // spoke-usage buffer
    cache.put_market_index(...);                     // cache only
    cache.record_debt_position_update(...);          // event buffer
    account::update_or_remove_debt_position(...);    // RAM map; drop if scaled==0
}
```

Exit usage delta = `old_scaled − outcome.new_scaled` (pool-returned). Cap checks are entry-only; exits do not re-check borrow caps. Zero-scaled slots are removed from the in-memory map before the eventual whole-map write (`update_or_remove_debt_position`), so durable maps never retain zero-share dust rows.

### 2.3 Why no post-pool solvency here

Borrow restamps LTV and gates HF **before** persist because it increases risk. Repay decreases debt only; loading supply solely to restamp LTV would be wasted work and would tempt a dangerous `Both` write after a borrow-only load. Skipping solvency is intentional and correct for this verb.

---

## 3. Finalize — the only controller durable write window

### 3.1 Order (A033)

`persist_spoke_usage` → `persist_account_positions` → `emit_position_batch`. Event buffers are not source of truth.

### 3.2 Spoke usage

`SpokeUsageContext::persist` writes every cached row via `set_spoke_usage`. Borrow side decreases; supply side of the same row is preserved. Both-zero rows are **removed** (`storage/spoke.rs:74-78`).

A080 residual: if no usage row existed, `apply_exit` returned early and finalize may write nothing for that hub — spoke borrow capacity can stay overstated relative to live positions. Soft governance limit; not direct fund theft. Certora `usage_repay_tracks_scaled_delta` covers the happy path where a row exists.

### 3.3 Debt map write + empty-key deletion

`PositionSides::Debt` ⇒ `set_debt_positions` only. Full-map replace; empty map ⇒ `persistent.remove(BorrowPositions(id))`. Sibling supply/meta/delegate keys are untouched.

### 3.4 TTL renew without NFT Owner bump

`renew_user_account` extends TTL of whichever of the four account keys still exist. After a full-hub repay that removes `BorrowPositions`, renew still extends meta / supply / delegates. NFT `Owner` TTL is **not** renewed on repay (INV-STOR-02b paths are mint / `renew_account` / explicit renew — agrees with A017/A034 theme). Not a repay write bug.

### 3.5 Cleanup deliberately skipped

```104:111:contracts/controller/src/positions/debt.rs
    finalize_position_flow(
        env,
        account_id,
        &account,
        &mut cache,
        PositionSides::Debt,
        false,  // remove_if_empty
    );
```

---

## 4. Load/persist coupling (load-bearing correctness)

### 4.1 Borrow-only load + Debt-only write

```160:168:contracts/controller/src/storage/account.rs
pub(crate) fn get_account_borrow_only(env: &Env, account_id: u64) -> Account {
    let meta = get_account_meta(env, account_id);
    let owner = account_owner(env, account_id);
    let borrow_positions = get_debt_positions(env, account_id);
    account_from_parts(owner, meta, Map::new(env), borrow_positions)
}
```

In-memory `supply_positions` is always empty on this path regardless of live collateral on disk. Persisting `PositionSides::Both` or `Supply` would call `set_supply_positions` with that empty map and **delete** the live `SupplyPositions` key — Critical collateral wipe. Current `PositionSides::Debt` makes that impossible. A021 §4.5 states the same layout contract; A025 confirms the repay call site obeys it.

### 4.2 Why `remove_if_empty` must stay false

`Account::is_empty` is `supply.is_empty() && borrow.is_empty()`. After a full repay under borrow-only load, **both** in-memory maps are empty even when durable supply still exists. If `remove_if_empty` were `true`, `cleanup_account_if_empty` would call `remove_account_and_burn_nft`, deleting meta/supply/debt/delegates and burning the NFT while collateral shares remain credited on the pool — Critical INV-STOR-03 / fund-control failure.

Contrast withdraw (A024): full account load + `PositionSides::Supply` + `remove_if_empty=true` is safe because `is_empty()` sees real supply+debt. Harness `test_repay_cleans_up_empty_account` and `test_account_auto_removed_after_full_repay_withdraw` clean only after a subsequent withdraw, not on repay alone — naming is historical; storage policy matches.

### 4.3 Asymmetry vs borrow

| Property | `process_borrow` | `process_repay` |
|---|---|---|
| Pause | `#[when_not_paused]` | live during pause |
| Auth on account | owner/delegate | caller-only (permissionless pay) |
| Load | full account | borrow-only |
| Post-pool solvency | yes; may restamp LTV | no |
| Persist sides | Debt, or Both if restamped | Debt only |
| `remove_if_empty` | false | false |
| Flash guard around pool/token | no (residual) | no (residual) |

---

## 5. Explicit non-writes

- **No `set_supply_positions`** — would be catastrophic after borrow-only load.
- **No `set_account_meta`** — spoke/mode immutable on this path.
- **No delegate map mutation**.
- **No spoke asset / spoke config / hub / pool / oracle / accumulator / protocol-config writes**.
- **No controller-side market-index persistence** — `put_market_index` is Cache-only; pool remains SoT (A038/A094).
- **No `FlashLoanOngoing` write** — flag is only checked.
- **No NFT mint/burn** on the happy path.
- **No oracle / price storage reads required** for the durable write set.

---

## 6. Shared helpers vs this verb’s persist owner

| Helper | Used by | Persists? |
|---|---|---|
| `apply_repay_batch` / `merge_debt_leg(Exit)` | `process_repay`, liquidation apply, `execute_repayment` | Buffer only |
| `finalize_position_flow(..., Debt, false)` | **`process_repay` only** among ordinary verbs | Yes — this file’s subject |
| Liquidation finalize | `PositionSides::Both` (+ separate cleanup branches) | A026 |
| Strategy repay-with-collateral | `strategy_finalize` → `Both`, `remove_if_empty=true` | A032 |

Do not conflate liquidation/strategy persist flags with ordinary repay.

---

## 7. Attack / regression scenarios

| Scenario | Outcome | Why |
|---|---|---|
| Stranger repays Alice’s debt | Success; stranger pays tokens; Alice debt ↓ | INV-AUTH-03; debt-only exit |
| Persist `Both` after borrow-only load | Would wipe supply | Prevented by `PositionSides::Debt` |
| `remove_if_empty=true` with borrow-only load | Would burn NFT + delete meta/supply | Prevented by `false` |
| Repay hub with no debt slot | Panic `DebtPositionNotFound` before transfer | `get_debt_position_or_panic` |
| Paused listing | Revert `SpokeAssetPaused` | `AllowOnExit` |
| Frozen listing | Repay allowed | Exit liveness |
| Delisted asset (no spoke-asset row) | Flags no-op; repay proceeds | Strand defense |
| Full repay all debt, supply remains | `BorrowPositions` key removed; meta+supply+NFT remain | Intended |
| Full repay then withdraw all | Account+NFT cleaned on withdraw | A024/A036 |
| Missing spoke-usage row | Exit no-op; usage may overstate capacity | A080 residual |
| Listed reentrant token during transfer | Possible mid-flight reentry before finalize | A007 residual; listing trust |
| Pool panic after transfers | Full tx revert | Soroban atomicity |
| Flash loan ongoing | Revert `#400` | `require_not_flash_loaning` |
| Zero / empty / negative payments | Rejected pre-storage | validators + harness |

---

## 8. Cross-links

- **A002** — permissionless repay auth matches INV-AUTH-03; liabilities only fall.
- **A007** — flash flag blocks monetary reentry during guarded windows; ordinary repay transfer residual shared with plain borrow.
- **A021** — layout matrix row for repay; borrow-only + Debt sides contract.
- **A023** — borrow mirror (full load, optional Both, post-pool gate).
- **A024** — withdraw contrast (`remove_if_empty=true` with full load).
- **A033** — persist-before-emit order.
- **A036** — empty-shell rent when cleanup flag false.
- **A076 / A080 / A082** — usage exit semantics, missing-row no-op, pool-output deltas.
- **A026 / A032** — other owners of `apply_repay_batch` finalize policy.

---

## 9. Tests / rules anchoring storage claims

| Claim | Evidence |
|---|---|
| Partial / full debt clear | harness `test_repay_partial`, `test_repay_full_clears_position` |
| Overpay does not inflate controller debt | `test_repay_overpayment_refunded` |
| Third-party payer | `test_repay_by_third_party`, `test_repay_permissionless_payer_auth_only` |
| Pause exit liveness | `test_repay_allowed_when_paused` |
| Multi-asset batch | `test_repay_multiple_assets`, `test_repay_duplicate_asset_payments_aggregate` |
| Flash gate | `test_repay_rejects_during_flash_loan` |
| Cleanup after repay+withdraw (not repay alone) | `test_repay_cleans_up_empty_account`, `test_account_auto_removed_after_full_repay_withdraw` |
| No oracle on full repay | `test_full_repay_fires_zero_redstone_calls` |
| Usage tracks pool scaled delta | Certora `usage_repay_tracks_scaled_delta` / `usage_repay_reachable` |
| Empty debt map removes key | unit `set_debt_positions_empty_map_removes_key` (A021) |
| Events after mutate | harness `test_repay_emits_events` + A033 order |

---

## Verdict

Ordinary `process_repay` durable writes are narrow, correctly sided, and fail-closed. The critical storage invariant is the **borrow-only load ↔ `PositionSides::Debt` ↔ `remove_if_empty=false`** triad. Treat any PR that breaks one leg of that triad without redesigning the load shape as Critical. Residual gaps are shared (A080 usage missing-row; A007 listed-token reentry; A036 empty-shell rent), not novel repay-only write-set bugs.
