# Action Plan — Post-Audit / Pre-Mainnet

**Date:** 2026-04-15
**Scope:** Audit remediation delta + Certora follow-ups + fuzzing extensions + docs cleanup.

---

## 1. Bug Fix Status (re-verified against HEAD `ac05343`)

Sweep of every Critical/High/Medium finding in `bugs.md` against current source.

| ID | Severity | Status | Evidence |
|---|---|---|---|
| C-01 | Critical | **FIXED** | `controller/src/lib.rs:481` — `#[only_owner]` on `edit_e_mode_category`. |
| H-01 | High (→Medium) | **FIXED** | `common/src/fp_core.rs:58-62` — sign-aware half-up. |
| H-02 | High (→Medium) | **FIXED** | `common/src/fp_core.rs:72-76` — sign-aware half-up. |
| H-03 | High (→Medium) | **FIXED** | `controller/src/oracle/mod.rs:164-169` — unconditional staleness. |
| NEW-01 | High | **FIXED** | `controller/src/strategy.rs:476-483` — balance-delta spend check + allowance zeroed. |
| NEW-02 | High | **FIXED** | `controller/src/router.rs:58-63` — admin allow-list (`is_token_approved`). |
| NEW-03 | High | **FIXED** | commit `ddbebe4`. |
| M-01 | Medium | **FIXED** | `controller/src/oracle/mod.rs:174-179` — `check_not_future`, 60 s skew. |
| M-02 | Medium | **FIXED** | `controller/src/oracle/mod.rs:42-43` — `price <= 0` panics. |
| M-03 | Medium | **FIXED** | `controller/src/oracle/mod.rs:246,299` — `min_required >= twap_records / 2`. |
| M-04 | Medium | **FIXED** | `controller/src/config.rs:360` — `[60, 86_400]` bounds. |
| M-05 | Medium | **FIXED** | `common/src/constants.rs:26` — `MAX_LAST_TOLERANCE = 5_000` aligned with validation. |
| M-06 | Medium | **FIXED** | `common/src/fp_core.rs:52` — `checked_mul` + explicit panic. |
| M-07 | Medium | **FIXED** | `controller/src/validation.rs:100-108` — `mid > 0`, `optimal > mid`, `optimal < RAY`. |
| M-08 | Medium | **OPEN** | Taylor 5-term still in `common/src/rates.rs`. Mitigated by mandatory index updates on every tx, but weak for >2-year idle markets. |
| M-09 | Medium | **FIXED** | `controller/src/strategy.rs:398,564` — `checked_sub` on withdrawal delta. |
| M-10 | Medium | **INDIRECTLY FIXED** | No explicit entry check, but `amount_out_min = 0` propagates to downstream `require_amount_positive`; regression covered by `fuzz_strategy_flashloan::prop_strategy_swap_collateral_balance_delta`. Recommend explicit check at entry for defense-in-depth. |
| M-11 | Medium | **FIXED** | `controller/src/strategy.rs:396-399,561-564` — balance-delta pattern. |
| M-12 | Medium | **FIXED** | `pool/src/lib.rs:44,214,262,498-499` — `saturating_sub_ray`. |
| M-13 | Medium | **FIXED** | `pool/src/lib.rs:364-365` — `balance_after < pre + fee` check. |
| M-14 | Medium | **FIXED** | `pool/src/lib.rs:187-196` — dust-lock guard promotes partial to full withdrawal. |

**Delta since audit:** 18 of 21 Critical/High/Medium findings closed. **Remaining actionable: M-08 (Taylor bound) and M-10 (explicit entry check).**

LOW (13) and INFO (12): most are by-design or ultra-low impact; a single sweep can close the handful worth fixing (L-01, L-04, L-08, L-09, L-11, L-12, L-13) in a few hours.

---

## 2. Priority Queue (remaining work)

### P0 — Before mainnet (hard blockers)

- [ ] **M-08 hardening.** Either extend Taylor to 8-10 terms OR cap `delta_ms` at 1 year, forcing a keeper-driven `update_indexes` before the error exceeds 1 %.
- [ ] **M-10 explicit entry check.** Add `validation::require_amount_positive(env, steps.amount_out_min)` at every strategy entry (`multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`). One line × 4 sites.
- [ ] **LOW sweep.** Close L-01 (reserve_factor > 10000 in `calculate_deposit_rate`), L-04 (pool `update_params` missing slope ≥ 0 check), L-11 (mid_utilization_ray = 0 now caught, drop the L-11 row), L-12 (combined bonus+fees cap), L-13 (PositionLimits bounds). ~2 h.
- [ ] **Commit cleanliness.** `bugs.md` must carry the FIXED annotations inline so future audits don't re-litigate.

### P1 — Certora trustworthiness (remaining HIGH rule/claim gaps, ~6-8 h)

From `bugs.md:593-611`. Each rule currently invokes internals or `cvlr_satisfy!(true)`; rewrite to invoke the production endpoint and assert the claim.

- [ ] `flash_loan_fee_collected` — pre/post `protocol_revenue()` delta via `pool_interface::LiquidityPoolClient`. **In-progress; fix missing `use pool_interface;` import in `controller/certora/spec/flash_loan_rules.rs`.**
- [ ] Index monotonicity (`index_rules.rs:58,82`) — invoke public `update_indexes` keeper endpoint.
- [ ] `claim_revenue_transfers_to_accumulator` — accumulator balance delta.
- [ ] `clean_bad_debt_requires_qualification` — real predicate (`debt > coll && coll ≤ $5`), not HF.
- [ ] `clean_bad_debt_zeros_positions` — call keeper-only endpoint, not internal helper.
- [ ] `swap_*_conserves_*` + `repay_with_collateral_reduces_both` — USD-value conservation, not "decreased/exists".
- [ ] `supply_scaled_conservation` / `borrow_scaled_conservation` — `delta == amount * RAY / index` within rounding.
- [ ] `borrow_exact_reserves` / `withdraw_more_than_position` — invoke real endpoints (currently local tautologies).

### P2 — Missing Certora coverage categories (~10-14 h)

- [ ] **Access control** for 22 `#[only_owner]` + 11 `#[only_role]` endpoints. Zero rules today. Regression gate for C-01 class.
- [ ] **Adversarial oracle inputs**: zero price, future timestamp, stale price through real endpoints (covers H-03, M-01, M-02 at spec level).
- [ ] **Token-behavior**: leftover allowance (NEW-01), fee-on-transfer (NEW-02), rebasing.
- [ ] **Global accounting / liveness**: `sum(user_scaled) ≤ pool_total_scaled` invariant; healthy-user-can-always-exit (M-12, M-14 at spec level).

### P3 — Weak Certora bounds (~2 h)

- [ ] `tolerance_bounds_valid` — move to config-validation rule (currently assumes+re-asserts).
- [ ] `ideal_repayment_targets_102` — compute real post-liquidation HF.
- [ ] `compound_interest_bounded_output` — tighten 100 000× RAY → 100× RAY (doc-claimed bound).

### P4 — Sanity rule rewrites (~1 h)

- [ ] 30 `*_sanity` rules use `cvlr_satisfy!(true)`. Replace with real-state witnesses or delete.

---

## 3. Fuzzing Extensions (the spawned-agent deliverable)

Existing surface is already strong: 11 libFuzzer targets (function + contract) + 11 proptest harnesses + differential liquidation + Miri + CI smoke + corpus seeding. Gaps ranked by (security impact × likelihood) / hours.

| # | Target | Kind | Hours | Surface |
|---|---|---|---|---|
| 1 | `flow_liquidation_focused` | libFuzzer | 4 | Forced-underwater sweep over liquidator repay fractions; covers `&lt;$5` bad-debt path + HF=1.02 ↔ 1.01 fallback solver (`helpers/mod.rs:221-303`, the most fragile code). |
| 2 | `flow_multi_collateral_seizure` | libFuzzer | 3 | 3+ collaterals with skewed decimals (6/8/18) — catches proportional-seizure drift unreachable from `build_min_context()`'s 2-market setup. |
| 3 | `fuzz_rate_accrual_differential` | proptest | 6 | Mirror the `num_rational::BigRational` trick for compound interest; diffs 5-term Taylor vs exact `e^(rt)`. Natural extension of `fuzz_liquidation_differential`. |
| 4 | `fuzz_e_mode_transitions` | proptest | 3 | E-mode category lifecycle: creation → deprecation → per-account transitions. Governance state changes are audit-bait. |
| 5 | `fuzz_flashloan_reentrancy` | proptest | 4 | Nested flash-loan receivers calling back into supply/borrow/flash-loan. Current `flow_flash_loan` only tests good vs bad receiver. |
| 6 | `flow_oracle_twap_manipulation` | libFuzzer | 3 | Multi-block price trajectories with attacker-shaped deviations. Covers time-series attacks on tolerance gate. |
| 7 | `fuzz_withdraw_partial_sequences` | proptest | 2 | Interleaved supply/withdraw/borrow sequences — catches scaled-amount ordering bugs. |
| 8 | `fp_mul_div_signed` | libFuzzer | 1 | Signed variant of `fp_mul_div` — ideal-repayment solver uses signed arithmetic. Quick win. |
| 9 | `fuzz_position_limit_boundary` | proptest | 2 | Exactly 10 positions then attempt 11th; liquidation iteration at full capacity. |
| 10 | `flow_admin_config_drift` | libFuzzer | 3 | Mutate `MarketParams` across valid bounds; verify LTV < threshold and bonus cap survive every reconfig. |

**Highest-ROI bundle:** #1, #3, #5 (~13 h) cover the two riskiest surfaces the current plan under-specifies (liquidation composition + flash-loan reentrancy + rate-accrual drift).

### Strategic answers to the original questions

- **Full e2e vs pure unit**: hybrid is right and already in place. 70 % campaign time on contract-level, 30 % on function-level. Full e2e is wrong — setup cost dominates per-iteration work.
- **Stellar-idiomatic pattern**: `cargo-fuzz` + `libfuzzer-sys` + `#[derive(Arbitrary)]` + `--sanitizer=thread -Zbuild-std` on macOS. Honggfuzz is a dead-end — adds complexity without matching our snapshot-seeded corpus model.
- **No Soroban-specific fuzz helper library**. Build on `test-harness::LendingTest` — that's the canonical pattern.

### Gotchas to enforce in new harnesses

1. **Miri scope is bounded** to pure-i128 in `fp_core.rs:45-77`. Anything using `&Env` / `I256` panics under Miri's interpreter.
2. **`--sanitizer=thread -Zbuild-std` required on macOS** for contract-level targets; Linux defaults work.
3. **Native vs WASM**: fuzzers run native. Treat all crashes as findings; reproduce in WASM separately before closing.
4. **4-frame-deep auth**: `env.mock_all_auths()` can't reach controller → pool → receiver → SAC. Use `LendingTest::without_auto_auth()` + explicit `MockAuth` trees.
5. **Budget metering**: opt new multi-op harnesses into `LendingTestBuilder::with_budget_enabled()` to catch on-chain-only budget panics.

---

## 4. Documentation Cleanup

Recent commits (`c767f73`, `954757b`, `fd52727`, `c23f9bb`, `e232163`, `253fec9`) already Strunk-ified README / ARCHITECTURE / INVARIANTS / DEPLOYMENT / fuzz docs. Targeted deltas remain:

- [ ] **`bugs.md` FIXED annotations.** Add a `Status:` line to each entry (above or below the description). The table in §1 above is the source.
- [ ] **`controller/certora/spec/oracle_rules.rs:269`** doc-comment lies — says `<= 10000 BPS (100%)` but `MAX_LAST_TOLERANCE = 5_000` (50 %). Single-line fix.
- [ ] **Add `Formal Verification` section to `INVARIANTS.md`**: which invariants are prover-covered, which are property-test-covered, which are runtime-only. Pointers to rules/confs by ID.
- [ ] **`README.md` testing section** should cross-reference `fuzz/README.md` (currently only lists unit/integration/smoke). The "Testing Philosophy" block is three layers but the repo has five (unit, integration, proptest, libFuzzer, Miri, formal-verification-in-progress).
- [ ] **New `SECURITY.md`** (stretch): one-pager pointing readers to `bugs.md`, Certora scope, fuzzing campaign cadence, responsible-disclosure contact.

No full rewrites required — the docs are already tight.

---

## 5. Sequencing

Two parallel tracks, ~wall time ≈ effort of the slower track.

```
Track A (Certora / security — serial)
P0 hardening (3-4 h)
  → P1 rule rewrites (6-8 h)
    → P2 missing categories (10-14 h)
      → P3/P4 polish (3 h)
= 22-29 h

Track B (Fuzzing — parallelizable across agents)
Tier-1 bundle: #1 + #3 + #5 (13 h)
  → Tier-2: #2 + #4 + #6 + #9 (12 h)
    → Tier-3: #7 + #8 + #10 (6 h)
= 31 h

Track C (Docs — 3 h, any time)
```

**Recommendation:** P0 bugs + P1 Certora rewrites + Tier-1 fuzzing first. That's the minimum bar to call the protocol audit-finished. Everything else is polish.
