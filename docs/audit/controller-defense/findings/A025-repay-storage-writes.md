# A025 — Repay path storage writes (`process_repay` / `merge_debt_leg` Exit / finalize)

- Agent: A025
- Theme: T2
- Severity: low
- Status: defended
- Paths: `contracts/controller/src/lib.rs:137-141`; `contracts/controller/src/positions/debt.rs:78-212`; `contracts/controller/src/positions/mod.rs:112-188,206-252,329-354,386-396`; `contracts/controller/src/account.rs:172-221`; `contracts/controller/src/storage/account.rs:66-168,247-270`; `contracts/controller/src/storage/spoke.rs:65-79`; `contracts/controller/src/context/{mod,spoke,market_index,events}.rs`; `contracts/controller/src/spoke_usage.rs:77-141`; `contracts/controller/src/risk/validation.rs:12-24`; `contracts/controller/src/payments.rs`; `common/src/token.rs:19-34`; `contracts/pool/src/ops/repay.rs`; `docs/reference/endpoints.md:114-123`
- Defense: Permissionless repay mutates controller durable state only in one `finalize_position_flow` tail after measured token pull + pool repay + in-memory Exit merges. Load is **borrow-only**; persist is **`PositionSides::Debt` only** with **`remove_if_empty: false`** — supply/meta/delegates values are never rewritten. Scaled debt and spoke-borrow usage come from pool mutation outputs; zero-scaled debt hubs are dropped from the map; empty `BorrowPositions` keys are removed. Instance + live account TTLs renew. Events emit after persists.
- Gap: (1) Shared A080 — `apply_exit` no-ops on missing spoke-usage row, so usage may stay overstated after a successful debt clear. (2) Shared A036/A021 — full debt close with no remaining supply leaves an empty-shell `AccountMeta` (+ NFT) because `remove_if_empty` is false; rent/stranded authority until a later cleanup path (withdraw already empty, bad-debt, or strategy finalize). Not a fund-theft or cross-account wipe under current flags.
- Impact: Successful repay can only **decrease or remove** target `BorrowPositions` hubs and **decrease** (or remove-if-both-zero) `SpokeUsage` borrow RAY for touched hubs, plus TTL bumps and observational events. Cannot mint debt, rewrite supply shares/risk stamps, retarget pool/oracle/NFT addresses, or mutate another account’s maps. Hypothetical footgun: flipping `remove_if_empty` to `true` without switching load to full `get_account` would treat borrow-only RAM as “empty” while `SupplyPositions` still exist on disk → burn NFT and delete meta while collateral key remains (INV-STOR-03 break). Current call site avoids that coupling.
- Evidence: INV-AUTH-03, INV-ACCT-03, INV-HALT-01/02 (`AllowOnExit`), INV-STOR-01/03; Certora `repay_does_not_increase_debt`, `repay_only_changes_target_account_debt`, `usage_exit_without_usage_row_is_a_noop`; harness `tests/test-harness/tests/controller/repay.rs` (partial/full/third-party/pause/flash/cleanup-via-withdraw); peers A002, A021, A023, A024, A033, A036, A080, A082, A094.
- Opinion: Repay’s write surface is the narrowest risk-reducing position path and is **correctly asymmetric** with borrow (no solvency restamp / no supply write) and with withdraw (no empty-account cleanup; borrow-only load). Keep the three-way coupling (`get_account_borrow_only` + `PositionSides::Debt` + `remove_if_empty: false`) documented as load-bearing; any cleanup improvement must load supply first.

## Scope and method

1. Read `shared/COORDINATION.md` + `SEED.md` (no git ops; findings-only).
2. Trace `Controller::repay` → `process_repay` → `settle_debt(Repay)` → `apply_repay_batch` → `merge_debt_leg(Exit)` → `finalize_position_flow(..., Debt, false)`.
3. Enumerate every durable write reachable on that path; separate Cache buffers, TTL-only bumps, pool FFI, and token movement.
4. Contrast with A023 (borrow) and A024 (withdraw) side-selection / cleanup flags; cross-check A002 (permissionless auth), A036 (cleanup), A080 (exit missing row).
5. Note shared helpers `apply_repay_batch` / `execute_repayment` used by liquidation/strategies — their finalize ownership is peer scope (A026/A032); this finding owns `process_repay`’s finalize args.

Out of scope as primary claims: pool cash/index books (A043), lying-token measurement depth (A055), liquidation repay legs’ seize storage (A026), strategy `repay_debt_with_collateral` finalize (A032/A049).

---

## Call graph (ordinary repay)

```
Controller::repay                            # no #[when_not_paused]; no owner gate
  └─ process_repay                           debt.rs:81-112
       ├─ require_authorized_caller          auth + !flash_loaning (caller only)
       ├─ aggregate_positive_payments        reject empty / ≤0; collapse duplicate hubs
       ├─ storage::get_account_borrow_only   meta + debt maps; supply = empty Map in RAM
       ├─ Cache::new                         renews controller instance TTL
       ├─ settle_debt(DebtFlowKind::Repay)   debt.rs:161-183
       │    ├─ enforce_spoke_asset_flags     AllowOnExit (paused blocks; frozen ok)
       │    ├─ get_debt_position_or_panic    cannot invent debt slot
       │    ├─ transfer_amount_measured      payer → pool (INV-ACCT-03); measured amount
       │    └─ apply_repay_batch
       │         ├─ pool_repay_call          pool burn + cash + overpay refund
       │         └─ for_each_leg → merge_debt_leg(Exit)
       │              ├─ apply_leg_usage Exit → apply_spoke_exit   Cache buffer
       │              ├─ put_market_index                          Cache only
       │              ├─ record_debt_position_update               event buffer
       │              └─ update_or_remove_debt_position            Account RAM
       └─ finalize_position_flow(..., Debt, remove_if_empty=false)
            ├─ persist_spoke_usage           SpokeUsage keys
            ├─ persist_account_positions
            │    ├─ (skip) set_supply_positions
            │    ├─ set_debt_positions       full map snapshot / remove if empty
            │    ├─ renew_user_account       TTL on existing account keys
            │    └─ cleanup skipped          remove_if_empty=false
            └─ emit_position_batch           after durable writes (A033)
```

Hard-coded finalize args at `debt.rs:104-111`: `PositionSides::Debt`, `remove_if_empty: false`. No `enforce_post_pool_solvency` on this path (debt only falls; no LTV restamp to persist).

---

## Durable writes inventory

| Phase | Storage class | Key / target | Writer | Notes |
|---|---|---|---|---|
| Entry | Instance | controller instance TTL | `Cache::new` → `renew_controller_instance` | Every mutating path; INV-STOR-01 |
| Mid (token) | Token contract | payer / pool balances | `transfer_amount_measured` | Cross-contract; rolls back with tx |
| Mid (pool FFI) | Pool contract | debt shares, cash, indexes | `pool_repay_call` | Overpayment refunded to payer; `actual_amount` = net repay |
| Mid (controller) | — | none | — | `merge_debt_leg` is **not** durable |
| Finalize | Persistent shared | `ControllerKey::SpokeUsage(spoke_id, hub)` | `SpokeUsageContext::persist` → `set_spoke_usage` | Removes key if both scaled sides are 0 |
| Finalize | Persistent user | `BorrowPositions(account_id)` | `set_debt_positions` → `write_side_map` | Full map snapshot; `remove` if empty map |
| Finalize | Persistent user TTL | AccountMeta / Supply / Borrow / Delegates if `has` | `renew_user_account` | Extends TTL; no value rewrite |
| Finalize | Events | `UpdatePositionBatchEvent` | `emit_position_batch` | After persists; observational |
| Never on this path | Persistent user | `SupplyPositions` **value** | — | Side skipped; disk supply untouched |
| Never on this path | Persistent user | `AccountMeta`, `Delegates` **value** | — | Spoke/mode/delegates unchanged |
| Never durable | Cache maps | `market_indexes`, event vecs, spoke_assets, … | `put_market_index`, buffers | Dropped at end of invocation |
| Never on this path | Temporary | `FlashLoanOngoing` | — | Read-only check via `require_not_flash_loaning` |

---

## 1. Pre-write gates (no accounting mutation)

1. **Pause** — no `#[when_not_paused]` on `repay` (INV-HALT-01 exit liveness; A001/A002). Storage may change during global pause by design.
2. **Auth** — `require_authorized_caller`: `caller.require_auth` + flash-loan gate. **No** `require_owner_or_delegate` on `account_id` (INV-AUTH-03 / A002 — anyone may repay anyone).
3. **Inputs** — `aggregate_positive_payments` rejects empty vectors and non-positive amounts; collapses duplicate hubs so each hub appears once in the pool batch.
4. **Account load** — `get_account_borrow_only` requires live `AccountMeta` (`AccountNotInMarket`) and resolvable NFT owner (`AccountNotFound`); assembles debt map from storage and an **empty** in-memory supply map (`storage/account.rs:160-168`).
5. **Listing halt** — per leg `FreezePolicy::AllowOnExit`: paused blocks; frozen tolerated; missing spoke-asset config no-ops so delisted debt stays repayable (`enforce_spoke_asset_flags`).
6. **Position existence** — `get_debt_position_or_panic` — stranger cannot open or invent a debt hub; missing hub → `DebtPositionNotFound`.

`Cache::new` only renews instance TTL in this phase. No `set_*` / `persist_*` of positions yet.

---

## 2. Money move then pool then in-memory Exit merge

### 2.1 Measured pull before pool

For each aggregated hub, repay transfers from `payer` (= `caller`) to the pool via `transfer_amount_measured`, then builds `PoolAction` with the **measured** receipt (`debt.rs:172-180`). Matches INV-ACCT-03 / A082 pattern: pool sees what the pool actually received, not a raw caller claim. Fee-on-transfer / lying tokens that deliver ≤0 panic before pool; short deliveries shrink the pool action.

Order within the loop: flag check → load position → **transfer** → push action. All transfers complete before `apply_repay_batch`’s single `pool_repay_call`. A later panic rolls back transfers and any pool work (Soroban atomicity).

### 2.2 `apply_repay_batch` / pool SoT

```190:212:contracts/controller/src/positions/debt.rs
pub(crate) fn apply_repay_batch(...) -> Vec<PoolPositionMutation> {
    let results = pool_repay_call(env, &pool_addr, payer, actions);
    for_each_leg(env, actions, &results, |entry, result| {
        merge_debt_leg(..., LegDirection::Exit, &LegOutcome::from(&result), ...);
    });
    results
}
```

Pool `ops/repay.rs` accrues, burns shares, credits net cash, refunds overpayment to `payer`, and returns `PoolPositionMutation` whose `actual_amount` is **net** repay (excludes refund). Positive net that burns zero scaled shares panics `RepayRoundsToZeroShares` — no controller merge for dust-only burns.

`for_each_leg` asserts `entries.len() == results.len()` then merges in order. Aggregation already deduped hubs, so evolving in-memory debt map is not double-applied for the same hub in one call.

### 2.3 `merge_debt_leg` (Exit) — buffer only

```148:188:contracts/controller/src/positions/mod.rs
// Exit must find a leg; old_scaled from get_debt_position_or_panic path / map.
apply_leg_usage(..., UsageSide::Borrow, LegDirection::Exit, old_scaled, outcome);
cache.put_market_index(...);           // Cache only — pool remains SoT (A094)
cache.record_debt_position_update(...);
account::update_or_remove_debt_position(...);  // remove if new_scaled == 0
```

| Step | Effect | Durable? |
|---|---|---|
| `old_scaled` | Exit: existing debt required (panic if missing) | No |
| `LegOutcome` | Pool `new_scaled`, index, net `actual_amount` | No (pool SoT) |
| `apply_spoke_exit` | Buffer `old − new` borrow RAY; no-op if delta 0 or **no usage row** (A080) | No until finalize |
| `put_market_index` | Refresh Cache for later same-tx reads | No |
| Event buffer | `PositionAction::Repay` debt delta | No until emit |
| `update_or_remove_debt_position` | Upsert or drop hub when `scaled_amount == Ray::ZERO` | No until finalize |

Exit never calls `enforce_spoke_cap` (INV-HALT-03 / A076). Underflow if usage row exists but is below the exit delta → `InternalError` / math panic → tx abort before finalize.

---

## 3. Why no post-pool solvency / no supply write

Borrow (A023) must re-prove INV-RISK-01 after increasing debt and may restamp supply LTV → conditional `PositionSides::Both`.

Repay **only reduces** debt. Health improves or stays solvent; there is no restamp to flush. Calling `enforce_post_pool_solvency` would be wasteful and, with borrow-only load, would see **empty supply** in RAM — a false insolvency / false “empty collateral” observation if gates ran. Omitting the gate is therefore both correct for permissionless repay and **required** given the load shape.

Consequently finalize is hard-coded to `PositionSides::Debt`:

```218:230:contracts/controller/src/positions/mod.rs
    if sides != PositionSides::Debt {
        storage::set_supply_positions(...);
    }
    if sides != PositionSides::Supply {
        storage::set_debt_positions(...);
    }
```

Skipping supply write means the empty in-memory supply map from `get_account_borrow_only` **cannot** clobber live `SupplyPositions` on disk. This is the primary defense against collateral erasure on the repay path (A021 blast-radius note).

---

## 4. `finalize_position_flow` / cleanup asymmetry

### 4.1 Spoke usage

`persist_spoke_usage` writes every row present in the invocation’s `SpokeUsageContext`. Rows appear when `apply_exit` successfully loaded and updated them. `set_spoke_usage` deletes the key when both supplied and borrowed scaled RAY are 0; a debt exit to zero leaves supply usage intact on a mixed row.

If no usage row existed, Exit no-ops (A080): account debt still clears correctly; spoke borrow capacity can stay soft-overstated until reconcile.

### 4.2 Debt map snapshot

`set_debt_positions` / `write_side_map`: empty map → `persistent.remove(BorrowPositions)`; else full-map `set`. Sibling debt hubs not repaid remain because they were loaded into the borrow-only account and never removed in memory. First-time empty after full multi-asset close deletes the key (rent hygiene for that side).

TTL: `renew_user_account` extends every still-present account key (meta, supply, borrow, delegates). After empty debt-map removal, borrow key is absent so it is not renewed; supply/meta/delegates still bump if present.

### 4.3 `remove_if_empty: false` (load-bearing)

```104:111:contracts/controller/src/positions/debt.rs
    finalize_position_flow(
        env,
        account_id,
        &account,
        &mut cache,
        PositionSides::Debt,
        false,
    );
```

| Scenario after successful repay | Disk supply | In-memory `is_empty()` | Cleanup if flag were `true` | Actual (`false`) |
|---|---|---|---|---|
| Partial repay; collateral remains | Non-empty | **true** (borrow-only load hides supply) | Would burn NFT + delete meta while supply key remains | Safe: no cleanup |
| Full debt clear; collateral remains | Non-empty | **true** | Same INV-STOR-03 disaster | Safe: shell kept with supply |
| Full debt clear; supply already empty (e.g. post-seize leftover debt repaid) | Empty | true | Correct cleanup | **Residual shell** (A036): meta + NFT remain until another path |

Harness `test_repay_cleans_up_empty_account` still ends empty only after a subsequent `withdraw_all` (which uses `remove_if_empty: true` + full `get_account` — A024). Repay alone is not the cleanup verb.

Agrees with A036 (“prefer remove_if_empty true on full-exit”) **with the constraint**: repay cannot flip the flag without also switching to `get_account` (or an explicit empty-supply read). A021 already flags this coupling.

### 4.4 Events after storage

`emit_position_batch` publishes buffered borrow deltas (`Repay` action) then clears buffers. Supply event vec stays empty. Order matches A033. Event amounts use pool net `actual_amount`; durable scaled debt uses pool `new_scaled`.

`AccountAttributes` on the event come from spoke/mode fields on `Account` (meta), not from the empty supply map — no false “zero collateral” attribute leak beyond what attributes already encode.

---

## 5. What is intentionally not written

| Item | Reason |
|---|---|
| `SupplyPositions` values | `PositionSides::Debt` + borrow-only load; must not persist empty map |
| `AccountMeta` | Spoke/mode immutable on repay |
| `Delegates` | Unrelated; TTL still renewed if key exists |
| Controller-persisted market indexes | Pool is SoT; Cache `put_market_index` is invocation-local |
| Flash temporary flag | Ordinary repay has no callback window; gate is read-only |
| Post-pool LTV restamp | Not invoked; no supply mutation to flush |
| Account cleanup / NFT burn | `remove_if_empty: false` — withdraw/strategy/bad-debt own empty lifecycle |

---

## 6. Permissionless write blast radius

Anyone who can pay tokens may:

- Reduce or clear **target** `BorrowPositions` hubs.
- Decrement (or fail-soft on missing) spoke borrow usage for those hubs.
- Renew instance + target account TTLs.

Anyone cannot:

- Increase target debt or open a missing debt hub.
- Rewrite target supply / meta / delegates values.
- Touch any other account’s position keys (Certora `repay_only_changes_target_account_debt`).
- Bypass flash reentrancy gate into nested monetary verbs (`test_repay_rejects_during_flash_loan`).

STRIDE Elevation.5 / I4: permissionless repay is risk-reducing only — consistent with A002.

---

## 7. Failure / atomicity matrix

| Failure point | Controller durable writes | Pool / token |
|---|---|---|
| Auth / aggregate / missing account or debt / paused listing | None (except instance TTL renew at `Cache::new`) | None |
| Measured transfer panics (≤0 receipt) | None | Rolled back |
| Pool repay panics (incl. rounds-to-zero) | None | Rolled back with prior transfers |
| Cap/usage underflow / merge panic after pool returns | None (finalize not reached) | Rolled back with tx |
| Finalize succeeds | SpokeUsage (+/− remove) + BorrowPositions set/remove + TTLs + event | Committed |

No externally visible “pool committed / controller skipped” window outside a failed transaction.

---

## 8. Related helpers (not `process_repay` finalize)

| Helper | Storage ownership |
|---|---|
| `apply_repay_batch` | Shared merge primitive; **caller** must finalize |
| `execute_repayment` | Single-leg wrapper for liquidation/strategies; same merge; finalize elsewhere (A026/A032) |
| `borrow_into_controller` | Entry merge only; strategy finalize (A023 §6 / A032) |

Do not assume `apply_repay_batch` alone persists — `process_repay`’s defense is the explicit finalize tail immediately after settle.

---

## 9. Invariant / defense checklist

| Claim | Verdict | Notes |
|---|---|---|
| Caller auth + flash gate before writes | Match | `require_authorized_caller` |
| No owner gate; liabilities only fall | Match | INV-AUTH-03 / A002; Exit-only merge |
| Measured receipt drives pool amount | Match | INV-ACCT-03 |
| Debt scaled only from pool `new_scaled` | Match | A082; Certora `repay_does_not_increase_debt` |
| Supply disk not clobbered | Match | `PositionSides::Debt` + borrow-only load |
| Spoke usage exit from pool scaled delta | Match | `old − new`; Certora usage exit rules |
| Exit does not hit borrow cap | Match | `apply_exit` skips caps |
| Empty debt map removes storage key | Match | `write_side_map` |
| Empty account + NFT paired cleanup on this path | **Not claimed** | Flag false; residual shell (A036) |
| Missing usage row decrements usage | **Residual** | A080 no-op |
| Pause still allows repay writes | Match | By design |
| Cross-account isolation | Match | Certora isolation rule; single `account_id` key space |
| Events not SoT | Match | After persist (A033) |

---

## 10. Tests / formal anchors

| Check | Location |
|---|---|
| Partial repay keeps debt hub | `repay.rs` `test_repay_partial` |
| Full repay clears borrow count | `test_repay_full_clears_position` |
| Overpay refunded; debt cleared | `test_repay_overpayment_refunded` |
| Third-party payer clears target debt | `test_repay_by_third_party`, `test_repay_permissionless_payer_auth_only` |
| Multi-asset + duplicate aggregate | `test_repay_multiple_assets`, `test_repay_duplicate_asset_payments_aggregate` |
| Allowed while paused | `test_repay_allowed_when_paused` |
| Flash blocked | `test_repay_rejects_during_flash_loan` |
| Empty account only after repay **and** withdraw | `test_repay_cleans_up_empty_account`, `account.rs` `test_account_auto_removed_after_full_repay_withdraw` |
| Debt non-increasing | Certora `repay_does_not_increase_debt` |
| Other account untouched | Certora `repay_only_changes_target_account_debt` |
| Exit missing usage row noop | Certora `usage_exit_without_usage_row_is_a_noop` |

---

## 11. Cross-links

| Peer | Relation |
|---|---|
| A002 | Permissionless auth / no owner pin — write blast radius is debt-down only |
| A021 | Layout + borrow-only / `PositionSides` coupling called out as load-bearing |
| A023 | Contrast: borrow may write supply when restamped; repay never |
| A024 | Contrast: withdraw uses full load + `remove_if_empty: true` |
| A033 | Persist-before-emit order identical |
| A036 | Empty-shell residual when flag false; agrees — fix requires full load |
| A040 | Only existing debt hubs (listed or delisted-exitable) reach merge |
| A076/A082 | Exit usage from pool outcomes |
| A080 | Only material soft-cap residual on this path’s storage semantics |
| A094 | `put_market_index` present after pool merge; Cache-only by design |

Disagreements: none filed. Do not treat A036’s “prefer remove_if_empty true” as a mandate to flip the repay flag alone — that would conflict with `get_account_borrow_only` unless the load shape changes in the same edit.

---

## 12. Verdict

`process_repay` controller storage writes are **defended**: single finalize tail, debt-side-only persistence, pool-true scaled shares, measured token intake, permissionless but strictly risk-reducing mutation set, and a deliberate **borrow-only load ↔ Debt sides ↔ no cleanup** triad that prevents supply wipe. Record A080 (missing usage row) and A036 (empty-shell rent after debt-only full close) as shared residuals, not as under-defended repay bookkeeping. Any future empty-account cleanup on repay must load supply (or prove both sides empty on disk) before `remove_if_empty: true`.
