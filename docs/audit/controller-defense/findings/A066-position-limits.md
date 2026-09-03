# A066 — Position limits (max supply/debt slots)
- Agent: A066
- Theme: T4 (untrustworthy input validation); T1 adjacency (INV-RISK-04 / DoS.5)
- Severity: info (primary defense holds); low residual for `swap_debt` same-slot refinance UX at cap
- Status: defended
- Paths:
  - `contracts/controller/src/risk/validation.rs:63-116` (`validate_bulk_position_limits`)
  - `contracts/controller/src/positions/mod.rs:306-327` (`validate_position_entry_gates`)
  - `contracts/controller/src/positions/liquidation/apply.rs:285-309` (`require_credit_position_limit`)
  - `contracts/controller/src/config/registry.rs:45-62` / `governance.rs:12-25` (`set_position_limits`, constructor defaults)
  - `contracts/controller/src/storage/protocol.rs:86-99` (`get_position_limits` / `set_position_limits`)
  - `common/src/constants/shared.rs:41-42` (`POSITION_LIMIT_MAX = 5`)
  - `common/src/types/controller.rs:189-195` (`PositionLimits`)
  - Callers: `positions/supply.rs` (`process_deposit`), `positions/debt.rs` (`process_borrow`, `borrow_into_controller`), strategies via those helpers, Credit seize via `require_credit_position_limit`
- Defense: Every path that can **open** a durable supply or borrow map key runs `validate_bulk_position_limits` (directly or via `validate_position_entry_gates` / `require_credit_position_limit`). The check counts only hub assets **not already present**, deduplicates within the candidate list, admits pure top-ups even when `current_count > max` after a governance cut, and panics `#109 PositionLimitExceeded` otherwise. Governance cannot set 0 or `> POSITION_LIMIT_MAX`. Constructor seeds both sides to 5. Zero-scaled rows are removed from maps (`update_or_remove_*`), so ghost slots cannot accumulate.
- Gap: (1) **`swap_debt` borrows the new asset before repaying the old**, so a full refinance into a **fresh** debt hub at `max_borrow_positions` reverts even though the subsequent repay would free the old slot — unlike `swap_collateral`, which withdraws first and is harness-proven to free a supply slot at cap. Workaround: repay then borrow, or top up an already-held debt asset. (2) No post-persist `len() <= max` assert (defense is single pre-mutate gate). (3) `HubAssetKey` identity means same SAC under two hubs consumes two slots (by design). None of these allow durable over-cap books or fund theft.
- Impact: Accounts cannot hold more than `max_supply_positions` / `max_borrow_positions` (each `∈ 1..=5`) distinct supply/borrow keys. Liquidation resource budgets and DoS.5 sizing stay valid. Lowering limits never strands exits, top-ups, or Credit seizure of an asset the receiver already holds. Blast radius if the gate were missing: unbounded account maps → liquidation / view budget exhaustion (STRIDE DoS.5 Medium inherent); **with the gate: residual Low**.
- Evidence: INV-RISK-04; STRIDE DoS.5 / R.1; endpoints.md supply third-party slot rule; unit `contracts/controller/tests/validation.rs`; harness `position_limit_lowering_keeps_topups.rs`, `borrow.rs::test_borrow_position_limit_exceeded`, `supply.rs::test_supply_position_limit_exceeded`, `liquidation_seize_modes.rs::a_receiver_at_the_supply_position_limit_reverts`, `strategy/edge/rejections.rs::test_swap_collateral_full_close_frees_slot_at_max_positions`, multiply/borrow limit rejections; Certora `supply_position_limit_enforced`, `borrow_position_limit_enforced`, bulk duplicate/distinct rules, `supply_topup_survives_lowered_limit` (+ fixture `POSITION_LIMIT_MAX == 5` compile assert). Agrees with A012 (third-party cannot open slots), A013/A052 (Credit limit before mutate), A048 (swap_collateral free-slot order), A062 (dedup belt-and-suspenders), A070 (flash list length uses same cap as hygiene, not slot math).
- Opinion: Core cardinality defense is correct and well-tested, including the GH-16 top-up-after-lower semantics. The only noteworthy residual is the **borrow-first vs withdraw-first** asymmetry between `swap_debt` and `swap_collateral` at the borrow cap — liveness/UX, not an invariant break. Prefer documenting it next to `swap_debt` in endpoints.md rather than reordering legs (borrow-first is load-bearing for the refinance cash flow).

---

## 1. Scope

Inventory every controller surface that can **add a key** to `account.supply_positions` or `account.borrow_positions`, and ask:

1. Does it call `validate_bulk_position_limits` (or an equivalent hard bound) **before** the map grows?
2. Does the check count **new** slots only (top-ups free)?
3. Can governance / strategies / liquidation bypass or temporarily persist past `max_*`?
4. What happens when the limit is **lowered** below an account’s current count?

Out of scope except as cross-links: raw payment Vec length before aggregate (A062), flash `refund_assets` length hygiene (A070), third-party auth for new supply slots (A012), delegate count `MAX_DELEGATES` (same INV-RISK-04 bullet, different primitive).

---

## 2. Primitive — `validate_bulk_position_limits`

```63:116:contracts/controller/src/risk/validation.rs
/// Panics with `PositionLimitExceeded` if adding `aggregated`'s new
/// positions to `account` would exceed the configured max position count
/// for `position_type`. Counts only hub assets not already present in the
/// account, deduplicated within `aggregated`.
pub(crate) fn validate_bulk_position_limits(...) {
    let limits = storage::get_position_limits(env);
    let (current_count, max_allowed) = match position_type { Deposit => supply len / max_supply, Borrow => debt len / max_borrow };

    // seen Map: in-list dedup
    // already_present ⇒ do not increment new_positions_count
    if new_positions_count == 0 { return; }  // pure top-up / empty list

    total = current_count + new_positions_count  // checked_add → MathOverflow
    assert total <= max_allowed  // else PositionLimitExceeded #109
}
```

| Property | Behavior |
|---|---|
| Unit of cardinality | Distinct `HubAssetKey` in the durable map (`hub_id` + `asset`) |
| In-list duplicates | Skipped via `seen` Map (belt-and-suspenders; callers usually pass aggregated Vecs) |
| Already held | Not counted toward `new_positions_count` |
| Pure top-up when `current > max` | Early return — **admitted** (GH-16 / INV-RISK-04) |
| Empty `aggregated` | `new_positions_count == 0` → noop |
| Overflow on `current + new` | `GenericError::MathOverflow` (#33) |
| Missing storage | `get_position_limits` panics `#29 PositionLimitsNotSet` (fail closed) |

Wrapped by `validate_position_entry_gates`, which runs the bulk limit **first**, then `require_can_supply` / `require_can_borrow` per leg (`positions/mod.rs:308-327`).

---

## 3. Configuration surface

| Layer | Rule |
|---|---|
| Constant | `POSITION_LIMIT_MAX = 5` (`common/src/constants/shared.rs`) |
| Constructor | Both maxes set to `POSITION_LIMIT_MAX` via `registry::set_position_limits` (`governance.rs:19-25`); contract starts paused |
| Admin setter | `#[only_owner] set_position_limits` → `1..=POSITION_LIMIT_MAX` both sides or `#36 InvalidPositionLimits` (`registry.rs:48-55`) |
| Governance | `AdminOperation::SetPositionLimits` re-validates the same domain at propose (`governance/src/validate/asset.rs:25-34`) |
| Storage | Instance key `ControllerKey::PositionLimits`; read panics if unset |
| Certora guard | `const _: () = assert!(POSITION_LIMIT_MAX == 5)` in `certora/controller/spec/fixture.rs` |

Lowering is intentional and live: accounts already above the new max keep their books; they may exit, top up held assets, and receive Credit seizure of held assets; they may **not** open new keys. Harness: `position_limit_lowering_keeps_topups.rs`, `governance_change_between_legs.rs::lowering_the_position_limit_between_two_opens_blocks_only_the_new_slot`.

Raising `POSITION_LIMIT_MAX` in code requires re-verifying worst-case liquidation budget (STRIDE DoS.5 R.2; numeric-bounds §6.1 `N <= POSITION_LIMIT_MAX`).

---

## 4. Call graph — who opens slots?

### 4.1 Supply map (`AccountPositionType::Deposit`)

| Path | Gate | Notes |
|---|---|---|
| `supply` → `process_deposit` | `validate_position_entry_gates(Deposit)` | After third-party existing-slot rule (A012) |
| `multiply` → `process_deposit` | same | Collateral deposit after borrow/swap |
| `swap_collateral` → withdraw then `process_deposit` | same **after** source withdraw | Full close frees a slot before check (tested) |
| `migrate_from_blend` → `deposit_withdrawn` → `process_deposit` | same | Multi-asset deposit Vec from measured Δ |
| `flash_position` | `validate_collaterals` → entry gates **pre**-callback; `process_deposit` **post**-callback | Double check; flash guard blocks reentry that could mutate books mid-flight |
| Credit seize → `credit_supply_shares` | `require_credit_position_limit` **before** mutate | Builds aggregated from seize entries with `credited_shares > 0`; liquidator can `Credit(0)` |

`get_or_create_supply_position` is **read-only** on the map (returns ephemeral zero-scaled seed). Durable insert only via `update_or_remove_supply_position` when `scaled_amount != 0` after pool/credit merge.

### 4.2 Borrow map (`AccountPositionType::Borrow`)

| Path | Gate | Notes |
|---|---|---|
| `borrow` → `process_borrow` | `validate_position_entry_gates(Borrow)` | Pre-pool |
| `borrow_into_controller` | same | Used by multiply, swap_debt, flash_position, migrate |
| Repay / liquidation debt burn | none needed | Exit only; `update_or_remove_debt_position` drops zero |

### 4.3 Paths that do **not** open controller slots

- Transfer-mode liquidation (tokens leave to liquidator; no receiver supply map write).
- `withdraw` / `repay` / bad-debt cleanup / protocol fee seize from liquidated account.
- Flash `refund_assets` (balance Δ to caller; length capped at `max_supply_positions` for DoS hygiene only — A070).

No open path was found that mutates supply/debt maps for a **new** key without going through `validate_bulk_position_limits`.

---

## 5. Strategy ordering asymmetries

### 5.1 `swap_collateral` — withdraw-first (slot-friendly)

```58:77:contracts/controller/src/strategies/swap_collateral.rs
let swapped_amount = withdraw_and_swap_from_supply(...);  // may remove `current`
supply::process_deposit(..., &[(new, swapped_amount)], ...);  // limit sees freed slot
```

Harness `test_swap_collateral_full_close_frees_slot_at_max_positions`: at `max_supply_positions = 4`, full USDC→DAI rotation succeeds; partial open of a fifth asset at cap fails (`POSITION_LIMIT_EXCEEDED` in adjacent rejection test).

### 5.2 `swap_debt` — borrow-first (cap friction)

```59:90:contracts/controller/src/strategies/swap_debt.rs
borrow_into_controller(... new_debt ...);  // validate while old debt still held
repay_debt_from_controller(... existing_debt ...);
```

At `current_count == max_borrow_positions`, opening a **new** debt hub fails even if the repay would fully close `existing_debt`. Top-up of an already-held `new_debt` still works (`new_positions_count == 0`). Cross-hub same-SAC refinance (A047) still opens a second `HubAssetKey` and hits the same rule.

**Residual:** low liveness/UX — user can `repay` then `borrow` in separate txs, or keep borrowing the same hub. Not a stuck-funds or over-cap persistence bug. No dedicated harness asserting “full debt swap frees slot at max” (contrast supply). Document as intentional cash-flow order unless product wants a repay-first variant.

### 5.3 Multiply / flash_position / migrate

- Multiply: borrow gate then deposit gate independently — can hit either cap.
- Flash position: collateral list also `len() <= max_supply_positions` (input hygiene) **and** entry-gate slot math; debt via `borrow_into_controller`.
- Migrate: unique debt assets required; deposits go through `process_deposit` once measured Δs are known — over-cap multi-asset migrate reverts rather than truncating.

---

## 6. Credit-mode liquidation

```285:309:contracts/controller/src/positions/liquidation/apply.rs
/// The limit is enforced rather than bypassed because the liquidator chooses
/// the receiver: a revert is actionable (Credit(0)), whereas letting the
/// bound be exceeded would grow accounts past the size the worst-case
/// liquidation resource budget is sized for.
```

- Aggregates only entries with positive `credited_shares` (protocol-fee-only / dust credit does not invent slots).
- In-list dedup inside `validate_bulk_position_limits`.
- Held asset Credit after limit lower: admitted (harness in `position_limit_lowering_keeps_topups.rs`).
- Receiver already at supply cap on a **different** asset: `#109`; fresh `Credit(0)` succeeds (`liquidation_seize_modes.rs`).

Transfer seize never consults this gate (correct).

---

## 7. Interaction with other defenses

| Concern | Interaction |
|---|---|
| A012 third-party supply | Stranger cannot open slots; limit check still runs for owners on new keys |
| A040 / A064 listing & freeze | Entry gates after limit; delisted exits do not need new slots |
| A062 Vec duplicates | Payments **sum** duplicates; limit counts unique keys once |
| A070 flash refund length | Reuses `max_supply_positions` as list cap — not slot accounting |
| A048 swap_collateral | Free-slot order verified |
| Ghost / dust rows | `update_or_remove_*` removes `scaled == 0`; restamp/finalize cannot leave empty keys that inflate `len()` |
| Post-pool solvency | Orthogonal — HF/LTV after money move; does not replace cardinality |

---

## 8. Attack / misuse cases

| Scenario | Outcome |
|---|---|
| Bulk supply of `max+1` distinct new hubs | `#109` before pool |
| Duplicate legs for one new hub at `max-1` | Counts as one; admitted (unit + Certora bulk duplicate rule) |
| Two distinct new hubs at `max-1` | `#109` (Certora `bulk_*_exceed_limit_reverts`) |
| Top-up after governance lowers below count | Passes (unit + Certora `supply_topup_survives_lowered_limit` + harness) |
| New slot after lower | `#109` |
| Third party opens new supply hub | `#NotAuthorized` (A012) before/alongside limit |
| Credit into full receiver, new asset | `#109`; `Credit(0)` workaround |
| `set_position_limits(0, _)` or `>5` | `#36` at admin / governance validate |
| Unset limits storage | `#29` on first open attempt |
| Same SAC, two hubs | Two slots (key identity); flash collaterals additionally reject duplicate **Address** |
| Oversized raw payment Vec | Pre-aggregate CPU DoS only (A062); cannot create >5 durable slots |

---

## 9. Evidence matrix

| Claim | Artifact |
|---|---|
| INV-RISK-04 ENFORCED | `docs/reference/invariants.md` INV-RISK-04 |
| Unit over-cap / top-up / dedup | `contracts/controller/tests/validation.rs` |
| Harness supply/borrow over-cap | `tests/.../controller/supply.rs`, `borrow.rs`, `meta/invariant.rs` |
| Lowered limit top-up + Credit | `position_limit_lowering_keeps_topups.rs` |
| Credit receiver at cap | `liquidation_seize_modes.rs` |
| Swap collateral frees slot | `strategy/edge/rejections.rs` |
| Multiply respects both caps | `strategy/edge/multiply.rs`, `rejections.rs` |
| Admin domain | `governance/admin_config.rs`, controller `admin_config.rs` |
| Formal | `certora/.../solvency_rules.rs`, `spoke_rules.rs` position-limit family |
| Threat | STRIDE DoS.5; numeric-bounds liquidation `N≤5` |

---

## 10. Gaps / residuals (non-critical)

1. **`swap_debt` at borrow cap cannot atomically replace a debt hub** — borrow-first ordering; documented product asymmetry vs `swap_collateral`. Severity: low (UX). Status: accepted residual unless endpoints docs call it out.
2. **No post-persist cardinality assert** — single pre-gate; acceptable given all open paths share the helper and tx atomicity.
3. **Raw mutator Vec length uncapped** — A062; does not defeat slot math.
4. **Historical Certora vacuity (F10)** when fixtures assumed limit 10 after constant moved to 5 — closed by fixture compile assert and rebased rules (`docs/explanation/certora-suite-review-2026-09-03.md`); treat as process lesson, not live hole.

---

## 11. Verdict

`validate_bulk_position_limits` correctly enforces INV-RISK-04 for both supply and borrow cardinality across ordinary verbs, strategies, and Credit liquidation. Top-up-after-lower semantics are intentional and verified. The defense is **defended**; the only actionable documentation/product note is the `swap_debt` vs `swap_collateral` free-slot ordering asymmetry at the respective caps.
