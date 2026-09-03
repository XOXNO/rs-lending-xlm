# A096 — Account load shapes (borrow-only / supply-only / full) vs `PositionSides` persist

- Agent: A096
- Theme: T6 (read/write savings; load-bearing coupling to T2 storage writes)
- Severity: low
- Status: defended
- Paths:
  - `contracts/controller/src/storage/account.rs:13-26` (`account_from_parts`)
  - `contracts/controller/src/storage/account.rs:71-103` (`set_*_positions` / `write_side_map` empty-key delete)
  - `contracts/controller/src/storage/account.rs:142-168` (`get_account` / `try_get_account` / `get_account_borrow_only`)
  - `contracts/controller/src/positions/mod.rs:206-252` (`PositionSides`, `persist_account_positions`, `finalize_position_flow`)
  - `contracts/controller/src/positions/mod.rs:96-104` (`enforce_post_pool_solvency` restamp + gate)
  - `contracts/controller/src/positions/debt.rs:35-112` (borrow: full load + conditional `Both`; repay: borrow-only + `Debt`)
  - `contracts/controller/src/positions/supply.rs:44-77` (supply: `load_or_create` full + `Supply`, `remove_if_empty=false`)
  - `contracts/controller/src/positions/supply.rs:161-189` (withdraw: full + `Supply`, `remove_if_empty=true`)
  - `contracts/controller/src/account.rs:88-114` (`load_or_create_account` → `get_account`)
  - `contracts/controller/src/account.rs:170-176` (`cleanup_account_if_empty` via `Account::is_empty`)
  - `common/src/types/controller.rs:315-365` (`Account`, `is_empty`, `debt_free`)
  - `contracts/controller/src/strategies/mod.rs:71-79` (`strategy_finalize` → `Both`, `remove_if_empty=true`)
  - `contracts/controller/src/strategies/{multiply,flash_position,migrate_blend,swap_collateral,swap_debt,repay_debt_with_collateral}.rs` (full `get_account` / `load_or_create`)
  - `contracts/controller/src/positions/liquidation/mod.rs:46-148` (victim full + `Both`; Credit receiver full/create + `Supply`)
  - `contracts/controller/src/positions/liquidation/apply.rs:323-340` (post-liq `cleanup_account_if_empty` on in-memory victim)
  - `contracts/controller/src/keepers.rs:143-237` (`sync_account_thresholds`: supply-first / optional debt / `set_supply_positions` only)
  - `contracts/controller/src/risk/params.rs:11-35,66-80` (`RiskRefreshScope` vs `account.debt_free()` gating)
  - `contracts/controller/src/risk/validation.rs:31-34` (post-pool gates no-op when `debt_free`)
  - `contracts/controller/src/views.rs:28-42,105-122,244-288` (full vs one-sided **read** shapes; no persist)
  - `contracts/controller/src/context/events.rs:48-61` (`emit_position_batch` uses attributes + buffers, not in-memory maps)
  - `certora/controller/spec/health_rules.rs:126-161` (`assert_gate_observation_is_final` / `restamped → PositionSides`)
- Defense: Three production load shapes exist. Each mutator that persists pairs the in-memory maps with `PositionSides` (and `remove_if_empty`) so that (a) a map that was never loaded cannot be written, (b) a sibling map that was loaded but not mutated is either rewritten identically (`Both`) or left untouched (one-sided persist), and (c) empty-account NFT burn only runs when `Account::is_empty` observes **both** durable sides. `write_side_map` deletes a key on empty map, which makes a mismatched persist a full-side wipe rather than a no-op. Current call sites obey the pairing. Views and keeper `LtvOnly` use one-sided **reads** and never call `persist_account_positions`.
- Gap: Pairing is convention, not a type. There is no `get_account_supply_only` helper and no newtype that would make `PositionSides::Both` after a borrow-only `Account` a compile error. Unit tests cover empty-map key deletion and `Both` persist, not the repay wipe / keeper dual-wipe regressions. Residual empty-shell rent on borrow-only repay (`remove_if_empty=false`) is A036 / A025, not a load-shape bug. Certora `get_position` Borrow-arm zero-fill (A035) is a verification epistemology issue, not production WASM.
- Impact: **No live fund-control bug** on current graphs. Hypothetical mismatch blast radius is **Critical per account**: `PositionSides::Both` or `Supply` after borrow-only load deletes `SupplyPositions` while pool still credits shares; `remove_if_empty=true` after any one-sided load can burn the NFT and `remove_account_entry` (all four keys) while the unloaded side (and pool) still holds value (INV-STOR-03). Dual: `PositionSides::Both` after keeper `LtvOnly` (empty debt in RAM) would delete `BorrowPositions` while pool debt remains. `restamped → Debt` (skipping `Both`) after a full-load solvency restamp would leave gate-observed LTV off disk (TOB-AAVE-7 class; Certora fence). Observed residual today is rent on emptied-debt shells and review risk on future PRs.
- Evidence: Exhaustive production grep of `get_account`, `try_get_account`, `get_account_borrow_only`, `account_from_parts`, `get_supply_positions`/`get_debt_positions` assembly, `PositionSides`, `persist_account_positions`, `finalize_position_flow`, `set_supply_positions`/`set_debt_positions`. INV-STOR-01, INV-STOR-03. Peers A021 §4.3–4.5, A022–A026, A032, A025 triad, A023 restamp coupling, A036 cleanup, A035 harness, A104 hole this file closes. Unit `set_supply_positions_empty_map_removes_key`, `persist_account_positions_writes_both_sides`, `persist_account_positions_removes_empty_account`. Harness `test_account_auto_removed_after_full_repay_withdraw` (cleanup on later full-load withdraw, not on repay). Health rule `assert_gate_observation_is_final`.
- Opinion: Treat load-shape × `PositionSides` × `remove_if_empty` as one invariant, not three independent optimizations. Current code is **defended**. Do not add `get_account_supply_only` unless persist is locked to `Supply` and `remove_if_empty` is forced false (or debt is loaded whenever cleanup is true). Do not “save a supply read” on repay by writing `Both`. Do not flip repay `remove_if_empty` without loading supply. Do not call `persist_account_positions(..., Both, _)` from `sync_account_thresholds`. A typed `LoadedAccount<Sides>` would be the only structural upgrade worth considering; it is not required to close a live gap.

---

## Method

1. Read `shared/COORDINATION.md`, `SEED.md`, `AGENT_MANIFEST` Wave 6 A096, README finding format, peers **A021–A026**, **A032**, **A036**, **A035**, **A088** (Wave-6 format), **A104** (explicit A096 hole).
2. Inventory every production assembler of `Account` under `contracts/controller/src` and every writer of supply/debt maps.
3. Classify each as full / borrow-only / supply-shaped / view-only / create-empty, then pair with `PositionSides` and `remove_if_empty`.
4. Enumerate **forbidden** combinations and check none are reachable on current call graphs.
5. Check `debt_free` / `is_empty` consumers that would mis-fire on a hollow sibling map (`require_post_pool_risk_gates`, `apply_gated_liquidation_params`, liquidation post-cleanup).
6. Check events: `emit_position_batch` does not persist maps from the hollow `Account`.
7. No production Rust edited. No git operations.

No novel Critical/High on live paths. Agrees with A021/A025 on the repay triad; generalizes the dual (supply-shaped load) that those notes only mention in passing. Fills A104 §7 A096 hole.

---

## 1. Why load shapes exist (T6)

Account state is four persistent keys (`AccountMeta`, `SupplyPositions`, `BorrowPositions`, `Delegates`) plus NFT `owner_of` (A021). Assembling a full `Account` is three user-storage reads plus one cross-contract owner read.

`write_side_map` (`storage/account.rs:90-103`) **replaces or deletes the entire side**. There is no patch API. Therefore:

- Loading a side you will not write is wasted decode (T6).
- Writing a side you did not load is not “skip”; it is **clobber with whatever is in RAM**, including a freshly allocated empty `Map`.

The optimization is therefore only safe when persist sides ⊆ loaded sides, and `remove_if_empty` is false unless both sides were loaded (or the unloaded side is known empty, which no production path proves without loading it).

---

## 2. Assemblers (the three shapes)

### 2.1 Full — `get_account` / `try_get_account`

```149:157:contracts/controller/src/storage/account.rs
pub(crate) fn try_get_account(env: &Env, account_id: u64) -> Option<Account> {
    let meta = try_get_account_meta(env, account_id)?;
    let owner = try_account_owner(env, account_id)?;
    Some(account_from_parts(
        owner,
        meta,
        get_supply_positions(env, account_id),
        get_debt_positions(env, account_id),
    ))
}
```

Missing meta or NFT owner → `None` / `AccountNotFound`. Both maps default to empty if the key is absent (legitimate empty side, not a hollow skip).

Used by: withdraw, borrow, supply/`load_or_create` (existing id), all account-touching strategies, liquidation victim + Credit receiver, bad-debt socialization, health/liquidation views that need both legs.

Create path (`create_account_with`) starts with **both maps empty in RAM and on disk** (meta + NFT only). Persist `Supply` or `Both` cannot wipe a sibling that does not exist yet.

### 2.2 Borrow-only — `get_account_borrow_only`

```163:168:contracts/controller/src/storage/account.rs
pub(crate) fn get_account_borrow_only(env: &Env, account_id: u64) -> Account {
    let meta = get_account_meta(env, account_id);
    let owner = account_owner(env, account_id);
    let borrow_positions = get_debt_positions(env, account_id);
    account_from_parts(owner, meta, Map::new(env), borrow_positions)
}
```

**Sole production caller:** `process_repay` (`debt.rs:90`).

Saves the supply-map read. Does **not** save NFT `owner_of` (still required for existence / event attributes). Error codes differ from full load: missing meta panics `AccountNotInMarket` via `get_account_meta`; missing owner panics `AccountNotFound`. Observability asymmetry only.

In RAM, `supply_positions` is always empty **regardless of durable collateral**. `Account::is_empty` and `debt_free` therefore **lie about supply**. They remain truthful about debt.

### 2.3 Supply-shaped — no named helper

There is **no** `get_account_supply_only`. The shape is assembled by hand:

| Site | Supply map | Debt map | Persist |
|---|---|---|---|
| `sync_account_thresholds` `LtvOnly` | `get_supply_positions` | `Map::new` | `set_supply_positions` only if changed |
| `sync_account_thresholds` `FullTuple` | `get_supply_positions` | `get_debt_positions` | supply write only; debt is **read** for HF / gated params |
| `total_collateral_in_usd` | `get_supply_positions` | empty, passed into risk totals | **none** (view) |
| `total_borrow_in_usd` | unused | `get_debt_positions` | **none** (view) |

Keeper skips NFT `owner_of` until after the empty-supply early return (`keepers.rs:157-167`) — a real T6 save repay does not make.

`PositionSides::Supply` after a **full** load (withdraw, supply, Credit receiver) is **not** this shape: debt is present in RAM and simply not written. That is safe.

---

## 3. Persist primitive (the other half)

```208:235:contracts/controller/src/positions/mod.rs
pub(crate) enum PositionSides {
    Supply,
    Debt,
    Both,
}

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

Facts:

1. `Debt` skips supply write; `Supply` skips debt write; `Both` writes both.
2. Comment “every variant writes at least one side” is true; it does **not** say the written side was loaded.
3. Empty in-memory map → `persistent.remove` of that key (`write_side_map`). Unit tests `set_supply_positions_empty_map_removes_key` / `set_debt_positions_empty_map_removes_key` make the wipe mechanical.
4. `cleanup_account_if_empty` uses `account.is_empty()` (both maps empty in **this** `Account`), then `remove_account_and_burn_nft` (all four keys + NFT). INV-STOR-03.

`finalize_position_flow` always: `persist_spoke_usage` → `persist_account_positions` → `emit_position_batch` (A032 / A033 / A078). Usage persist is independent of position sides.

`emit_position_batch` publishes **buffered deltas** plus `account_attributes` (`spoke_id`/`mode` from meta). It does not dump the in-memory maps. Hollow supply on repay does not emit a fake “all collateral closed” batch.

Keeper is the only mutator that writes a side map **without** `persist_account_positions` (`set_supply_positions` + `renew_user_account` earlier + `emit_position_batch`). That bypass is load-bearing: it cannot accidentally pass `PositionSides::Both`.

---

## 4. Call-site matrix (production)

| Path | Load | `PositionSides` / writer | `remove_if_empty` | Why this pairing |
|---|---|---|---|---|
| `process_supply` | full (`load_or_create`) | `Supply` | **false** | Must see existing supply slots (INV-AUTH-03 third-party); never empties; writing `Both` would only rewrite loaded debt (safe, wasteful); `remove_if_empty=true` would be wrong on a hollow-debt future refactor but is harmless today because debt was loaded |
| `process_withdraw` | full | `Supply` | **true** | HF/restamp need debt in RAM; persist supply only (debt unchanged); `is_empty` sees real debt so NFT burn cannot strand pool debt |
| `process_borrow` | full | `Debt` or **`Both` if restamped** | false | Solvency restamps supply LTV in RAM; must land on disk iff changed (A023 / TOB-AAVE-7). Cannot empty an account |
| `process_repay` | **borrow-only** | **`Debt`** | **false** | **Mandatory triad** (A025). Any other persist/cleanup choice is Critical |
| `strategy_finalize` | full (`get_account` / `load_or_create`) | `Both` | **true** | Both legs move; restamp always; cleanup after full close |
| Liq victim | full | `Both` | **false** (cleanup post-step) | Seize mutates supply; repay mutates debt; empty/bad-debt is `check_bad_debt_after_liquidation` (A026 vs A036 shorthand) |
| Liq Credit receiver | full or create-empty | `Supply` | false | Only supply shares move onto receiver; writing `Debt`/`Both` with a **create-empty** receiver would be OK (no debt); writing `Both` after a **mistaken borrow-only** receiver load would wipe receiver debt — current load is full |
| Bad-debt / `remove_account_and_burn_nft` | full (for totals) then delete | n/a | forced four-key remove | Not a side persist |
| `update_account_threshold` | supply-shaped ± debt | `set_supply_positions` only | n/a | Must not persist hollow debt. `FullTuple` loads debt so `apply_gated_liquidation_params` sees `debt_free()` truthfully |
| Views (`health_factor`, estimate, LTV collateral) | full `try_get_account` | none | n/a | HF / plan need both maps |
| Views (`total_collateral_in_usd`) | supply + empty borrow **into risk only** | none | n/a | Explicit comment: cheaper market fetch; not a persist path |
| Views (`total_borrow_in_usd`) | debt only | none | n/a | `sum_debt_usd`; not HF |
| `flash_loan` | no account | none | n/a | Pool-internal (A032 exception) |
| `recapitalize` / `update_indexes` / `claim_revenue` | no account maps | none | n/a | Market-level |

**No production path** calls `get_account_borrow_only` except repay. **No production path** calls `persist_account_positions` with `Both` or `Supply` on a borrow-only `Account`. **No production path** sets `remove_if_empty=true` on a hollow sibling.

---

## 5. Forbidden pairings (blast radius)

Let `S` / `D` mean “this side was loaded from disk (or created empty for a new id)”. Hollow means “RAM map is empty **without** having read disk.”

| RAM | Persist | `remove_if_empty` | Outcome |
|---|---|---|---|
| Hollow supply, real debt | `Debt` | false | **Live repay. Safe.** May leave empty-debt shell if supply exists on disk (A036 rent) or if supply was already empty (shell + NFT until a later full-load cleanup) |
| Hollow supply, real debt | `Debt` | **true** | After full repay, `is_empty` true → **burn NFT + delete supply key** while pool still holds collateral. **Critical** |
| Hollow supply | `Supply` or `Both` | any | **Deletes `SupplyPositions`.** Pool shares orphaned from controller book. **Critical** |
| Hollow debt, real supply (keeper `LtvOnly`) | `set_supply_positions` only | n/a | **Live keeper. Safe.** |
| Hollow debt | `Both` or `Debt` | any | **Deletes `BorrowPositions`.** Controller forgets debt; pool still owed. Protocol insolvency / unliquidatable hole. **Critical** |
| Hollow debt | `Supply` + `remove_if_empty=true` | true | If supply also emptied in RAM, **burn NFT while disk/pool debt remains**. **Critical** |
| Full, restamp mutated supply | `Debt` only | false | Gate observed new LTV; disk keeps old LTV. Later loads **overstate** collateral vs admission. TOB-AAVE-7 class. **High** (A023). Live borrow uses `Both` when `restamped` |
| Full, withdraw | `Supply` + true | true | **Live withdraw. Safe.** `is_empty` sees remaining debt |
| Full, strategy | `Both` + true | true | **Live. Safe.** |
| Full, supply | `Supply` + false | false | **Live. Safe.** |
| Create-empty | `Supply` or `Both` | false/true | Safe: both sides truly empty on disk |

These rows are the review checklist. A096’s job is to confirm the live table in §4 never hits a red row. It does not.

---

## 6. `debt_free` / `is_empty` on hollow maps

Several gates key off the in-memory `Account`, not a fresh disk read:

1. **`require_post_pool_risk_gates`** — no-op if `debt_free()`. Borrow-only + leftover debt: `debt_free` false, but supply hollow ⇒ HF ≈ 0 ⇒ **would revert a partial repay** if solvency were added without loading supply. Skipping solvency on repay is therefore coupled to the load shape (A025, A072). Not a gap; a load-bearing absence.

2. **`apply_gated_liquidation_params`** — liquidator-favoring LT/bonus/fee updates are skipped when `!account.debt_free()` unless hypothetical HF still clears 1.05 WAD. If `FullTuple` ran with a hollow debt map, `debt_free()` would be **true**, the skip would not apply, and harsher liquidation params could stamp onto supply **while real debt exists**. Keeper pairs `FullTuple` with `get_debt_positions` (`keepers.rs:169-174`). `LtvOnly` never enters `apply_gated_liquidation_params` (`risk/params.rs:31-33`). **Defended.** Desync of `has_risks` vs load is the dual of the repay triad.

3. **Liquidation `check_bad_debt_after_liquidation`** — `cleanup_account_if_empty` only if `borrow_positions.is_empty()` then `is_empty`. Victim was full-loaded and both sides persisted `Both` just before, so RAM matches disk for this account. **Defended** (A026/A027).

4. **Views `health_factor`** — uses `try_get_account` (full). A supply-only view is **not** used for HF. `total_collateral_in_usd` passing an empty borrow map into `calculate_account_risk_totals` is documented as discarding debt arithmetic; it returns collateral USD, not HF.

---

## 7. Asymmetries that look like bugs and are not

### 7.1 Why repay is the only borrow-only load

Repay is permissionless, risk-reducing, and does not restamp LTV. Loading supply would (a) cost a map decode, (b) invite a `Both` persist “for symmetry,” (c) invite `remove_if_empty=true` “because the account might now be empty.” A025 already called that triad Critical to keep intact. A096 confirms no second borrow-only mutator has appeared (`grep get_account_borrow_only` → `debt.rs` + re-export only).

### 7.2 Why withdraw is full + `Supply`, not supply-only

Withdraw must `enforce_post_pool_solvency`. That restamps listed LTV and values **debt**. A supply-only load would make `debt_free()` true whenever RAM debt is empty → gates skipped → undercollateralized withdraw. Full load is a **security read**, not wasted T6. Persist stays `Supply` because debt shares did not change; restamped LTV still lands because supply is the written side (A024). Contrast borrow, which writes `Debt` by default and must widen to `Both` when restamp mutates supply (A023).

### 7.3 Why strategies always `Both`

Multi-leg flows mutate both maps in RAM (`process_deposit`, `merge_debt_leg`, withdraw-all, net-settle). One-sided persist would drop the other leg’s merge. `remove_if_empty=true` is safe only because load is full (A032).

### 7.4 Why Credit receiver is `Supply` not `Both`

Receiver debt is loaded (full `get_account`) but not mutated. Writing `Both` would be a redundant full-map rewrite of debt (safe). `Supply` avoids that write. `remove_if_empty=false` because the receiver just received shares.

### 7.5 Error-code split on borrow-only vs full

`get_account_borrow_only` uses `get_account_meta` (`AccountNotInMarket`) vs `get_account` (`AccountNotFound` when meta **or** owner missing). Same fail-closed existence; different code. Info-level API inconsistency, not a wipe vector.

### 7.6 A036 empty shell after repay

Borrow-only + `remove_if_empty=false` leaves `AccountMeta` (+ Delegates, NFT) after debt-key deletion even when supply was already empty on disk. A later withdraw/strategy/liq with full load + cleanup burns. Rent, not theft. **Do not “fix” by flipping the flag.**

---

## 8. Tests and rules

Present:

- Empty-map delete: `contracts/controller/tests/storage/account.rs` (`set_supply_positions_empty_map_removes_key`, `set_debt_positions_empty_map_removes_key`).
- Persist `Both` writes both; persist `Both` + empty + `remove_if_empty` deletes meta: `tests/positions/flags.rs`.
- Health fence that persisted book matches gate observation (catches `restamped`/`PositionSides` regression): `certora/controller/spec/health_rules.rs` `assert_gate_observation_is_final`.
- Harness cleanup after repay **then** withdraw, not on repay alone (A025 naming note).

Absent (review residual, not a live bug):

- No unit test: seed supply + debt, `get_account_borrow_only`, `persist_account_positions(..., Debt, false)`, assert supply key unchanged.
- No unit test: keeper `LtvOnly` hollow debt must not call `set_debt_positions`.
- No unit test: `remove_if_empty=true` + borrow-only panics-or-is-unreachable (would be a negative test of a forbidden API).
- No Certora rule that `process_repay` does not write `SupplyPositions` (health rules skip repay’s post-gate; `supply_preserves_frozen_valuation` checks debt unchanged on supply, not the dual).
- Controller unit tests never mention `get_account_borrow_only`.

A108 may pick these up as missing-tests; they are **regression fences**, not evidence of current failure.

Certora harness `get_position` Borrow-arm zero-filling supply risk fields (A035) can make a rule think a borrow-only view is a full book. That is **not** `get_account_borrow_only` in WASM. Do not cite green position rules as proof of the persist pairing.

---

## 9. Cross-links

| Peer | Relation |
|---|---|
| **A021** | Layout contract; §4.3 matrix that this note expands and re-verifies |
| **A025** | Repay triad; A096 owns the dual (supply-shaped / keeper) and the global forbidden table |
| **A023** | `restamped → Both`; full load is the prerequisite |
| **A022** / **A024** | Full load + `Supply`; withdraw `remove_if_empty=true` needs real debt in RAM |
| **A026** | Victim `Both` + full load; receiver `Supply` + full/create; `remove_if_empty=false` then post-step cleanup |
| **A032** | Strategy `Both` + true; flash_loan has no account |
| **A036** | Empty-shell rent from `remove_if_empty=false`; A026 correction on liq flag |
| **A035** | Harness zero-fill ≠ production load shape |
| **A072** | Post-pool gates skipped when `debt_free`; must not run on hollow supply with live debt |
| **A034** | `renew_user_account` still walks all four keys that **exist**; hollow RAM does not prevent renewing a live supply key on repay (renew is key-has, not map contents) |
| **A104** | This file closes the “A096 unfiled” hole; no new Wave-6 Critical |

**Agreements:** A021/A025/A023/A024/A026 on live pairings. A036 on rent. A104 that this was the remaining Wave-6 account-shape deep-dive.

**Disagreements:** None on severity. A036 header still says liquidation sets `remove_if_empty` “where appropriate”; A026/A096: liquidation finalize uses `false`. Factual nuance already logged in A109; not re-filed.

---

## 10. Residuals and review checklist

| Residual | Sev | Disposition |
|---|---|---|
| Convention-only pairing (no type) | info | Accept; PR review using §5 table |
| No `get_account_supply_only` | info | Prefer keeping it **absent** unless persist is locked |
| Missing regression tests for wipe duals | low (process) | Optional unit tests listed in §8 |
| Repay empty-shell rent | low | A036; do not flip flag |
| Error-code split borrow-only vs full | info | Cosmetic |
| Certora does not prove repay non-write of supply | info | A035 + §8 |

**PR nacks (any one is Critical unless the load is redesigned in the same patch):**

1. `finalize_position_flow(..., Both | Supply, _)` on `process_repay`.
2. `remove_if_empty=true` on `process_repay` without switching to full load.
3. Calling `enforce_post_pool_solvency` on the borrow-only `Account`.
4. `persist_account_positions(..., Both | Debt, _)` from `sync_account_thresholds`.
5. `RiskRefreshScope::FullTuple` without `get_debt_positions`.
6. New `get_account_supply_only` used by withdraw/strategy/liq cleanup.
7. Dropping borrow’s `restamped → Both` branch.
8. Switching withdraw to a supply-only load while keeping post-pool solvency.

---

## 11. Verdict

Account load shapes are a **deliberate T6 optimization** whose safety is entirely in the persist pairing. Live controller paths match loaded sides to written sides and refuse cleanup on hollow books. The only borrow-only mutator is repay; the only supply-shaped mutator writes supply via a dedicated setter, never `PositionSides::Both`. Blast radius of a future mismatch is full side-wipe or NFT unpairing for that account; current code does not have that bug.

Severity **low** (process residual: untyped convention + missing negative tests), status **defended**, not `optimization-note`: the pairing is a fund-control invariant that happens to also save reads.
