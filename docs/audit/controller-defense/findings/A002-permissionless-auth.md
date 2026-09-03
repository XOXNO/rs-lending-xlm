# A002 — Permissionless entrypoint claims vs body auth (supply / repay / liquidate)
- Agent: A002
- Theme: T1
- Severity: info
- Status: defended
- Paths: `scripts/permissionless_entrypoints.txt:63-65`; `contracts/controller/src/lib.rs:99-166`; `contracts/controller/src/positions/supply.rs:44-102,126-155`; `contracts/controller/src/positions/debt.rs:81-112,161-183`; `contracts/controller/src/positions/liquidation/mod.rs:46-216`; `contracts/controller/src/positions/liquidation/plan.rs:19-49`; `contracts/controller/src/positions/liquidation/apply.rs:31-83`; `contracts/controller/src/positions/liquidation/math.rs:465-491`; `contracts/controller/src/risk/validation.rs:12-15`; `contracts/controller/src/account.rs:28-77,88-143`; `common/src/token.rs:19-34`
- Defense: All three declared lines are `caller-auth` and reach `Address::require_auth` on the caller-supplied actor. Supply adds a third-party existing-slot gate; repay has no account-owner gate and only exits debt; liquidate is HF-gated and only binds receiver ownership in Credit seize mode.
- Gap: Justification text for `controller::supply` says “non-owner” cannot open a new slot; the body admits an active listed **delegate** to open slots (`is_owner_or_delegate`). Claim semantics match code; wording is slightly under-precise vs STRIDE Elevation.5. No auth bypass found.
- Impact: No foreign-risk creation path found on these three verbs. Blast radius of a stranger call is: (supply) collateral top-up on existing hubs only or self-owned new account; (repay) caller pays own tokens to reduce target debt; (liquidate) unhealthy account only, with Credit credits only to liquidator-controlled Normal accounts on the same spoke.
- Evidence: INV-AUTH-03, INV-ACCT-03, INV-LIQ-01, INV-LIQ-02; Certora `supply_new_slot_requires_owner_or_delegate`; harness `supply.rs`, `repay.rs`, `liquidation.rs`, `liquidation_seize_modes.rs`, `security_audit.rs`. Agrees with A012 on third-party supply slot rules.
- Opinion: Permissionless inventory claims for supply/repay/liquidate are faithful to body auth. Treat the “non-owner” wording as a docs nit, not a gate failure. Pause asymmetry (supply paused; repay/liquidate live) is intentional exit liveness, outside the three claim lines.

## Declared surface (source of truth)

From `scripts/permissionless_entrypoints.txt`:

| Line | Entrypoint | Category | Invariants | Claim summary |
|---|---|---|---|---|
| 63 | `controller::supply` | caller-auth | INV-AUTH-03, INV-ACCT-03 | Anyone may top up foreign accounts only on hubs already held; non-owner cannot open a new slot; `account_id == 0` creates an account owned by the caller |
| 64 | `controller::repay` | caller-auth | INV-AUTH-03, INV-ACCT-03 | Anyone may repay any account; funds from caller; measured receipt; liabilities only fall |
| 65 | `controller::liquidate` | caller-auth | INV-AUTH-03, INV-LIQ-01, INV-LIQ-02 | Anyone (incl. owner) may liquidate HF < 1; Credit receiver ≠ liquidated account; seizure coupled to repaid debt |

Category definition (same file, lines 21–26): `caller-auth` means the body requires `Address::require_auth` on a caller-supplied address; further owner/delegate/payer rules live in the body and must be stated in the justification.

INV-AUTH-03 (`docs/reference/invariants.md:43-52`): permissionless actions must not create an unwanted account slot or increase another user’s risk; third-party supply may only top up an existing supply position.

---

## 1. `controller::supply`

### Entrypoint wiring

```99:107:contracts/controller/src/lib.rs
    #[when_not_paused]
    fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> u64 {
        positions::process_supply(&env, &caller, account_id, spoke_id, &assets)
    }
```

Paused under `#[when_not_paused]` (risk-increasing / exposure-opening path). Auth is not in the wrapper; it is in `process_supply`.

### Body auth chain

1. **`validation::require_authorized_caller`** — `caller.require_auth()` plus flash-loan reentrancy gate (`risk/validation.rs:12-15`).
2. **`payments::aggregate_positive_payments`** — rejects empty/non-positive legs before account work.
3. **`account::load_or_create_account(..., AccountGuard::Supply, ...)`** (`account.rs:88-114`):
   - `account_id == 0` → `create_account` / `create_account_with` mints the position NFT to `caller` as owner (`account.rs:28-76`). Matches claim “account_id 0 creates an account owned by the caller.”
   - Existing id → **spoke match only**. Unlike `Migrate` / `Multiply`, `AccountGuard::Supply` does **not** call `require_owner_or_delegate`. That is what keeps third-party top-up permissionless.
4. **`require_third_party_existing_supply`** (`supply.rs:83-102`) — if `account_id != 0` and caller is neither owner nor active listed delegate, every aggregated hub must already be in `account.supply_positions` or panic `GenericError::NotAuthorized`.
5. **`settle_supply`** (`supply.rs:126-155`) — `transfer_amount_measured` from `caller` → pool (`common/src/token.rs:19-34` via `payments` re-export); pool supply credited with **measured** `received` (INV-ACCT-03).

### Claim checks

| Claim fragment | Verdict | Evidence |
|---|---|---|
| Category `caller-auth` | Match | `require_authorized_caller` → `caller.require_auth()` |
| INV-AUTH-03 third-party top-up only | Match | `require_third_party_existing_supply` |
| Non-owner cannot open new slot | **Partial wording mismatch** | Gate is `!is_owner_or_delegate` (`account.rs:118-130`), so an **active listed delegate** may open new hubs. Strangers cannot. STRIDE Elevation.5 states “non-owner, non-delegate”; the permissionless file says only “non-owner.” |
| `account_id == 0` → caller-owned | Match | `create_account` NFT mint to `owner`/`caller` |
| INV-ACCT-03 measured credit | Match | `transfer_amount_measured` then `make_pool_action(..., received, ...)` |
| Does not create foreign risk | Match | New slots blocked for strangers; top-up increases collateral (risk-reducing). LT/bonus/fee restamp is gated so a stranger cannot force a threshold cut that drops HF below the update floor (see harness `third_party_supply_and_risk_restamp.rs`, `security_audit.rs` H-RISK-03/04 sticky LT). |

### Formal / test evidence

- Certora `supply_new_slot_requires_owner_or_delegate` (`certora/controller/spec/market_guard_rules.rs:68-93`): non-owner, non-manager caller, missing hub → must revert (`cvlr_assert!(false)` after call).
- `tests/.../controller/supply.rs`: `test_third_party_supply_to_existing_account_succeeds`, `regression_non_owner_cannot_open_new_supply_slot_on_victim`.
- `tests/.../controller/security_audit.rs`: `regression_third_party_cannot_open_new_supply_slots`, `revalidation_third_party_can_top_up_only_existing_leg` (extended).

### Cross-check vs A012

A012 concludes the same slot rule and marks INV-AUTH-03 defended. A002 concurs; A002 additionally maps category/`require_auth`, INV-ACCT-03 measured receipt, pause flag, and the non-owner vs delegate wording nit.

---

## 2. `controller::repay`

### Entrypoint wiring

```137:141:contracts/controller/src/lib.rs
    /// Repays `payments` against `account_id`'s debt positions, pulling the
    /// funds from the caller.
    fn repay(env: Env, caller: Address, account_id: u64, payments: Vec<(HubAssetKey, i128)>) {
        positions::process_repay(&env, &caller, account_id, &payments);
    }
```

No `#[when_not_paused]` — repay stays live during pause (risk-reducing exit). No owner/delegate check on the target account.

### Body auth chain

1. **`validation::require_authorized_caller`** (`debt.rs:87`) — auth + flash gate on `caller` only.
2. Aggregate positive payments; load **borrow-only** account (`storage::get_account_borrow_only`) — no owner pin.
3. **`settle_debt(..., DebtFlowKind::Repay { payer: caller, ... })`** (`debt.rs:161-183`):
   - Exit flags via `FreezePolicy::AllowOnExit`.
   - **`get_debt_position_or_panic`** — cannot invent a debt slot; missing position reverts.
   - **`transfer_amount_measured`** from `payer` (caller) → pool (INV-ACCT-03).
   - **`apply_repay_batch`** → `merge_debt_leg` with `LegDirection::Exit` (`positions/mod.rs:148-188`) — scaled debt only decreases / is removed.

### Claim checks

| Claim fragment | Verdict | Evidence |
|---|---|---|
| Category `caller-auth` | Match | `require_authorized_caller` |
| Anyone may repay any account | Match | No `require_owner_or_delegate` / `require_account_owner` on `account_id` |
| Funds from caller’s balance | Match | Transfer `from = payer = caller` |
| Measured receipt credit | Match | `transfer_amount_measured`; pool action uses measured amount |
| Target liabilities only fall | Match | Exit-only merge; no borrow path in `process_repay` |
| INV-AUTH-03 (no foreign risk) | Match | Reduces debt; cannot open borrow slots |

### Test evidence

- `tests/.../controller/repay.rs`: `test_repay_by_third_party`, `test_repay_permissionless_payer_auth_only`.
- `tests/.../controller/security_audit.rs`: `poc_permissionless_repay_any_caller`.

---

## 3. `controller::liquidate`

### Entrypoint wiring

```143:166:contracts/controller/src/lib.rs
    /// `liquidator` repays `debt_payments` and seizes collateral at a bonus
    /// scaled by the health factor. Permissionless — an owner may liquidate
    /// itself. ...
    fn liquidate(
        env: Env,
        liquidator: Address,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
        seize_mode: SeizeMode,
    ) -> u64 {
        positions::liquidation::process_liquidation(
            &env,
            &liquidator,
            account_id,
            &debt_payments,
            seize_mode,
        )
    }
```

No pause macro (liquidation remains available when paused). Doc comment matches INV-LIQ-01 self-liquidation.

### Body auth chain

1. **`liquidator.require_auth()`** then **`validation::require_not_flash_loaning`** (`liquidation/mod.rs:53-54`). Equivalent to `require_authorized_caller` split across two calls (no functional gap).
2. Load liquidated account — **no** owner/delegate requirement on `account_id` (permissionless, owner included).
3. **`resolve_seize_receiver`** (`liquidation/mod.rs:170-216`) before any token movement:
   - `Transfer` → no receiver account.
   - `Credit(0)` → `create_account_with(..., liquidator, ..., SpokeAdmission::AllowDeprecated)` — receiver owned by liquidator.
   - `Credit(id)` → `requested != account_id` else `CollateralError::SelfLiquidationNotAllowed` (#133); then **`require_owner_or_delegate(liquidator, receiver)`**; same spoke; `PositionMode::Normal`.
4. **`build_liquidation_plan`** (`plan.rs:19-49`) — empty debt or `health_factor >= Wad::ONE` → `HealthFactorTooHigh` (INV-LIQ-01).
5. **`apply_liquidation_repayments`** — measured pull from liquidator (`apply.rs:52-59`).
6. **`scale_seizures_to_received`** (`math.rs:465-491`) — if measured USD < planned USD, floor-scale every seizure leg (INV-LIQ-02 coupling; under-delivery path also covered by INV-LIQ-03 in STRIDE).

### Claim checks

| Claim fragment | Verdict | Evidence |
|---|---|---|
| Category `caller-auth` | Match | `liquidator.require_auth()` |
| Anyone may liquidate HF < 1 | Match | No target-account owner gate; plan asserts HF < 1 |
| Including account’s own owner | Match | Transfer self-liq allowed; harness `test_self_liquidation_allowed`, `refutation_owner_can_self_liquidate` |
| Credit: receiving ≠ liquidated | Match | `SelfLiquidationNotAllowed` when `requested == account_id` |
| Seizure coupled to debt repaid | Match | Plan close bound + `scale_seizures_to_received` |
| INV-AUTH-03 | Match | Does not open foreign risk; Credit receiver must be liquidator-controlled; Transfer pays liquidator’s own wallet |

**Undocumented but defended (not a claim mismatch):** Credit to a third-party account the liquidator does not control reverts `NotAuthorized` (`liquidation_seize_modes.rs` `credit_to_an_account_the_liquidator_does_not_control_reverts`). Claim text omits this; body is stricter in the safe direction.

### Test evidence

- `liquidation.rs`: self-liq and third-party supply + liq liveness.
- `liquidation_seize_modes.rs`: Credit(0), Credit self-reject, foreign receiver reject, spoke/mode gates.
- `security_audit_extended.rs`: `refutation_owner_can_self_liquidate`.

---

## Cross-cutting auth comparison

| Property | supply | repay | liquidate |
|---|---|---|---|
| `require_auth` on actor | yes (`require_authorized_caller`) | yes (`require_authorized_caller`) | yes (`liquidator.require_auth`) |
| Flash-loan gate | yes (bundled) | yes (bundled) | yes (explicit) |
| Target account owner/delegate | only for **new** supply slots | never | never (liquidated); **receiver** yes in Credit |
| Can create account | `account_id == 0` → caller-owned | no | `Credit(0)` → liquidator-owned |
| Pause | blocked | live | live |
| Risk direction on foreign account | collateral ↑ (or refuse) | debt ↓ | debt ↓ / collateral seized under HF < 1 |

---

## Mismatches / nits flagged

1. **Wording — supply justification “non-owner”** (`permissionless_entrypoints.txt:63`): body uses `is_owner_or_delegate`. Delegates may open new supply slots. Inventory category and INV-AUTH-03 intent still hold. Prefer aligning the line with STRIDE Elevation.5 (“non-owner, non-delegate”). Severity: info. Status: partial (docs only).
2. **Auth helper shape — liquidate**: uses bare `require_auth` + separate `require_not_flash_loaning` instead of `require_authorized_caller`. Behaviorally equivalent; not a security mismatch.
3. **Pause asymmetry**: supply paused; repay/liquidate not. Consistent with STRIDE exit-liveness notes; not contradicting the permissionless claims (those claims do not assert pause policy).
4. **No category false positive**: none of the three are `UNGATED-MUTATOR`; each reaches an auth primitive on the payer/liquidator address.

---

## Verdict

Permissionless inventory claims for `controller::supply`, `controller::repay`, and `controller::liquidate` match body authorization and INV-AUTH-03. Strangers cannot open foreign supply slots, cannot invent debt, and cannot liquidate healthy accounts or Credit-seize into uncontrolled / self-same accounts. The only flagged delta is soft documentation precision on owner vs delegate for supply new-slot authority.
