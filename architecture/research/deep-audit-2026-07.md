# Deep Protocol Audit — rs-lending-xlm (2026-07)

**Scope:** Gap-focused security audit hunting net-new issues the existing defenses (STRIDE, ~230 Certora rules, libFuzzer + proptest, cargo-mutants, testnet e2e) don't already cover. Four sections: governance/access-control, controller endpoints (admin + user), invariants, math/utils.

**Method:** Multi-agent workflow — 38 finder agents (one per work-item) → 3-skeptic adversarial verify per candidate (default *refuted*, kept only on ≥2/3 confirm) → high-effort synthesis (dedupe, rank, cross-check STRIDE residuals). 90 agents total. Every confirmed finding below was additionally re-verified by reading the cited code directly.

**Result:** 3 net-new findings confirmed (1 High, 1 Medium, 1 Low). Nine other candidates were raised by finders but **refuted** by the skeptic panel (recorded in §Negative results). No code was modified.

**Posture:** Formal-verification and fuzz coverage is strong on core per-op invariants (conservation, HF monotonicity, bonus bounds, index monotonicity). The structural blind spots are: liquidation **tier selection** (not just bonus values), **multi-actor** governance collusion, and **lazy risk-parameter re-stamping** on the debt-increasing path.

---

## Findings

### F1 — High — Liquidation tier selection mints avoidable socialized bad debt from a solvent position

- **Location:** `contracts/controller/src/positions/liquidation/math.rs:501-519` (guard at `:515`)
- **Verified:** 3/3 adversarial (adjusted High/Medium/High) + direct code read.
- **Net-new:** yes.

**Root cause.** `estimate_liquidation_amount` returns the *base* tier only when `base.new_hf < Wad::ONE && base.new_hf < snap.hf`. When a base-bonus **full** repayment heals the account (`new_debt == 0` → `calculate_post_liquidation_hf` returns `i128::MAX ≥ 1`), that guard is false, so the function returns `fallback.unwrap_or(base.candidate)`. The fallback tier runs at the HF-scaled (up to max) bonus and caps repayment at `total_collateral/(1+bonus)`. `normalize_repayment_plan` then forces the estimate's bonus, so no liquidator can opt into the safer base path.

**Impact.** For `max_bonus_for_threshold`, `proportion_seized·(1+max_bonus) ≈ 1`, so the fallback repayment ≈ `threshold·collateral = weighted_coll < total_debt`, while seizing ~100% of collateral. The account is left with `total_debt − weighted_coll` of debt and ~0 collateral → falls into the `≤ 5 WAD` socialization band and is written down against hub suppliers. A base-bonus full liquidation would have repaid all debt within collateral and created **zero** bad debt.

**Reachability (broader than the demonstrator).** Triggers whenever `weighted_coll < total_debt ≤ total_collateral/(1+base_bonus)` — i.e. the base tier can fully repay within collateral but the fallback leaves a shortfall. This is **not** limited to exotic low-LT assets: a threshold-0.85 market with `debt/collateral ∈ (~0.85, ~0.95]` qualifies. The agent's clean demonstrator (LT=0.45, $250 collateral → $100 after a 60% drop, $90 debt, HF≈0.5): fallback repays ~$45 seizing all $100 collateral for a ~$54 liquidator windfall, leaving ~$45 socialized bad debt; base-bonus full liquidation would repay all $90, seize ~$94.5, leave ~$5.5, zero bad debt.

**Existing coverage checked.** Certora `derived_bonus_respects_threshold` / `bonus_bounded` / `ideal_repayment_targets_102` (`liquidation_rules.rs`) constrain bonus/ideal **values** in isolation, none constrain tier **selection** or forbid bad debt from a solvent position — and the 102-target bound is what forces the sub-debt repayment. Unit tests (`liquidation_ratchet.rs`, `liquidation_math.rs`) use LT=80% only. Differential fuzz `liquidation_vs_reference.rs` uses LT=80% and its reference (`test-harness/src/reference/liquidation.rs:426`) mirrors the same guard, so it cannot flag the flaw. STRIDE only documents the `≤5 WAD` tainted-debt path (DoS.2), not avoidable tier-selection bad debt.

**Recommendation.** Prefer the base tier whenever it fully heals the account (`base.new_hf ≥ 1`), not only when `base.new_hf < 1` — i.e. select the lowest bonus that still restores health / minimizes resulting bad debt. Equivalently, cap the applied bonus so `seizure·(1+bonus)` never exceeds `total_collateral` while `total_debt` is still fully repayable within collateral. Add a Certora rule and a tier-selection test asserting that liquidating a solvent position (`total_collateral > total_debt`) can never end with `total_debt > 0 && total_collateral == 0`; include an LT<0.5 case.

---

### F2 — Medium — Two colluding CANCELLERs permanently freeze governance and are mutually unremovable

- **Location:** `contracts/governance/src/timelock.rs:281-296` (guard at `:288`)
- **Verified:** 3/3 adversarial (all Medium) + direct code read.
- **Net-new:** yes. **STRIDE D1R1 is stale/incorrect.**

**Root cause.** `cancel()` rejects only a canceller vetoing its **own** revocation (`target != canceller`); the comment states "Other cancellers keep the veto." A CANCELLER can cancel *any* pending operation. The only path to revoke a CANCELLER is the timelocked `propose → execute_self` flow (`revoke_role_immediate` is restricted to GUARDIAN/ORACLE at `timelock.rs:413-421`).

**Impact.** Two compromised/colluding CANCELLER keys C1, C2: C2 cancels the `RevokeGovRole{C1}` op during its Waiting window; C1 cancels `RevokeGovRole{C2}` symmetrically; either cancels any `UpgradeGov`/reconfig op (no revocation marker). Since every op must sit Waiting ≥ min_delay (up to 14 days) before executing, the coalition always has time to cancel. Result: no timelocked op ever executes, the rogue pair is unremovable, and the controller can never be upgraded or reconfigured through governance — a permanent, unrecoverable brake. Owner-immediate pause/unpause and GUARDIAN/ORACLE immediate revokes do not restore governance. Requires 2 privileged keys compromised/colluding.

**Existing coverage checked.** STRIDE DoS.1/D1R1 describes a `mark_uncancellable`/`OperationNotCancellable` "always ejectable" design that was **never implemented** — the shipped code is only a single-target self-veto guard, so the residual is both uncovered and stale. Elevation.4/E4R1 covers only owner-holds-executor+canceller. `governance/tests/timelock.rs` tests single self-veto (`:234`) and the cross-veto primitive (`:257`) but no colluding-pair mutual-block. No Certora rule covers governance cancel (governance is outside the pool/controller FV scope).

**Recommendation.** Add an owner-gated, veto-immune governance-recovery path (e.g. a dedicated timelocked "reset cancellers" op no canceller can cancel) and/or bound the number of CANCELLER holders — accepting the tension that fully-uncancellable revocations re-expose an honest single canceller to instant post-timelock stripping by a compromised owner. Correct STRIDE D1R1 to reflect the real guarantee ("ejectable only given fewer than a colluding pair of cancellers"). Add a regression test for the ≥2-collusion entrenchment scenario.

---

### F3 — Low — Borrow gate uses stamped collateral LTV, bypassing a governance LTV cut

- **Location:** `contracts/controller/src/risk/totals.rs:161`
- **Verified:** 3/3 adversarial (adjusted Low/Low/Medium) + direct code read.
- **Net-new:** yes.

**Root cause.** The post-pool borrow gate (`require_post_pool_risk_gates → calculate_account_risk_totals`) accumulates `ltv_collateral` from the **stored** `position.loan_to_value`, stamped at last supply/withdraw. Borrow never refreshes collateral risk params (`refresh_supply_risk_params` runs only on supply `supply.rs:171` and withdraw `withdraw.rs:219`), and no governance path re-stamps existing accounts.

**Impact.** Asset X listed LTV 80%/threshold 85%; Alice supplies $1,000 (stamped 80%/85%). Governance timelocks a risk-off cut to LTV 50%/threshold 55%. Alice never re-touches X, so her stamp stays 80%/85%. `borrow(Y)` uses the stale 80% LTV → she borrows ~$800 of Y instead of the ~$500 the new policy intends. Freezing X does not help: `enforce_spoke_asset_flags` checks only the borrowed leg (Y), not the collateral leg (X). If X later drops 20%, she becomes liquidatable only under the stale 85% threshold, and a liquidator recovers at most ~$700, leaving ~$100 socialized to Y's suppliers — exactly what the LTV cut was meant to prevent. Conditional on a governance LTV cut + untouched collateral + subsequent price drop.

**Existing coverage checked.** Certora `ltv_borrow_bound_enforced` (`solvency_rules.rs:31`) proves `total_debt ≤ ltv_collateral` only against the stamped fields it reads back (masks the flaw), for len≤1. `threshold_downgrade_implies_account_safe` (`health_rules.rs:446`) covers the supply-refresh threshold-lowering path, not borrow. STRIDE Tamper.3 and memory `supply_threshold_tightening_bypass` cover the supply path; neither covers borrow reading stale collateral LTV.

**Recommendation.** On the borrow (and strategy/migration finalize) debt-increasing paths, re-derive each collateral position's `loan_to_value` from the current effective spoke config before the LTV gate, preserving the existing HF≥1.05 protection so the threshold side is not retroactively tightened on unhealthy accounts. Alternatively, if lazy stamping is intended, document in an ADR/STRIDE that governance LTV cuts don't bind existing collateral until it's next touched, and pair every LTV cut with pausing all borrowable assets.

---

## Negative results (raised by finders, refuted by adversarial verify)

Recorded for the audit trail — these did **not** reach 2/3 confirmation. Worth a targeted second look only where noted.

| Candidate | Location | Finder sev | Note |
|---|---|---|---|
| Timelock production floor enforced off-chain only | `governance/access.rs:180` | Info | Refuted — off-chain runbook + delay-ratchet cover it. |
| Market-creation omits asset-decimal `[3,18]` (only gov validates) | `controller/src/setup/mod.rs:28` | Medium | Refuted — owner=governance always validates; direct-owner path is out of trust model. |
| Compromised GUARDIAN mass-pauses every listing | `governance/timelock.rs:340` | Medium | Refuted — accepted incident-key power; resume is timelocked by design. |
| Permissionless `execute` forces `UpgradeController` at chosen moment | `governance/timelock.rs:196` | Low | Refuted — op already approved via timelock; timing is not attacker-controlled harm. |
| Third-party supply force-restamps victim's liquidation bonus/fees | `controller/src/risk/params.rs:25` | Low | Refuted — bounded by validated risk params; overlaps accepted Tamper.3. |
| Permissionless `clean_bad_debt` front-runs a liquidation near the $5 threshold | `liquidation/math.rs:86` | Low | Refuted — qualification gate + sub-cent fees make it unprofitable/harmless. |
| `swap_debt` Certora rules don't cover cross-hub/same-asset refinance | `certora/controller/spec/strategy_rules.rs:230` | Info | Refuted as exploit; genuine **formal-coverage** gap — consider adding a cross-hub rule. |
| `migrate_from_blend` debt reconciliation rests on Blend over-repay refund semantics | `strategies/migrate_blend.rs:437` | Medium | Refuted (mock-verified), but this is the **thinnest-tested** path (no Certora rule) — retest against real Blend behavior before mainnet. |
| `set_position_limits` stores values with no controller-side re-validation | `controller/src/config/limits.rs:11` | Low | Refuted — governance validates `1..=POSITION_LIMIT_MAX` at propose-time; defense-in-depth nit. |

---

## Recommended follow-ups (priority order)

1. **F1** — fix tier selection to minimize bad debt; add the Certora rule + LT<0.5 test. Highest priority: touches solvency, not covered by any existing rule.
2. **F2** — governance-recovery path / CANCELLER bound; correct STRIDE D1R1.
3. **F3** — re-derive collateral LTV on debt-increasing paths, or document lazy stamping in an ADR.
4. **Coverage debt (from negatives):** add a cross-hub `swap_debt` Certora rule; retest `migrate_from_blend` reconciliation against real Blend refund semantics.
