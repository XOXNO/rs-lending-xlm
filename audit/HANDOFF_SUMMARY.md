# Adversarial Audit Hand-Off Summary

**Series**: 13 loops × ~10 min each, post-remediation of `d59afe1`.
**Baseline**: Stellar `rs-lending-xlm` HEAD `96a93c4` vs MVX sibling `rs-lending` HEAD `426639b`.
**Prior hunt baseline**: `audit/FINDINGS.md` (25+ findings, most remediated in `d59afe1`).

---

## Finding Tally

| ID | Severity | Title | File:Line | Status | Fix |
|---|---|---|---|---|---|
| N-01 | Medium | Supply top-up refreshes LT; bypasses keeper's 1.05 HF buffer | `controller/src/positions/supply.rs:157-158` | ✅ verified | Remove LT refresh (align MVX) OR add post-refresh HF guard |
| **N-02** | **CRITICAL** | `withdraw(amount < 0)` mints phantom collateral; permissionless drain | `controller/src/positions/withdraw.rs:85`; `pool/src/lib.rs:171-227` | ✅ verified | Add `require_amount_positive` + pool-side defense-in-depth |
| N-03 | Low | Pool mutators lack defense-in-depth sign guards | All pool `verify_admin` endpoints | ✅ verified | Add `amount < 0` rejection at pool entry |
| N-04 | Info | `add_rewards` at zero supply strands tokens | `pool/src/lib.rs:307`; `common/src/rates.rs:111-114` | ✅ verified | Panic on `supplied == ZERO` |
| N-05 | Medium | Isolated-debt ceiling bypass via lax oracle on repay | `controller/src/positions/repay.rs:23`; `utils.rs:61-92` | ✅ verified | Proportional reduction (oracle-free) OR strict-oracle on adjust |
| N-06 | Low | Pool accumulator is construct-only; no rotation path | `pool/src/lib.rs:77,485-504` | ✅ verified | Read accumulator from controller at claim time |
| N-07 | Medium | `SpotOnly` accepted on Active markets; removes oracle tolerance | `controller/src/config.rs:363`; `oracle/mod.rs:82-86` | ✅ verified | Reject SpotOnly under non-test builds |
| N-08 | Medium | 32+32 liquidation exceeds event-size budget (~34 KB vs 16 KB) | `controller/src/positions/liquidation.rs:56-126` | ⚠️ needs benchmark | Aggregate-event refactor OR staged partial-liquidation OR reduce PositionLimits |
| N-09 | Medium | Deprecated e-mode leaves inflated LTV/LT; borrow/withdraw bypass | `controller/src/positions/emode.rs:11-27`; `borrow.rs:376-440` | ✅ verified | Add `ensure_e_mode_not_deprecated` to `process_borrow` + live-config fallback |
| N-10 | Info | Pause blocks `clean_bad_debt`; post-unpause interest pulse | `controller/src/lib.rs:355-359` | ✅ verified | Document OR decouple `clean_bad_debt` from pause |
| ~~N-11~~ | ~~Low~~ | **REFUTED loop 23** — `token_price` DOES check `price <= 0` at egress (oracle/mod.rs:41-44). Original finding missed the composition-level guard. | — | ❌ false positive | — |
| N-12 | Low | `compound_interest` Taylor degrades at high `x`; overflows at x≈25 and bricks pool | `common/src/rates.rs:67-105` | ✅ verified | Cap `delta_ms` in `global_sync` + iterate compound over sub-intervals |
| N-13 | Info | HF calc can overflow for high-decimal borrow asset at dust debt + large collateral | `controller/src/helpers/mod.rs:116` | ✅ verified | Clamp division overflow to `i128::MAX` OR cap `asset_decimals` |

**Distribution**: 1 Critical · 5 Medium · 3 Low · 3 Info = **12 findings** (was 13; N-11 refuted in loop 23).

---

## Meta-Patterns

Three structural patterns drove most findings:

1. **Cached-parameter drift** (N-01, N-05, N-09). Controller stores a snapshot of risk parameters (LT, LTV, price) at one moment; uses it later against a different live reality. Fix template: block the divergent path OR recompute against live config.

2. **Signed-type ingress leaks** (N-02 Critical, N-11). Stellar's `i128` at every SEP-40 / ABI boundary has no type-level positivity guarantee. MVX uses `ManagedDecimal<BigUint>` — same class of bugs is unrepresentable there. Fix template: `require_amount_positive` at every user-facing ingress + defense-in-depth at pool layer. Consider `PositiveI128` newtype for ABI boundaries.

3. **Scope/rotation trade-offs in remediations** (N-06). L-05 fix baked the accumulator into pool storage to prevent caller-supplied destination abuse, but removed rotation capability. Shared lesson: "immutability for security" must not foreclose operational recovery (key rotation, etc.).

---

## Cross-Chain Differential Summary

| Finding | MVX parity |
|---|---|
| N-01 | **MVX immune** — `update_deposit_position` refreshes only LTV/bonus/fees, not LT. Stellar's M-06 over-reached. |
| N-02 | **MVX structurally immune** — `ManagedDecimal<BigUint>` is unsigned at the type system. |
| N-03 | **MVX immune** — same type-system reason as N-02. |
| N-05 | MVX has same shape with **~half magnitude** — MVX uses `(agg + safe) / 2` at high deviation; Stellar uses safe_price (TWAP) alone. |
| N-07 | MVX parity TBD — oracle architecture differs (no `SpotOnly` equivalent as a single flag). |
| N-08 | Not applicable — MVX has different per-tx resource model. |
| N-09 | **MVX partially immune** — blocks borrow leg via `ensure_e_mode_not_deprecated` at `lib.rs:252`; withdraw leg still shared. Stellar full exploit ≈ 2× MVX's withdraw-only magnitude. |
| Others | Not yet compared. |

**Takeaway**: MVX's stricter type system prevented ~3 of the 11 findings from being possible. Stellar pays for `i128` flexibility with ongoing manual-validation burden.

---

## Fuzz Harness Integration

Existing fuzz harness at `fuzz/fuzz_targets/` (9 targets) has coverage gaps for 6 of 11 findings. Property-test manifest lives in Appendix A (below) and maps each finding to a specific invariant suitable for libFuzzer / proptest.

**Top-priority new fuzz target**: `flow_withdraw_sign_guard.rs` with amount generator including `{-100, -1, 0, 1, 100, i128::MAX}`. Would have caught N-02 at first run.

**Current generator bug**: `flow_multi_op.rs` uses `arb_amount(amount, 1.0, 50_000.0)` — positive-only, provably cannot surface N-02.

---

## Recommended Fix Order

1. **N-02** (Critical, one-line in `withdraw.rs` + defense-in-depth in `pool/lib.rs`).
2. **N-07** (Medium, config-time reject of `SpotOnly` under non-test builds).
3. **N-09** (Medium, `ensure_e_mode_not_deprecated` in `process_borrow`).
4. **N-05** (Medium, proportional isolated-debt decrement).
5. **N-01** (Medium, align with MVX — remove LT refresh on supply).
6. **N-08** (Medium, benchmark first to confirm; then staged-liquidation or aggregate event).
7. **N-11** (Low, oracle sign check at ingress).
8. **N-03** (Low, pool sign guards — class fix via type-system or explicit checks).
9. **N-06** (Low, accumulator source-of-truth in controller).
10. **N-04, N-10** (Informational, DEPLOYMENT.md runbook additions).

---

## Outstanding Empirical Work

| Item | Owner | Purpose |
|---|---|---|
| Benchmark 32+32 liquidation event-size + read/write entries | eng team | Confirm or refine N-08 severity |
| PoC tests for N-01, N-02, N-05, N-09 | eng team | Regression coverage; reproducers for auditors |
| Fuzz target `flow_withdraw_sign_guard.rs` | eng team | Regression for N-02 class |
| MVX parity deep-dive on N-07, N-09 | cross-chain auditor | Classify MVX exposure per finding |

---

## Audit Memory Files

- `audit/SCOPE.md` — frozen commit, file list, in/out scope.
- `audit/AUDIT_PREP.md` — review goals and auditor-facing questions.
- `audit/AUDIT_CHECKLIST.md` — hand-off gating checklist.
- `audit/THREAT_MODEL.md` — adversary models and residual-risk analysis.
- `audit/CODE_MATURITY_ASSESSMENT.md` — Trail-of-Bits 9-category maturity scorecard.
- `audit/FINDINGS.md` — prior pre-audit hunt baseline (pre-remediation, H/M/L series).
- `audit/new-findings.md` — N-01 through N-13 full write-ups with reproducers and invariant catalog (I1-I23 + proof obligations PO-1..PO-5).
- `audit/HANDOFF_SUMMARY.md` — this document; tally, meta-patterns, cross-chain parity, fuzz property-test manifest (appendix A below).

---

## Sign-Off Recommendation

- Fix N-02 BEFORE any external deployment (Critical; permissionless drain).
- Fix N-01, N-05, N-07, N-09 before audit hand-off (Medium; governance / oracle / e-mode governance safety).
- N-03, N-06, N-11 can be bundled into a hardening PR with auditor sign-off.
- N-04, N-10 are operational-policy documentation; no code change required.
- N-08 requires empirical measurement; fix path depends on benchmark result.

Audit depth diminishing; recommend handing off to formal auditor + team empirical work for remaining questions.

---

## Loop 15 Wind-Down (2026-04-17)

After 15 loops, the adversarial audit has plateaued. The last three loops yielded:
- Loop 13: 0 findings (TTL discipline clean, fuzz-gap pinpointed).
- Loop 14: 0 findings (views clean, HANDOFF_SUMMARY published).
- Loop 15: 0 findings (PositionMode cross-chain parity OK; swap_debt atomicity confirmed).

**Axes that have been covered in depth**:
- Boundary math (fixed-point, rounding, overflow)
- Cached parameter drift (LT, LTV, price, decimals)
- Signed-type ingress (amount, price, fee)
- Cross-chain differential (Stellar vs MVX)
- Storage (keys, entries, TTL)
- Soroban budget (events, reads, writes, footprint)
- Access control (owner, keeper, oracle, revenue)
- Pause/upgrade/rotation semantics
- Flash-loan begin/end atomicity
- Liquidation cascade math
- E-mode lifecycle and deprecation
- Isolated-debt accounting
- Oracle three-branch tolerance + SpotOnly fencing
- Remediation regression (post-d59afe1 verification)
- Fuzz harness coverage mapping

**Axes best served by formal auditor + team empirical work**:
- Instruction-count worst case at 32+32 liquidation (needs live metering).
- Property-based fuzzing with negative-amount generators (N-02 class).
- Certora / formal verification of HF monotonicity across all entrypoints.
- Reflector oracle live-integration edge behavior (beyond our static assumptions).
- Real-token (FoT, rebase) property harnesses.

**Recommended hand-off actions**:
1. Engage Runtime Verification + Certora per `audit/AUDIT_CHECKLIST.md`.
2. Ship N-02 fix PRE-AUDIT (one-line, Critical).
3. Deliver Appendix A (property-test manifest) to auditor team as acceptance-criteria manifest.
4. Deliver `HANDOFF_SUMMARY.md` + all 13 `new-findings.md` entries as engagement scope.
5. Add `flow_withdraw_sign_guard.rs` fuzz target before engagement start.

Cron `65aba23d` can be cancelled; audit loop's marginal value is zero at this point.

---

## Loop 17 Final Wind-Down (2026-04-17)

17 loops total. Two post-wind-down loops (16, 17) were precipitated by full cover-to-cover reads that loops 1-15 had only sampled:

- **Loop 16** surfaced N-12 (Low/Hardening) by reading `common/src/rates.rs` end-to-end.
- **Loop 17** confirmed zero new findings from `common/src/fp.rs` and the remainder of `controller/src/oracle/mod.rs`.

**Lesson captured**: "no new findings for N loops" is a weak stopping signal; "every module read cover-to-cover" is the strong one. At loop 15, the audit was PLAUSIBLY done; at loop 17, it is DEMONSTRABLY done.

All in-scope modules were read cover-to-cover across loops 1-17. Further loops would require either empirical instrumentation (runtime metering, gas benchmarks, property-based fuzzing with negative generators) or re-reading for regressions after remediation PRs land.

**Hand off to formal auditor and team empirical work.**

---

## Loop 19 Verification Pass (2026-04-17)

Cross-checked the original `audit/AUDIT_PREP.md` pre-audit questions against the current 12-finding inventory:

- Q1 (missing `require_not_flash_loaning` on user mutators) — **DOC OUTDATED**; post-remediation every mutator gates correctly. Loop 2 verified.
- Q2 (Σ liquidation slices ≤ capped) — holds by construction: protocol_fee is a cut of bonus, not a fourth slice. Loop 5 verified.
- Q3 (clean_bad_debt path match) — both paths call same `apply_bad_debt_to_supply_index`. Loop 9 verified.
- Q4 (32+32 footprint) — Covered by N-08.
- Q5 (token donation desync) — refuted. Donation gifts suppliers, cannot drain. Attacker-side donation loses the donation.
- Q6-9 (Certora-track questions) — out of scope for this adversarial series.

All in-scope pre-audit questions either answered or transparently mapped to tracked findings. No unaddressed items.

---

## Appendix A — Property-Test Manifest

Testable invariants that the N-series findings break. A fuzz harness landing these properties catches the live bugs AND regressions during remediation.

Target harness: `fuzz/` (existing workspace). Cross-chain analogue: `rs-lending/fuzz/` on MVX.

### How to read each property

- **Pre**: the state / inputs that satisfy the property.
- **Action**: the call(s) to exercise.
- **Post**: the invariant to assert after Action.
- **Expected failure**: if the bug is present, how the property breaks.

### Properties

#### P-N01 — Supply top-up never decreases HF
- **Pre**: account has borrow positions with `HF ∈ [1.0, 1.05)`; admin has lowered asset LT via `edit_asset_config` since the last supply.
- **Action**: `process_supply([(asset_X, any_amount > 0)])`.
- **Post**: `helpers::calculate_health_factor(account)` post-action ≥ `helpers::calculate_health_factor(account)` pre-action.
- **Expected failure (N-01)**: HF drops because M-06 LT refresh propagates a lower LT without HF guard.

#### P-N02 — Pool conservation: post-withdraw balances equal pre-withdraw balances + transferred amount
- **Pre**: any valid account state; any `amount`.
- **Action**: `process_withdraw([(asset, amount)])`.
- **Post**: `pool.supplied_scaled × pool.supply_index / RAY` (nominal supply) decreases by exactly `amount` (capped at position's current value). `token.balance(alice)` increases by exactly the transferred amount. Sum of pool.supplied + pool.revenue remains invariant with respect to unrelated accounts.
- **Expected failure (N-02)**: with `amount < 0`, both `pool.supplied_scaled` and `alice.scaled_amount_ray` INCREASE without any token transfer — phantom collateral.
- **Simpler test**: `assert_amount_positive(amount)` at `process_single_withdrawal` entry before any math. If test passes under negative amount, bug exists.

#### P-N03 — Pool-level mutators reject negative amounts at ingress
- **Pre**: any pool state.
- **Action**: call each `pool.{supply, borrow, withdraw, repay, add_rewards, flash_loan_begin, flash_loan_end, create_strategy}` directly (as admin) with `amount = -1`.
- **Post**: each call panics.
- **Expected failure (N-03)**: currently only `flash_loan_end` has an explicit `fee < 0` guard. Other endpoints proceed silently and rely on controller to have already validated.

#### P-N04 — `add_rewards` at zero supply reverts
- **Pre**: pool with `supplied = Ray::ZERO`.
- **Action**: `add_rewards(price_wad, amount = 100)`.
- **Post**: the call should panic with a specific error (e.g., `NoSuppliersToReward`).
- **Expected failure (N-04)**: call succeeds; `amount` tokens transfer to pool; `update_supply_index` short-circuits; tokens stranded.

#### P-N05 — Isolated-debt counter is proportional to repaid fraction
- **Pre**: account in isolated mode with `isolated_debt = D`, debt outstanding = `debt`, debt token price spot = `p_spot`, TWAP = `p_twap`, tolerance band = strict/lax divergence.
- **Action**: `process_repay([(asset, r)])` for `r < debt`.
- **Post**: `new_isolated_debt = D × (debt - r) / debt` (proportional; oracle-independent).
- **Expected failure (N-05)**: under spot-TWAP divergence >LAST tolerance, `new_isolated_debt = D - r × p_twap` where `p_twap > p_spot`. Decrement is inflated; counter drifts below real USD debt.

#### P-N06 — Pool accumulator follows controller accumulator
- **Pre**: deploy pool P with accumulator `A₀`. Controller has `Accumulator = A₀`.
- **Action**: controller-owner calls `set_accumulator(A₁)`. Then `P.claim_revenue(...)`.
- **Post**: revenue is transferred to `A₁`, not `A₀`.
- **Expected failure (N-06)**: revenue goes to `A₀` (pool's baked-in address). Rotation fails.

#### P-N07 — `configure_market_oracle` rejects `SpotOnly` on Active markets
- **Pre**: market `asset_X` is Active.
- **Action**: `configure_market_oracle(asset_X, { exchange_source: SpotOnly, ... })` as ORACLE role.
- **Post**: call panics with `SpotOnlyNotProductionSafe` (or equivalent).
- **Expected failure (N-07)**: call succeeds; subsequent price reads bypass all tolerance checks.

#### P-N08 — Worst-case bulk endpoints fit Soroban budget
- **Pre**: account with `max_supply_positions` + `max_borrow_positions` populated (currently 32+32).
- **Action**: liquidate the account via `process_liquidation(debt_payments_covering_all)`.
- **Post**: transaction succeeds; event-size ≤ 16,384 bytes; write entries ≤ 200; read entries ≤ 200; instructions ≤ 400M.
- **Expected failure (N-08)**: event-size exceeds 16 KB; tx aborts mid-seize.

#### P-N09 — Deprecated e-mode blocks risk-increasing operations
- **Pre**: account in e-mode category X with positions; admin calls `edit_e_mode_category(X, is_deprecated=true)`.
- **Action 1**: `borrow_batch([(any, positive_amount)])`.
- **Post 1**: call panics with `EModeCategoryDeprecated` (or `calculate_ltv_collateral_wad` falls back to base LTV).
- **Action 2**: `process_withdraw([(asset, positive_amount)])` where post-withdraw HF at base LT would be < 1.
- **Post 2**: call panics with `InsufficientCollateral`.
- **Expected failure (N-09)**: both actions succeed using stored-position boosted LTV/LT, creating bad debt.

#### P-N10 — `clean_bad_debt` available during pause (optional behavior)
- **Pre**: protocol paused; account with bad debt.
- **Action**: `clean_bad_debt(account_id)`.
- **Post** (optional design choice): if the team accepts N-10's alternative, `clean_bad_debt` succeeds. Otherwise, expected to panic.
- **Expected behavior (current N-10)**: panics with `ContractPaused`. This is documented behavior, not a fix target unless the team chooses option B.

#### P-N11 — Oracle rejects negative and zero prices at ingress
- **Pre**: any market.
- **Action**: mock Reflector to return `ReflectorPriceData { price: -100, timestamp: now }` (or `price = 0`).
- **Post**: any `cached_price(asset)` call panics with `InvalidPrice` (or equivalent).
- **Expected failure (N-11)**: negative/zero price propagates into HF/LTV math and produces non-sensical values.

### Cross-cutting invariants

#### P-CONSERVATION — Total value is conserved across operations
- **Pre**: any valid protocol state; snapshot `total_pool_tokens = Σ reserves_i`, `total_user_debt_usd = Σ (debt_i × price_i)`, `total_user_collateral_usd = Σ (collateral_i × price_i)`, `total_scaled_supply = Σ pool_i.supplied`, `total_scaled_revenue = Σ pool_i.revenue`.
- **Action**: any SINGLE successful endpoint call.
- **Post**: `Δ(total_pool_tokens) = Δ(transfers_in) - Δ(transfers_out)`. `Δ(total_scaled_supply) - Δ(total_scaled_revenue) = scaled_user_delta`. Accrual deltas explain any residual.
- **Expected failure**: any phantom-mint (N-02 family) breaks this. Any stranded-token (N-04 family) breaks reserves conservation.

#### P-ACCRUAL-MONOTONIC — Interest indexes only grow
- **Pre**: any pool state.
- **Action**: any call that triggers `global_sync`.
- **Post**: `new_borrow_index ≥ old_borrow_index`; `new_supply_index ≥ old_supply_index`, UNLESS the call is `apply_bad_debt_to_supply_index` (which explicitly drops supply_index to socialize losses).
- **Expected failure**: reveal any unintended index-dropping path.

#### P-ACCOUNT-BOUNDED — No user position can exceed the pool's total
- **Pre**: any account state.
- **Action**: any endpoint.
- **Post**: `Σ all_users.scaled_amount_ray_for(asset) ≤ pool_i.supplied`. Mirror for borrow.
- **Expected failure**: N-02 family; any off-by-one in scaled conversion.

### Existing fuzz harness inventory

`fuzz/fuzz_targets/` contains:
- `compound_monotonic.rs` — compound-interest property.
- `flow_flash_loan.rs` — flash-loan flow.
- `flow_multi_op.rs` — multi-op sequences.
- `flow_oracle_tolerance.rs` — oracle tolerance.
- `flow_supply_borrow_liquidate.rs` — supply/borrow/liquidate flow.
- `fp_div_by_int.rs`, `fp_mul_div.rs`, `fp_rescale.rs` — fixed-point primitives.
- `rates_borrow.rs` — rate model.

Three-layer strategy per `fuzz/README.md`: (1) function-level libFuzzer on `common` primitives; (2) contract-level libFuzzer on full flows; (3) proptest under `test-harness/tests/fuzz_*.rs`.

### Coverage gap map vs. finding properties

| Property | Likely covered? | Gap notes |
|---|---|---|
| P-N01 (supply HF monotonicity after LT refresh) | ⚠️ maybe (`flow_multi_op` combines supply + edit_asset_config) | Confirm property asserts HF post-supply ≥ HF pre-supply. |
| P-N02 (negative amount withdraw phantom) | ❌ **NO** | Existing flows likely use `amount > 0` generators. This is the one-property test that would have caught the Critical. |
| P-N03 (pool-level sign guards) | ❌ no (pool is always called via controller in flows) | Add pool-direct calls with negative amount. |
| P-N04 (add_rewards zero-supply) | ❌ likely no (add_rewards not in listed flow targets) | Low priority. |
| P-N05 (isolated-debt proportional) | ⚠️ `flow_oracle_tolerance` may hit this but doesn't test the counter directly | Add an invariant check on isolated_debt counter after repay. |
| P-N06 (accumulator rotation) | ❌ not in flows; hardening fix required first | File alongside fix PR. |
| P-N07 (SpotOnly rejection) | ❌ not fuzzed; config-space fuzzing not listed | Add targets that mutate `exchange_source` values. |
| P-N08 (worst-case budget) | ⚠️ `flow_multi_op` may stress positions but budget assertion unclear | Add explicit `env.budget()` assertion after 32+32 liquidation. |
| P-N09 (deprecated e-mode bypass) | ❌ not fuzzed | Add e-mode deprecation + borrow_batch sequence. |
| P-N10 (pause blocks clean_bad_debt) | n/a (documentation/design) | — |
| P-N11 (negative/zero oracle price) | ⚠️ `flow_oracle_tolerance` may test; unclear if it mocks negatives | Confirm oracle mock generates negative prices; add if not. |

### Recommendation

Top-priority new fuzz target: **`flow_withdraw_sign_guard.rs`** — runs every user entrypoint with `amount ∈ {-100, -1, 0, 1, 100, i128::MAX}` and asserts pool.supplied + pool.revenue invariant + controller's `total_user_scaled = pool.supplied - pool.revenue` across all outcomes. A run under the pre-fix code would flag N-02 immediately.

### Fuzz harness notes

- The existing fuzz harness at `fuzz/` should be extended to cover these. P-N02 specifically is a one-property fix that would have caught the Critical bug.
- Property-based testing (proptest / soroban-test-utils) is the natural fit for the parametric properties (P-N01 HF range, P-N09 LTV boost range).
- For budget properties (P-N08), the existing Soroban test harness can capture `env.budget()` metrics.
- Cross-chain differential harness would run each property on both Stellar and MVX and assert identical behavior (or flag intentional deviation).
