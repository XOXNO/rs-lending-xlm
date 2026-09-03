# A027 — Bad-debt socialization storage writes (`bad_debt.rs`)

- Agent: A027
- Theme: T2 (storage mutations)
- Severity: low
- Status: defended (accepted residuals documented)
- Paths: `contracts/controller/src/positions/liquidation/bad_debt.rs`; callers `liquidation/mod.rs:238-275`, `liquidation/apply.rs:326-340`; cascade `account.rs:164-168`, `storage/account.rs:248-254`, `storage/spoke.rs:65-78`, `spoke_usage.rs:77-141`, `external/pool.rs:91-97` → `contracts/pool/src/lib.rs:222-224`, `ops/seize.rs`, `interest.rs:75-91`, `cache/mod.rs:72-84`, `cache/shares.rs:31-48`
- Defense: Cleanup is a single atomic invocation that (1) buffers spoke-usage exits for every remaining supply and debt share, (2) seizes those shares on the pool (deposit → revenue; borrow → index writedown + debt burn), (3) persists spoke usage, (4) emits `CleanBadDebtEvent`, (5) deletes all four account persistent keys and burns the position NFT. Gates live outside `bad_debt.rs` (`BadDebtGate` / `is_socializable_bad_debt`). No controller path writes pool `PoolKey::State`; no pool path writes controller account keys.
- Gap: (a) Pool seize commits with no `guards::require_backed_market` post-condition (INV-LIQ-04 **NOT ENFORCED**). (b) Same-market supply is absorbed as revenue before debt writedown without netting (ADR-0021 deferred; pinned arithmetic). (c) Liquidation path may `finalize_position_flow` write residual position maps then immediately `remove_account_entry` them. (d) `apply_exit` missing-row no-op is inherited (A080). None of these are silent cross-key corruption bugs inside `execute_bad_debt_cleanup`.
- Impact: (a) A market can be left with a backing shortfall until `recapitalize`; ordinary utilization caps bound how close one liquidation gets to the index floor. (b) Same-market cleanup over-haircuts suppliers relative to a netted design while booking protocol revenue — value conserved, allocation intentional until ADR-0021. (c) Extra rent write in-tx only. (d) Cap accounting drift, not direct theft.
- Evidence: INV-LIQ-04, INV-IDX-02/03, INV-STOR-01/03; ADR-0012, ADR-0021; Certora `clean_bad_debt_zeros_positions`, `usage_liq_bad_debt_cleanup_sheds_every_wiped_position`, `bad_debt_writedown_is_noop_on_empty_market`, `seize_borrow_reduces_debt_and_writes_down_supply`; harness `bad_debt_index.rs`, `same_market_bad_debt_cleanup_arithmetic.rs`, `position_nft.rs` (`clean_bad_debt_burns_nft`, `force_socialize_bad_debt_burns_nft`). Cross-refs: A014 (authority gates), A080 (exit missing row), A084 (liq usage), A031/A036 (NFT/account delete), A033 (event-after-persist on other flows).
- Opinion: Storage write set for bad-debt socialization is small, ordered, and fail-closed. Treat post-seize solvency and same-market netting as known protocol residuals, not as missing deletes or double-writes in `bad_debt.rs`.

## Method

1. Enumerated every durable write/remove reachable from `execute_bad_debt_cleanup`, including the cross-contract pool seize cascade.
2. Mapped call sites (`clean_bad_debt`, `force_socialize_bad_debt`, post-liquidation auto-clean) and what state each site has already committed before cleanup runs.
3. Checked key lifecycle against INV-STOR-01/03 and index isolation against INV-IDX-03.
4. Compared observed arithmetic/order to formulas.md, ADR-0012, ADR-0021, and INV-LIQ-04’s explicit “NOT ENFORCED” note.
5. Did not re-litigate authority/dust gates (A014) except where they gate whether storage mutates at all.

---

## 1. Surface under audit

`execute_bad_debt_cleanup` is the only body that socializes and deletes. It does **not** re-check insolvency or dust; callers must have admitted the account:

| Caller | Gate | Auth / halt |
|---|---|---|
| `socialize_bad_debt` ← `clean_bad_debt_standalone` ← `process_clean_bad_debt` | `BadDebtGate::DustCapped` → `is_socializable_bad_debt` | `caller.require_auth` + flash-loan gate; not pause-gated |
| `socialize_bad_debt` ← `process_force_socialize_bad_debt` | `BadDebtGate::InsolventOnly` | `#[only_owner]`; flash-loan gate; `renew_then!` on admin wrapper |
| `check_bad_debt_after_liquidation` | dust gate only; else empty-account cleanup | Runs inside `process_liquidation` after position finalize |

```15:62:contracts/controller/src/positions/liquidation/bad_debt.rs
pub(crate) fn execute_bad_debt_cleanup(
    env: &Env,
    cache: &mut Cache,
    account_id: u64,
    account: &Account,
    totals: &AccountRiskTotals,
) {
    // ... build PoolSeizeEntry batch from remaining supply then debt ...
    pool_seize_positions_call(env, &pool_addr, &entries);
    cache.persist_spoke_usage();
    CleanBadDebtEvent { /* pre-cleanup totals */ }.publish(env);
    remove_account_and_burn_nft(env, account_id);
}
```

---

## 2. Controller durable writes (complete inventory)

### 2.1 Keys touched

| Key | Storage class | Op in cleanup | Helper |
|---|---|---|---|
| `ControllerKey::SpokeUsage(spoke_id, hub_asset)` | persistent shared | set or remove (zero prune) | `SpokeUsageContext::persist` → `set_spoke_usage` |
| `ControllerKey::AccountMeta(account_id)` | persistent user | **remove** | `remove_account_entry` |
| `ControllerKey::SupplyPositions(account_id)` | persistent user | **remove** | `remove_account_entry` |
| `ControllerKey::BorrowPositions(account_id)` | persistent user | **remove** | `remove_account_entry` |
| `ControllerKey::Delegates(account_id)` | persistent user | **remove** | `remove_account_entry` |
| Position NFT `Owner` / `Balance` (external) | NFT contract | burn | `nft_burn_call` after controller removes |

**Not written by cleanup:** instance protocol keys (`Pool`, aggregators, …), spoke/hub config, position-manager allowlist, flash-loan temp flag, account position maps via `set_supply_positions` / `set_debt_positions` (standalone path never rewrites maps—it deletes them).

### 2.2 Spoke usage sequence

1. For each remaining supply position: `cache.apply_spoke_exit(spoke, Supply, hub, scaled)`.
2. For each remaining debt position: `cache.apply_spoke_exit(spoke, Borrow, hub, scaled)`.
3. After the pool FFI returns: `cache.persist_spoke_usage()`.

`apply_exit` loads-or-skips the row, subtracts the **exact** controller scaled amount, panics on underflow/`next < 0`, buffers in `SpokeUsageContext`. `persist` writes every buffered hub; `set_spoke_usage` deletes the key when both sides are zero (INV-STOR-01 empty prune).

Certora `usage_liq_bad_debt_cleanup_sheds_every_wiped_position` pins: after cleanup, usage equals the seeded “extra” contributed by other accounts, and the wiped account’s stored scaled totals read as zero/absent.

**Inherited residual (A080):** if no usage row exists, exit is a silent no-op. Positions are still seized and the account still deleted, so a missing row cannot leave a live account with stranded usage—but a pre-existing **over-count** (usage > sum of positions) is only reduced by this account’s scaled amounts, leaving excess capacity consumption until some other reconcile. Cap distortion only; see A080.

### 2.3 Account deletion and NFT pairing (INV-STOR-03)

```164:168:contracts/controller/src/account.rs
pub(crate) fn remove_account_and_burn_nft(env: &Env, account_id: u64) {
    storage::remove_account_entry(env, account_id);
    let nft = storage::get_position_nft(env);
    nft_burn_call(env, &nft, account_id);
}
```

```248:254:contracts/controller/src/storage/account.rs
pub(crate) fn remove_account_entry(env: &Env, account_id: u64) {
    let persistent = env.storage().persistent();
    persistent.remove(&ControllerKey::AccountMeta(account_id));
    persistent.remove(&ControllerKey::SupplyPositions(account_id));
    persistent.remove(&ControllerKey::BorrowPositions(account_id));
    persistent.remove(&ControllerKey::Delegates(account_id));
}
```

Order is **remove controller keys → burn NFT**. Host transaction atomicity undoes both on failure. Delegates are cleared with the account (no orphaned authority under INV-STOR-01). Harness `clean_bad_debt_burns_nft` / `force_socialize_bad_debt_burns_nft` pin the pairing on both gates.

Cleanup does **not** emit `UpdatePositionBatchEvent`. That is contractual (`liquidation/mod.rs:122-125`, `docs/reference/events.md` CleanBadDebt section, `architecture.md` indexer notes): indexers must treat `CleanBadDebtEvent` + NFT burn as the wind-down signal.

### 2.4 Liquidation path: write-then-delete

`process_liquidation` calls `finalize_position_flow(..., PositionSides::Both, false)` **before** `check_bad_debt_after_liquidation`. That finalize:

1. persists spoke usage (post-repay/seize deltas),
2. writes residual `SupplyPositions` / `BorrowPositions`,
3. emits the liquidated account’s position batch,

then cleanup may delete those same position keys (and meta/delegates) in the same transaction.

Verdict: correct, not a consistency bug. It is in-tx write amplification and ensures indexers see post-liquidation residuals before `CleanBadDebtEvent` (architecture event-order contract). Standalone `clean_bad_debt` / `force_socialize` never rewrite position maps—only usage persist + full remove.

---

## 3. Pool durable writes (cascade from `pool_seize_positions_call`)

Controller builds one `Vec<PoolSeizeEntry>`: **all supply legs first**, then **all debt legs**, keyed by the account’s maps (at most one entry per hub per side).

Pool entrypoint:

```222:224:contracts/pool/src/lib.rs
    fn seize_positions(env: Env, entries: Vec<PoolSeizeEntry>) {
        ops::run_batch(&env, entries, |e, entry| ((), ops::seize::apply(e, entry)));
    }
```

`run_batch` renews the pool instance TTL, applies each leg, commits, then emits a market-state batch. Each leg:

```18:35:contracts/pool/src/ops/seize.rs
pub(crate) fn apply(env: &Env, entry: &PoolSeizeEntry) -> MarketStateSnapshot {
    require_nonneg_amount(env, entry.position.scaled_amount);
    let mut cache = ops::synced_market(env, &entry.hub_asset);
    let position = Ray::from(entry.position.scaled_amount);
    match entry.side {
        AccountPositionType::Borrow => {
            let bad_debt = cache.unscale_borrow_ceil_ray(position);
            interest::apply_bad_debt_to_supply_index(&mut cache, bad_debt);
            cache.burn_debt(position);
        }
        AccountPositionType::Deposit => {
            cache.absorb_supply_as_revenue(position);
        }
    }
    cache.commit()
}
```

### 3.1 Per-leg state mutations → `PoolKey::State(hub_asset)`

| Side | In-memory ops | Committed fields |
|---|---|---|
| Deposit | `absorb_supply_as_revenue`: `revenue += scaled`; `supplied` unchanged; `require_revenue_backed` | `revenue` ↑; indexes/cash/borrowed unchanged by absorb |
| Borrow | `unscale_borrow_ceil_ray` (liability ceil) → `apply_bad_debt_to_supply_index` → `burn_debt` | `supply_index` ↓ (floored); `borrowed` ↓ by seized scaled |

`apply_bad_debt_to_supply_index`:

- No-op if `supplied * supply_index == 0` (empty market); debt burn still proceeds afterward.
- Caps loss at total supplied value; two-floor reduction (`div_floor` then `mul_floor`); clamps to `SUPPLY_INDEX_FLOOR_RAW` (INV-IDX-02).
- Touches **only** the synced hub’s cache (INV-IDX-03). Untouched markets bit-identical — harness `test_socialization_leaves_an_untouched_market_bit_identical` / force twin.

`cache.commit()` → `storage::write_state` under `PoolKey::State`. `PoolKey::Params` is not written on seize.

### 3.2 Isolation and trust boundary

- Seize is `#[only_owner]` on the pool; the controller is the owner. Users cannot call seize directly.
- Seize amounts come from **controller** position books, not from caller-supplied floats. Over-seize vs pool `borrowed`/`supplied` underflows in `burn_debt` / `absorb` path asserts → full tx revert (fail closed).
- One `synced_market` + `commit` per entry: same-hub supply then debt sees the post-absorb `revenue` before writedown; accrual uses ledger time (same-ledger second sync is typically a no-op).

### 3.3 No post-commit solvency guard (INV-LIQ-04 residual)

Unlike `ops/revenue.rs`, seize does not call `require_backed_market` / `require_solvent_withdraw_state`. Invariants.md states this explicitly under INV-LIQ-04 **NOT ENFORCED**. Recovery path is permissionless `recapitalize`. Harness `test_socialization_leaves_the_market_backed_and_open` shows ordinary liq+dust socialization stays backed; the index-floor wipeout case is bounded away by utilization caps for a single ordinary liquidation.

Severity for A027: **low** — documented residual, not an omitted controller key delete.

---

## 4. Ordering, atomicity, and consistency claims

### 4.1 Intended order inside `execute_bad_debt_cleanup`

```
spoke exits (buffer)
  → pool seize batch (each leg: sync → mutate → commit State)
  → persist SpokeUsage
  → CleanBadDebtEvent
  → remove AccountMeta/Supply/Borrow/Delegates → NFT burn
```

Implications:

- If pool seize panics mid-batch, host aborts: no usage persist, no account delete, prior legs in the same tx also roll back.
- Usage is persisted only after the pool has accepted the full seize batch — avoids “usage shed, pool still holding liability” on revert paths.
- Event is observational; published after durable pool+usage updates, before account delete. On success, account keys are gone; on failure, nothing publishes.

Contrast with `finalize_position_flow` (A033): that helper persists then emits position batches. Bad-debt intentionally skips position-batch emission.

### 4.2 Controller ↔ pool book consistency after success

| Book | Post-condition |
|---|---|
| Controller account maps / meta / delegates | Absent |
| Controller spoke usage | Decremented by wiped scaled amounts (or unchanged if row was missing — A080) |
| Pool `borrowed` | Reduced by each seized debt scaled amount |
| Pool `revenue` | Increased by each seized supply scaled amount |
| Pool `supply_index` | Reduced on each debt market per formula (or unchanged if empty-market no-op / zero bad debt) |
| Pool `supplied` / `cash` | Unchanged by seize itself |

User positions are not stored on the pool; aggregate share conservation relies on controller always seizing the full remaining maps. Certora `clean_bad_debt_zeros_positions` asserts deposit/borrow lists empty after standalone clean.

### 4.3 Same-market supply-then-debt (ADR-0021)

With supply and debt on the same hub in one batch:

1. Deposit leg reclassifies the account’s supply shares as protocol revenue (`supplied` unchanged).
2. Borrow leg writes the **full** debt value into the supply index and burns debt shares.

Pinned by `same_market_bad_debt_cleanup_arithmetic.rs`. Threat model and ADR-0021 record this as accepted allocation (protocol revenue vs supplier haircut) until an explicit `net_settle`-before-seize redesign. **Not a storage double-write bug** — two intentional commits to the same `PoolKey::State`.

Cross-market: collateral hubs only absorb revenue; debt hubs only writedown + burn. Loss does not jump hubs (INV-IDX-03 / ADR-0012).

---

## 5. Call-site storage context matrix

| Path | Cache / positions before cleanup | Extra prior writes | Cleanup effect |
|---|---|---|---|
| `clean_bad_debt` | Fresh `Cache`; account loaded once | None | Usage persist + full account remove + pool seize |
| `force_socialize_bad_debt` | Same | Controller instance renew via `renew_then!` | Same |
| Post-`liquidate` auto-clean | Same `Cache` as liquidation (spoke usage already partially updated & once-persisted) | `finalize_position_flow` wrote residual positions + usage; optional receiver finalize | Further usage exits + persist; **deletes** residual position keys just written; pool seize of leftovers |

Flash-loan temporary storage is only read (gate); cleanup does not set `FlashLoanOngoing`.

`clean_bad_debt` does not wrap `renew_then!` on the controller instance (unlike force). Shared usage writes still bump shared TTL; account keys are removed. Instance renew asymmetry is outside this file’s mutation set (see A017).

---

## 6. Negative checks (what cleanup must not do)

| Anti-property | Result |
|---|---|
| Write another account’s position maps | Does not — only `account_id` remove |
| Mute or alter untouched markets’ `PoolKey::State` | Does not — per-entry hub sync only |
| Leave AccountMeta without burning NFT (or burn without remove) | Single helper pairs both; atomic tx |
| Leave Delegates after meta gone | Delegates key removed in same `remove_account_entry` |
| Persist empty SpokeUsage rows | Zero prune in `set_spoke_usage` |
| Socialize without gate | Gates in callers; `bad_debt.rs` assumes admission |
| Emit position deltas that disagree with deleted maps | No position batch on this path by design |
| Reduce supply index below floor | `max(..., SUPPLY_INDEX_FLOOR_RAW)` |
| Credit liquidator / stranger from cleanup | No token transfers; deposit→revenue, borrow→index |

---

## 7. Findings summary

| ID | Issue | Severity | Status |
|---|---|---|---|
| A027-1 | Controller account + spoke-usage + NFT lifecycle on cleanup is complete and ordered | — | defended |
| A027-2 | Pool state writes isolated per hub; formula + floor match ADR-0012 / formulas.md | — | defended |
| A027-3 | No `require_backed_market` after seize | low | accepted residual (INV-LIQ-04 NOT ENFORCED); recapitalize hatch |
| A027-4 | Same-market absorb-then-full-writedown | info/low | accepted design (ADR-0021 deferred); harness pin |
| A027-5 | Liq path position write then delete | info | defended (indexer event order) |
| A027-6 | Missing usage row no-op on exit | medium (inherited) | partial — owned by A080; cleanup still deletes account |
| A027-7 | Empty-market index writedown no-op while debt burns | info | defended (Certora `bad_debt_writedown_is_noop_on_empty_market`); loss already outside share claims |

No finding of: orphaned `AccountMeta` with live NFT, surviving position maps after successful cleanup, cross-market index clobber, or controller writing `PoolKey` / pool writing `ControllerKey` account trees.

---

## Verdict

`execute_bad_debt_cleanup`’s storage footprint is defended: spoke-usage exits persist after a successful pool seize batch; all four account persistent keys and the position NFT are removed together; pool mutations are confined to seized hubs’ `PoolKey::State` with directed rounding and a non-zero supply-index floor. Residuals that remain are the documented post-seize solvency gap, the deferred same-market netting design, and the shared `apply_exit` missing-row behavior—not missing or duplicated durable writes inside the cleanup body.
