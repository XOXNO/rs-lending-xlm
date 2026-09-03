# A067 — Min borrow collateral floor enforcement

- Agent: A067
- Theme: T4 (input / risk-gate validation), with T1 touch on admin setter and T1/T4 on keepers
- Severity: low (config / verification residuals); core gate is sound
- Status: defended (risk-increasing + strategy finalize chokepoint); partial (passive / keeper grandfathering + known `BAD_DEBT` desync)
- Paths:
  - `contracts/controller/src/risk/validation.rs:26–60` (`require_post_pool_risk_gates`)
  - `contracts/controller/src/positions/mod.rs:92–104` (`enforce_post_pool_solvency`)
  - `contracts/controller/src/positions/debt.rs:35–76` (`process_borrow`)
  - `contracts/controller/src/positions/supply.rs:157–189` (`process_withdraw`)
  - `contracts/controller/src/strategies/mod.rs:68–80` (`strategy_finalize`)
  - `contracts/controller/src/keepers.rs:68–237` (`update_account_threshold` / `sync_account_thresholds`)
  - `contracts/controller/src/storage/protocol.rs:101–114` (instance get/set + default)
  - `contracts/controller/src/config/registry.rs:64–74` (`set_min_borrow_collateral_usd`)
  - `contracts/controller/src/governance.rs:27` (init default)
  - `contracts/controller/src/constants.rs:3` (`BAD_DEBT_USD_THRESHOLD` compile-time alias of **default**)
  - `common/src/constants/shared.rs:35–36` (`DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD = 5 * WAD`)
  - `common/src/errors.rs:145` (`MinBorrowCollateralNotMet = 126`)
- Defense: Single post-pool chokepoint compares **LTV-weighted** collateral (WAD) to the instance floor when the account still has debt and `floor != 0`. Inclusive at equality. Borrow, withdraw, and every `strategy_finalize` path re-prove it after LTV restamp. Debt-free accounts skip. Admin setter rejects negatives; `0` is the intentional disable switch.
- Gap: (1) Keepers never evaluate the floor — `LtvOnly` / `FullTuple` can restamp LTV downward and leave an indebted account below the floor without revert (grandfathering; next risk-increasing action fails). (2) Known threat-model residual: `BAD_DEBT_USD_THRESHOLD` is a compile-time copy of the **default** floor; raising live `MinBorrowCollateralUsd` desyncs dust cleanup from the economic floor. (3) Certora health fixtures force floor `0`, so formal post-gate rules do not prove the floor inequality. (4) `errors.md` over-lists `#126` callers (`supply` / `repay` / `flash_loan` do not hit the gate).
- Impact: Cannot open or deepen an undersized LTV-collateral book on gated paths; cannot withdraw / strategy-exit collateral below the floor while debt remains. No path found that mints debt under a positive floor without clearing it. Residuals are liveness / cleanup-band / docs / prover coverage — not silent undercollateralized origination.
- Evidence: INV-RISK-01; formulas.md risk gates; numeric-bounds.md §6; STRIDE DoS.9; threat-model “dust gate and configured floor can drift”; harness `tests/controller/min_borrow_collateral.rs`, `admin_config.rs` boundary pin; unit liquidation_math floor profitability suite; peers A015, A029, A049, A072, A102.
- Opinion: The floor belongs exactly where it is — inside `require_post_pool_risk_gates`, after restamp, on risk-increasing and strategy finalize paths. Do not bolt it onto keepers or plain `repay` without a product decision; that would either brick param sync or block dust-debt cleanup. Highest-value follow-ups are (a) ops hygiene when raising the floor vs `BAD_DEBT_USD_THRESHOLD`, and (b) optional Certora witness with non-zero floor — not a new runtime assert on every mutator.

---

## 1. Scope and method

1. Read `shared/COORDINATION.md`, `SEED.md`, README finding format; confirmed `A067-min-borrow-collateral.md` absent.
2. Traced floor read/write, defaulting, admin validation, and every production caller of `require_post_pool_risk_gates` / `enforce_post_pool_solvency` / `strategy_finalize`.
3. Contrasted keeper `update_account_threshold` (HF floor only when `has_risks`) with the min-borrow floor.
4. Cross-checked INV-RISK-01, formulas.md, numeric-bounds §6, threat-model dust/floor drift, STRIDE DoS.9, harness + unit pins, Certora fixture, peers A015 / A029 / A049 / A072 / A102.

Out of primary claim: liquidation seize math profitability (owned by numeric-bounds / A053), oracle freshness (A065), post-gate HF/LTV pairing depth beyond the floor (A072), keeper Vec bounds (A015).

---

## 2. What the floor measures

The gate is **not** a minimum borrow *amount*. It is a minimum **LTV-weighted collateral value (WAD USD)** while the account still carries any debt:

```57:59:contracts/controller/src/risk/validation.rs
    let floor = storage::get_min_borrow_collateral_usd_wad(env);
    if floor != 0 && totals.ltv_collateral.raw() < floor {
        panic_with_error!(env, CollateralError::MinBorrowCollateralNotMet);
```

| Property | Behavior |
|---|---|
| Metric | `AccountRiskTotals.ltv_collateral` (effective LTV = `min(ltv, LT)` applied floor to floored gate value — `risk/totals.rs`) |
| Compare | strict `<` → inclusive at exact equality |
| Disable | `floor == 0` skips the branch entirely |
| Debt-free | early `return` before LTV/HF/floor (`account.debt_free()`) |
| Units | raw WAD `i128` vs stored floor WAD — no token/RAY mix |

Default and init: `DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD = 5 * WAD`; `governance::init` writes it; unset instance key unwraps to the same default (`storage/protocol.rs`).

Economic role (DoS.9 / numeric-bounds §6): keep indebted accounts large enough that liquidations remain profitable under listed decimals×price. Paired conceptually with `BAD_DEBT_USD_THRESHOLD` (same default $5), which gates permissionless `clean_bad_debt` on **total collateral**, not LTV collateral.

---

## 3. Chokepoint and call graph

### 3.1 Central gate

```26:60:contracts/controller/src/risk/validation.rs
/// Validates solvency after a pool-mutating operation; a no-op if the
/// account carries no debt. ...
pub(crate) fn require_post_pool_risk_gates(env: &Env, cache: &mut Cache, account: &Account) {
    if account.debt_free() {
        return;
    }
    // ... LTV collateral ≥ debt, HF ≥ 1 WAD ...
    let floor = storage::get_min_borrow_collateral_usd_wad(env);
    if floor != 0 && totals.ltv_collateral.raw() < floor {
        panic_with_error!(env, CollateralError::MinBorrowCollateralNotMet);
    }
}
```

Order is load-bearing: debt-free skip → LTV coverage → HF → floor. An account that fails LTV/HF never reaches `#126`; an account that clears solvency but sits under the floor still reverts on gated paths.

`enforce_post_pool_solvency` restamps listed supply LTV **then** calls the gate (`positions/mod.rs`). `strategy_finalize` does the same pair without the `restamped` return flag.

### 3.2 Paths that enforce the floor

| Path | Wrapper | Notes |
|---|---|---|
| `borrow` | `enforce_post_pool_solvency` | Primary origination gate |
| `withdraw` | `enforce_post_pool_solvency` | Blocks collateral exit that would leave debt under floor |
| `multiply` | `strategy_finalize` | Post-legs |
| `flash_position` | `strategy_finalize` | Post-callback deposit + finalize |
| `swap_collateral` | `strategy_finalize` | Collateral reshape while debt open |
| `swap_debt` | `strategy_finalize` | Debt reshape; collateral unchanged but still re-proved |
| `repay_debt_with_collateral` | `strategy_finalize` | Collateral-reducing repay still must leave LTV ≥ floor **or** go debt-free |
| `migrate_from_blend` | `strategy_finalize` | Imported book must clear floor if debt remains |

### 3.3 Paths that intentionally skip

| Path | Why skip is correct |
|---|---|
| `supply` | Risk-decreasing / additive collateral; no post-pool solvency gate (Certora `post_gate_supply_skips_gate_witness`) |
| `repay` (plain) | Debt reduction only; may leave dust **debt** atop large collateral (`test_partial_repay_leaving_small_debt_succeeds`) |
| `liquidate` / bad-debt cleanup | Must unwind unhealthy or dust-insolvent books; floor would brick liquidations |
| `flash_loan` | Pool temporary loan; no controller position book mutation |
| Keepers (`update_indexes`, `claim_revenue`, `update_account_threshold`, `recapitalize`) | Timing / param sync / revenue; must not require borrower top-ups mid-refresh |

**Asymmetry (documented, not a bypass):** wallet `repay` can leave tiny residual debt with large collateral. Collateral-selling strategies cannot leave tiny residual **collateral** with debt — `strategy_finalize` would raise `#126`. Escape hatches: repay to debt-free first, then withdraw; or `close_position` on repay-with-collateral (requires empty debt).

---

## 4. Admin / storage surface

| Concern | Enforced? | Location |
|---|---|---|
| Owner-only setter | yes | `lib.rs` + `only_owner` (A001/A009) |
| Negative floor | rejected `#116` | `registry::set_min_borrow_collateral_usd`; governance op pre-assert |
| Zero floor | allowed — disables gate | harness `test_min_borrow_collateral_gate_disabled_when_floor_zero` |
| Event | yes | `UpdateMinBorrowCollateralEvent` |
| Default if unset | `DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD` | `get_min_borrow_collateral_usd_wad` |
| Inclusive boundary | yes | `test_min_borrow_floor_is_inclusive_at_exact_boundary` |

Governance can raise the floor above existing accounts’ LTV collateral. Those accounts are not force-liquidated; they become unable to borrow/withdraw/strategy-mutate until they repay fully or add collateral. That is intentional ratchet behavior for INV-RISK-01 “risk-increasing actions re-prove.”

---

## 5. Keepers interaction (in scope)

### 5.1 `update_indexes` / `claim_revenue` / `recapitalize`

No account risk valuation. Index accrual can change debt/supply share values over time, but keepers do not invoke the floor. Passive drift below the floor (price crash, LTV cut, interest) is expected; the next gated user action fails closed.

### 5.2 `update_account_threshold`

```221:233:contracts/controller/src/keepers.rs
    if full_tuple {
        let hf = risk::calculate_account_risk_totals(...)
            .health_factor;
        assert_with_error!(
            env,
            hf >= Wad::from(THRESHOLD_UPDATE_MIN_HF_RAW),
            CollateralError::HealthFactorTooLow
        );
    }
```

| Scope | LTV restamp | Liq tuple | HF ≥ 1.05 | Min-borrow floor |
|---|---|---|---|---|
| `has_risks = false` (`LtvOnly`) | yes | no | **no** | **no** |
| `has_risks = true` (`FullTuple`) | yes (unconditional in `refresh_supply_risk_params`) | gated | yes | **no** |

`refresh_supply_risk_params` always writes `position.loan_to_value = effective_config.loan_to_value` before the liquidator-favor gate on LT/bonus/fees. A permissionless keeper can therefore apply a governance LTV cut that drops `ltv_collateral` below the instance floor while HF remains healthy. Outcome: account is “below floor” in storage until a risk-increasing path reverts with `#126`. No new debt is created; liquidation eligibility still keys off LT-weighted HF, not the borrow floor (A015).

**Judgment:** missing keeper floor check is **not** an undefended origination bug. Adding `#126` to `sync_account_thresholds` would turn governance LTV reductions into keeper DoS (refresh reverts until every affected account tops up). Prefer leaving grandfathering as designed.

---

## 6. Residuals and non-gaps

### 6.1 Known: floor vs `BAD_DEBT_USD_THRESHOLD` desync (threat-model)

```3:3:contracts/controller/src/constants.rs
pub const BAD_DEBT_USD_THRESHOLD: i128 = DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD;
```

Live floor is instance-storage and governance-settable. Bad-debt dust cap is a **compile-time** alias of the default. Raising `MinBorrowCollateralUsd` opens a band where:

- new risk-increasing actions require the higher LTV floor, but
- permissionless `clean_bad_debt` still only admits `total_collateral ≤ $5` (default), so larger “economically dust” insolvent stubs need `force_socialize_bad_debt` (owner).

Severity: **low / ops** — already in `threat-model.md`; not a novel A067 critical. Remediation is product: couple the constants, or document that raising the floor requires a coordinated code upgrade of `BAD_DEBT_USD_THRESHOLD`.

### 6.2 Passive below-floor “zombies”

Price drop or LTV cut → HF ≥ 1 and `ltv_collateral < floor`. Account cannot withdraw/borrow/strategy; can repay or receive third-party supply. Liquidators idle until HF < 1. This is the intended dust-prevention posture, not a bypass.

### 6.3 Listing unit-value vs floor (numeric-bounds §6.4)

Floor clears profitability for **currently listed** assets. Extreme 3-decimal / high-price listings can make floor-sized liquidations seize nothing. Listing-admission constraint, not a missing assert in `validation.rs`. Agree with numeric-bounds follow-ups; out of A067 runtime scope.

### 6.4 Certora coverage gap

`certora/controller/spec/fixture.rs` sets `set_min_borrow_collateral_usd_wad(env, 0)`. Post-gate rules prove LTV≥debt / observation finality with the floor **disabled**. No rule asserts `ltv_collateral >= floor` after borrow/withdraw. Verification residual only.

### 6.5 Docs drift

`errors.md` claims `#126` shares callers with `#100`, including `supply`, `repay`, and `flash_loan`. Live code: those three do not call `require_post_pool_risk_gates`. Accurate caller set ≈ borrow, withdraw, multiply, swap_*, repay_debt_with_collateral, migrate_from_blend, flash_position.

### 6.6 Explicit non-gaps

- Floor uses LTV-weighted collateral (not raw USD) — matches INV-RISK-01 / formulas.
- `floor == 0` disable is intentional test/ops escape (`with_min_borrow_collateral_disabled`).
- Liquidation skipping the floor is required for INV-LIQ unwind.
- Inclusive equality is pinned by harness.
- Aggregate multi-asset LTV is used (`test_borrow_not_blocked_by_unrelated_supply_price_crash`).

---

## 7. Test / evidence map

| Claim | Evidence |
|---|---|
| Borrow below floor reverts `#126` | `min_borrow_collateral.rs::test_borrow_rejected_when_ltv_collateral_below_instance_floor` |
| Borrow at/above floor OK | `test_borrow_succeeds_when_ltv_collateral_meets_instance_floor`; `admin_config::test_min_borrow_floor_is_inclusive_at_exact_boundary` |
| Withdraw while indebted below floor reverts | `test_withdraw_while_in_debt_rejected_when_ltv_collateral_falls_below_floor` |
| Debt-free small residue OK | `test_small_supply_succeeds_when_debt_free`; `test_withdraw_while_debt_free_allows_small_residue` |
| Plain partial repay OK | `test_partial_repay_leaving_small_debt_succeeds` |
| Floor disabled at 0 | `test_min_borrow_collateral_gate_disabled_when_floor_zero` |
| Negative setter rejected | governance harness + integration `admin.sh` |
| Floor clears liq profitability (listed set) | `liquidation_math.rs::the_min_borrow_collateral_floor_clears_the_unprofitability_threshold_for_every_listed_pair` |
| Keeper HF floor ≠ min-borrow floor | A015; `keepers.rs` FullTuple assert only |
| Strategy finalize shares gate | A048 / A049 / A072 |

---

## 8. Cross-agent alignment

| Peer | Relation |
|---|---|
| A072 | Owns broader post-pool gate; A067 specializes the floor branch and keeper non-enforcement |
| A015 | Keeper 1.05 HF floor defended; confirms no min-borrow check on threshold sync — agree |
| A029 | Floor lives in instance protocol storage — agree |
| A049 | `strategy_finalize` applies min collateral after repay-with-collateral — agree; A067 explains why that is stricter than plain repay |
| A102 | Placeholder row for A067; this file fills dedicated semantics |
| A053 / A059 | Depend on floor for dust-fee economic irrelevance — agree contingent on non-zero configured floor |

No disagreement file warranted: residuals are already named in threat-model / numeric-bounds or are intentional grandfathering.

---

## 9. Remediation backlog (audit-only)

| Priority | Action | Closes |
|---|---|---|
| P2 | When raising live floor above $5, coordinate `BAD_DEBT_USD_THRESHOLD` (upgrade) or document owner-only socialization for the gap band | §6.1 / threat-model |
| P3 | Fix `errors.md` `#126` caller list to match live gate sites | §6.5 |
| P4 | Optional Certora rule with non-zero floor + borrow/withdraw witness | §6.4 |
| P5 | Do **not** add floor assert to `update_account_threshold` or plain `repay` without product review | anti-regression |
| P5 | Do **not** apply floor inside liquidation / `clean_bad_debt` | preserves unwind |

---

## 10. Verdict

Min-borrow-collateral enforcement is **defended** at the correct chokepoint for every risk-increasing user and strategy path that can leave an indebted book. Keepers correctly omit it. Residual severity is **low**: governance/config desync with the compile-time bad-debt constant, prover fixtures that disable the floor, and docs caller-list drift. No novel critical gap vs SEED / threat-model / A072.
