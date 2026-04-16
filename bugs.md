# Security Audit Report — Stellar Lending Protocol

**Date:** 2026-04-14
**Branch:** `feature/stellar-migration`
**Scope:** `common/src/`, `pool/src/`, `controller/src/`
**Methodology:** two-iteration parallel agent-based audit with manual verification and external LLM second-opinions.
- **Iteration 1:** five specialized agents (math-precision, pool-security, positions, oracle-validation, flash-loan/strategy) audited all 34 Rust source files
- **Iteration 2:** three agents (cross-module attack chains, Codex/Gemini second-opinion on C-01, Certora coverage gap analysis)

## Summary

### Current Status (2026-04-15 re-verification, uncommitted working tree on `ac05343`)

**All 21 Critical/High/Medium findings closed.** P0 sprint (2026-04-15) addressed
the last two Medium gaps (M-08, M-10) and closed 2 Low findings (L-01, L-13).
See `ACTION_PLAN.md` §1 for the full table with file:line evidence.

| Severity | Count | Status |
|----------|-------|--------|
| Critical | 0 | C-01 FIXED |
| High | 0 | H-01/H-02/H-03 demoted & FIXED; NEW-01/NEW-02/NEW-03 FIXED |
| Medium | 0 open (was 18) | M-08 FIXED via 8-term Taylor; M-10 FIXED via explicit entry checks |
| Low | 11 | L-01, L-13 FIXED this sprint; 11 remaining are low-impact or by-design |
| Informational | 12 | mostly by-design |
| Attack Chains | 7 analyzed — all exploitable chains closed |
| Certora Gaps | 6 → see §Formal Verification Coverage for P1-P4 plan |

### Post-Codex Verification (original, 2026-04-14)
| Severity | Count | Change |
|----------|-------|--------|
| Critical | 0 | C-01 fixed in working tree (uncommitted) |
| High | 2 | NEW-01 (router allowance), NEW-02 (fee-on-transfer) — NEW-03 (repay_debt_with_collateral ordering + M-11 dup) FIXED in commit `ddbebe4` |
| Medium | 18 | H-01, H-02, H-03 demoted here |
| Low | 13 | unchanged |
| Informational | 12 | unchanged |
| Attack Chains | 7 (3 exploitable, 2 blocked, 1 theoretical, 1 not feasible) |
| Certora Gaps | 6 (2 not covered, 4 partial) |

### Pre-Codex (original agent counts)
| Severity | Count |
|----------|-------|
| Critical | 1 |
| High | 3 |
| Medium | 15 |
| Low | 13 |
| Informational | 12 |

---

## CRITICAL

### C-01: Missing Access Control on `edit_e_mode_category`

**Status:** FIXED — `#[stellar_macros::only_owner]` present at `controller/src/lib.rs:481`. Regression guard: `fuzz_auth_matrix` proptest.
**File:** `controller/src/lib.rs:484`
**Module:** Controller — Config

**Description:** The `edit_e_mode_category` public endpoint is missing the `#[only_owner]` macro. Its siblings `add_e_mode_category` (line 480) and `remove_e_mode_category` (line 488) are both properly guarded. Any address can call `edit_e_mode_category` to modify LTV, liquidation threshold, and liquidation bonus of any e-mode category.

```rust
// Line 484 — NO #[only_owner]
pub fn edit_e_mode_category(env: Env, id: u32, ltv: i128, threshold: i128, bonus: i128) {
    config::edit_e_mode_category(&env, id, ltv, threshold, bonus);
}
```

**Attack Scenario:**
1. Attacker calls `edit_e_mode_category(1, 9999, 10000, 0)` to set near-100% LTV
2. All existing positions in that category become instantly liquidatable OR attacker enables extreme leverage for themselves
3. Complete manipulation of e-mode risk parameters for all accounts in any category

**Impact:** Protocol-draining leverage or mass wrongful liquidations.

**Fix:**
```rust
#[stellar_macros::only_owner]
pub fn edit_e_mode_category(env: Env, id: u32, ltv: i128, threshold: i128, bonus: i128) {
```

---

## HIGH

### H-01: `rescale_half_up` Rounds Toward Zero for Negative Values

**Status:** FIXED — sign-aware branch at `common/src/fp_core.rs:58-62`. Covered by `fp_rescale` libFuzzer target + `test_rescale_downscale_negative_rounds_away_from_zero`.
**File:** `common/src/fp_core.rs:44-58`
**Module:** Common — Fixed-Point Math

**Description:** The function always adds `half` (positive) before dividing, regardless of sign. For negative values, half-up rounding ("away from zero") requires *subtracting* half.

```rust
let half = factor / 2;
(a + half) / factor  // BUG: should be (a - half) for negative a
```

Example: `a = -15, factor = 10` → result is `-1` but should be `-2` (away from zero).

**Impact:** Any downscaling conversion applied to negative values rounds toward zero instead of away from zero. Breaks the protocol's stated rounding invariant. If used on PnL calculations or signed values, systematically favors users over the protocol.

**Fix:**
```rust
if a >= 0 { (a + half) / factor } else { (a - half) / factor }
```

### H-02: `div_by_int_half_up` Rounds Toward Zero for Negative Values

**Status:** FIXED — sign-aware branch at `common/src/fp_core.rs:72-76`. Covered by `fp_div_by_int` libFuzzer target.
**File:** `common/src/fp_core.rs:62-65`
**Module:** Common — Fixed-Point Math

**Description:** Same issue as H-01. Always adds `half_b` regardless of sign:

```rust
pub fn div_by_int_half_up(a: i128, b: i128) -> i128 {
    let half_b = b / 2;
    (a + half_b) / b  // BUG: wrong for negative a
}
```

Used by `Ray::div_by_int` in the Taylor series compound interest calculation.

**Impact:** Currently mitigated because all compound interest terms are positive. But the semantic contract is broken for any negative input, and future callers will get silently wrong results.

**Fix:**
```rust
pub fn div_by_int_half_up(a: i128, b: i128) -> i128 {
    debug_assert!(b > 0);
    let half_b = b / 2;
    if a >= 0 { (a + half_b) / b } else { (a - half_b) / b }
}
```

### H-03: Stale Price Bypass for Supply/Repay Operations

**Status:** FIXED — `controller/src/oracle/mod.rs:164-169` now enforces staleness unconditionally; `allow_unsafe_price` no longer gates `check_staleness`.
**File:** `controller/src/oracle/mod.rs:157-161`
**Module:** Controller — Oracle

**Description:** `check_staleness` only enforces staleness when `allow_unsafe_price == false`. Supply and repay operations set `allow_unsafe_price = true`, accepting arbitrarily stale prices.

```rust
if now_secs > feed_ts && (now_secs - feed_ts) > max_stale && !cache.allow_unsafe_price {
    panic_with_error!(cache.env(), OracleError::PriceFeedStale);
}
```

**Attack Scenario:**
1. Oracle goes offline, real market price of collateral drops 50%
2. Attacker supplies collateral using the stale (higher) price
3. Oracle comes back online; attacker borrows against the now-lower real price
4. Attacker extracted more value than collateral is actually worth

**Impact:** Stale price exploitation on supply operations. Supply/repay should tolerate wider *deviations* but not arbitrarily *old* prices.

**Fix:** Always enforce staleness, regardless of `allow_unsafe_price`:
```rust
fn check_staleness(cache: &ControllerCache, feed_ts: u64, max_stale: u64) {
    let now_secs = cache.current_timestamp_ms / 1000;
    if now_secs > feed_ts && (now_secs - feed_ts) > max_stale {
        panic_with_error!(cache.env(), OracleError::PriceFeedStale);
    }
}
```

---

## MEDIUM

### M-01: Future Timestamp Bypass in Staleness Check

**Status:** FIXED — `check_not_future` at `controller/src/oracle/mod.rs:174-179` rejects `feed_ts > now + 60`.
**File:** `controller/src/oracle/mod.rs:157-161`
**Module:** Controller — Oracle

**Description:** Staleness check only fires when `now_secs > feed_ts`. A compromised oracle could submit prices with far-future timestamps, bypassing staleness forever.

**Fix:** Reject `feed_ts > now_secs + 60` (small clock skew tolerance).

### M-02: No Zero/Negative Price Validation from Oracle

**Status:** FIXED — `controller/src/oracle/mod.rs:42-43` panics with `InvalidPrice` on `price <= 0`.
**File:** `controller/src/oracle/mod.rs:41-48, 163-183`
**Module:** Controller — Oracle

**Description:** Neither spot nor TWAP fetchers validate that returned prices are positive. A zero price for a borrowed asset makes health factor infinite (division produces MAX), making underwater positions appear healthy.

**Fix:** `if feed.price_wad <= 0 { panic_with_error!(env, OracleError::InvalidPrice); }`

### M-03: TWAP Degrades to Single Observation with Partial History

**Status:** FIXED — `min_required = max(1, twap_records / 2)` at `controller/src/oracle/mod.rs:246,299`.
**File:** `controller/src/oracle/mod.rs:212-232`
**Module:** Controller — Oracle

**Description:** If most TWAP history slots are `None` (oracle gaps), the TWAP is computed from as few as 1 observation, providing zero smoothing benefit.

**Fix:** Require `count >= twap_records / 2` minimum valid observations.

### M-04: No Validation Bounds on `max_price_stale_seconds`

**Status:** FIXED — `controller/src/config.rs:360` enforces `[60, 86_400]`.
**File:** `controller/src/config.rs:373-384`
**Module:** Controller — Config

**Description:** Oracle role can set `max_price_stale_seconds` to 0 (DoS) or `u64::MAX` (disable staleness). No bounds validation.

**Fix:** Enforce `MIN_PRICE_STALE_SECONDS = 60` to `MAX_PRICE_STALE_SECONDS = 86400`.

### M-05: Conflicting Tolerance Cap Constants

**Status:** FIXED — `common/src/constants.rs:26` now `MAX_LAST_TOLERANCE = 5_000` aligned with validation. Stale comment in `controller/certora/spec/oracle_rules.rs:269` corrected 2026-04-15.
**File:** `controller/src/validation.rs:134-142` vs `constants.rs`
**Module:** Controller — Validation

**Description:** `MAX_LAST_TOLERANCE = 10_000` (100%) is defined but `validate_oracle_bounds` rejects `last >= 5000` (50%). The constant is unreachable and misleading. Additionally, 50% tolerance is excessively permissive for most assets.

### M-06: `rescale_half_up` Overflow on Large Upscale

**Status:** FIXED — `checked_mul` + descriptive panic at `common/src/fp_core.rs:52-53`.
**File:** `common/src/fp_core.rs:48-51`
**Module:** Common — Fixed-Point Math

**Description:** Upscaling uses `a * factor` in native i128 without checked arithmetic. Large token amounts (e.g., meme tokens with 0 decimals) converted to RAY precision (27 decimals) can overflow. Soroban panics on overflow but produces an opaque error instead of `MathOverflow`.

**Fix:** Use `checked_mul` with descriptive error, or use I256 for upscaling.

### M-07: Division by Zero in `calculate_borrow_rate`

**Status:** FIXED — `validate_interest_rate_model` at `controller/src/validation.rs:100-108` enforces `mid > 0`, `optimal > mid`, `optimal < RAY`.
**File:** `common/src/rates.rs:7-42`
**Module:** Common — Interest Rates

**Description:** Region 1 divides by `mid`, Region 2 by `optimal - mid`, Region 3 by `1 - optimal`. If `mid = 0`, `optimal = mid`, or `optimal = 1.0 RAY`, the function panics with an opaque I256 trap.

**Fix:** Guard at math layer: `if mid == Ray::ZERO { panic_with_error!(env, InvalidBorrowParams); }`

### M-08: Taylor 5-Term Expansion Insufficient for Extended Stale Periods

**Status:** FIXED — `common/src/rates.rs:59-94` now uses 8-term Taylor expansion. Error drops from ~1.66 % at x=2 (5 terms) to < 0.01 % at x=2 (8 terms). Adds 3 extra multiplications per `compound_interest` call.
**File:** `common/src/rates.rs:59-86`
**Module:** Common — Interest Rates

**Description:** For `x > 2` (100% rate over 2+ years without index update), error exceeds 1.66%. For `x = 3`, error is 18.5%. Systematically under-calculates interest, benefiting borrowers.

**Fix:** Add more Taylor terms (8-10), or cap `delta_ms` at 1 year forcing periodic index updates.

### M-09: `saturating_sub` in `swap_tokens` Silently Returns Zero

**Status:** FIXED — swap output uses `checked_sub` at `controller/src/strategy.rs:488-490` and `expect("swap output went down")`; incoming side uses explicit `>` checks at 472-478.
**File:** `controller/src/strategy.rs:446`
**Module:** Controller — Strategy

**Description:** If a buggy aggregator decreases the controller's token balance, `balance_after.saturating_sub(balance_before)` silently returns 0. The caller proceeds with zero swap output.

**Fix:** Use `checked_sub` + explicit minimum output validation.

### M-10: No Validation That `amount_out_min > 0` in Strategy Swaps

**Status:** FIXED — explicit `require_amount_positive(env, steps.amount_out_min)` at entry of `process_multiply` (`strategy.rs:73`), `process_swap_debt` (`239`), `process_swap_collateral` (`365`), and `process_repay_debt_with_collateral` (`515`, skipped on same-asset short-circuit). Regression covered by `fuzz_strategy_flashloan::prop_strategy_swap_collateral_balance_delta`.
**File:** `controller/src/strategy.rs:162-168`
**Module:** Controller — Strategy

**Description:** Users can set `amount_out_min = 0`, fully exposing strategy swaps to MEV sandwich attacks. The controller holds tokens and executes on behalf of users.

**Fix:** `if steps.amount_out_min <= 0 { panic_with_error!(env, InvalidMinOutput); }`

### M-11: Strategy `swap_collateral` Uses Requested Amount, Not Actual Withdrawn

**Status:** FIXED — balance-delta pattern at `controller/src/strategy.rs:380-399` and `545-564`.
**File:** `controller/src/strategy.rs:389`
**Module:** Controller — Strategy

**Description:** `from_amount` (requested) is passed to `swap_tokens`, but actual withdrawal may differ due to scaled amount rounding. If mismatch occurs, the swap fails at token transfer with an unclear error.

**Fix:** Use balance-delta pattern: record balance before/after withdrawal, pass actual received amount.

### M-12: Pool Subtraction Underflow on Scaled Totals

**Status:** FIXED — `saturating_sub_ray` helper at `pool/src/lib.rs:44` used on all pool-total subtractions (`214`, `262`, `498-499`).
**File:** `pool/src/lib.rs:187, 235, 450-451`
**Module:** Pool — Core

**Description:** `cache.supplied - scaled_withdrawal` panics if rounding accumulation causes `sum(user_scaled) > total_scaled`. Over many small operations, rounding could theoretically cause the last withdrawer to be permanently stuck.

**Fix:** Use saturating subtraction for pool-level totals as safety net.

### M-13: Flash Loan Has No Balance Verification

**Status:** FIXED — `balance_after < pre_balance + fee` check at `pool/src/lib.rs:364-365`.
**File:** `pool/src/lib.rs:306-327`
**Module:** Pool — Flash Loan

**Description:** `flash_loan_end` trusts the controller to pass the correct amount. No independent balance check. Vulnerable to fee-on-transfer tokens where actual received < expected.

**Fix:** Record pre-flash balance in `flash_loan_begin`, verify `balance_after >= balance_before + fee` in `flash_loan_end`.

### M-14: Withdraw May Leave Permanently Stuck Dust

**Status:** FIXED — dust-lock guard at `pool/src/lib.rs:187-196` promotes partial to full withdrawal when residual rounds to zero. Regression property in `fuzz_ttl_keepalive`.
**File:** `pool/src/lib.rs:160-170`
**Module:** Pool — Withdraw

**Description:** If the controller's computed amount differs from the pool's `calculate_original_supply` by 1 unit (rounding), the full withdrawal condition fails. The remaining scaled dust rounds to 0 tokens, making it impossible to withdraw.

**Fix:** After partial withdrawal, check if remaining scaled amount rounds to 0 in asset decimals. If so, treat as full withdrawal.

---

## LOW

### L-01: `calculate_deposit_rate` Accepts `reserve_factor_bps > 10000`
**Status:** FIXED — defense-in-depth clamp at `common/src/rates.rs:58-60` returns `Ray::ZERO` on `reserve_factor < 0 || >= BPS`. Upstream validation still rejects invalid config at source; this is a belt-and-suspenders guard.
**File:** `common/src/rates.rs:44-57` — Silently computes nonsensical negative rewards.

### L-02: `apply_to_wad` Double Rounding
**File:** `common/src/fp.rs:193-197` — Two rounding steps where one suffices. Max error ~2 WAD units.

### L-03: No Overflow Protection in Add/Sub Trait Implementations
**File:** `common/src/fp.rs:74-86` — Relies on Soroban overflow-checks producing opaque panics.

### L-04: Pool `update_params` Missing Slope/Mid Validation
**File:** `pool/src/lib.rs:460-527` — No check for slopes >= 0 or mid_utilization > 0.

### L-05: Pool Endpoints Lack Defense-in-Depth `amount > 0` Checks
**File:** `pool/src/lib.rs` — All mutating endpoints trust controller for positivity.

### L-06: `add_protocol_revenue_ray` Explodes Near Floor Supply Index
**File:** `pool/src/interest.rs:57-64` — If supply_index ~= 10^-27 (post bad-debt floor), fee division produces astronomical values.

### L-07: Repay Full Position May Underpay by 0.5 Asset Unit
**File:** `pool/src/lib.rs:221-256` — Half-up rounding on `to_asset()` can round down, shortchanging pool.

### L-08: Isolated Debt Tracking Inconsistency (Direct Storage vs Cache)
**File:** `controller/src/positions/borrow.rs:202-235` — `handle_isolated_debt` writes storage directly while the rest of the protocol uses cache. If borrow+repay combine in one tx, cache flush overwrites the direct write.

### L-09: Borrow Cap Uses Non-Saturating Addition
**File:** `controller/src/positions/borrow.rs:280` — Uses `+` while supply cap uses `saturating_add`. Inconsistent.

### L-10: E-Mode Does Not Override `liquidation_fees_bps`
**File:** `controller/src/positions/emode.rs:11-27` — E-mode users pay base liquidation fees, not e-mode-specific fees.

### L-11: `validate_interest_rate_model` Allows `mid_utilization_ray = 0`
**File:** `controller/src/validation.rs:90-108` — Causes division by zero in Region 1 rate calculation.

### L-12: No Combined Validation of `liquidation_bonus_bps + liquidation_fees_bps`
**File:** `controller/src/validation.rs:111-132` — Sum can be unreasonable (e.g., 15% bonus + 90% fees).

### L-13: `PositionLimits` Has No Bounds Validation
**Status:** FIXED — `controller/src/config.rs:92-108` rejects `max_*_positions == 0` (DoS) and `> 32` (gas exhaustion) with new `GenericError::InvalidPositionLimits` (code 36).
**File:** `controller/src/config.rs:91-93` — Admin can set max_positions to 0 (DoS) or u32::MAX (gas exhaustion).

---

## INFORMATIONAL

### I-01: TWAP Average Uses Integer Truncation Instead of Half-Up Rounding
**File:** `controller/src/oracle/mod.rs:230` — `sum / count` truncates. Inconsistent with "half-up everywhere" policy.

### I-02: `is_within_anchor` Parameter Naming Confusion
**File:** `controller/src/oracle/mod.rs:310-325` — Computes `safe/aggregator` but naming suggests inverse.

### I-03: View Functions Use Permissive Price Settings Without Caveat
**File:** `controller/src/views.rs:19,44,74` — Views accept stale/deviated prices. Off-chain bots may make incorrect decisions.

### I-04: `SpotOnly` Exchange Source Bypasses All Tolerance Validation
**File:** `controller/src/oracle/mod.rs:75-78` — Dev/test mode with no compile-time gate for production.

### I-05: `disable_token_oracle` Blocks Liquidations for Disabled Markets
**File:** `controller/src/config.rs:431-435` — Emergency kill switch prevents liquidation, risking bad debt accumulation.

### I-06: Supply Index Update Has Extra Rounding vs Simplified Form
**File:** `common/src/rates.rs:92-106` — ~2-3 RAY units per update. Negligible.

### I-07: `claim_revenue` Ratio Uses `Ray::from_raw` on Asset-Decimal Values
**File:** `pool/src/lib.rs:431-437` — Mathematically correct but semantically confusing. Refactoring risk.

### I-08: Flash Loan Fee Can Be Zero
**File:** `controller/src/flash_loan.rs:37` — If `flashloan_fee_bps = 0`, free flash loans enabled. Governance decision.

### I-09: Account Nonce Has No Overflow Check
**File:** `controller/src/storage/mod.rs:98-105` — u64 overflow requires ~18 quintillion accounts. Unreachable.

### I-10: Strategy `process_repay_debt_with_collateral` Has Redundant Double HF Check
**File:** `controller/src/strategy.rs:539-564` — Manual HF check + `strategy_finalize` HF check. Wasted compute.

### I-11: Strategy Operations Missing Explicit `require_market_active` Checks
**File:** `controller/src/strategy.rs` — Rely on downstream oracle failures as implicit guard.

### I-12: `clean_bad_debt_standalone` Has No Access Control
**File:** `controller/src/positions/liquidation.rs:440-469` — By design (public good), but worth noting for audit trail.

---

## Threat Model Summary

### Attack Vectors Analyzed

| Vector | Status | Relevant Findings |
|--------|--------|-------------------|
| **E-Mode Parameter Manipulation** | VULNERABLE | C-01 |
| **Stale Price Exploitation** | VULNERABLE | H-03, M-01 |
| **Signed Rounding Exploitation** | LATENT RISK | H-01, H-02 |
| **Oracle Manipulation (zero price)** | VULNERABLE | M-02 |
| **TWAP Degradation Attack** | VULNERABLE | M-03 |
| **Flash Loan Reentrancy** | SAFE | Soroban atomic txs |
| **First Depositor Attack** | SAFE | Index initialized at RAY |
| **MEV Sandwich on Strategy** | VULNERABLE | M-10 |
| **Dust Position Lock** | VULNERABLE | M-14 |
| **Withdrawal Underflow DoS** | LATENT RISK | M-12 |
| **Self-Liquidation Profit** | SAFE | Bonus < seized value |
| **Position Count Bypass** | SAFE | Validated on insert |
| **Isolation Mode Escape** | SAFE | Validated in supply/borrow |
| **E-Mode / Isolation XOR** | SAFE | Validated on account creation |
| **Flash Loan Nesting** | SAFE | Guard in Instance storage |
| **Admin Privilege Escalation** | PARTIAL (C-01) | Missing auth on one endpoint |
| **Fee-on-Transfer Token** | VULNERABLE | M-13 |
| **Bad Debt Gaming** | LOW RISK | $5 threshold reasonable |

---

## Cross-Module Attack Chains (Iteration 2)

### Chain A: Stale Price Supply → Fresh Price Borrow — **BLOCKED**
Supply records `scaled_amount_ray` (not USD snapshots). All value calculations use fresh prices at query time. Even if attacker supplies at stale high price, borrow-time LTV/HF check uses fresh prices. No exploit path.

### Chain B: E-Mode Manipulation → Mass Liquidation — **PARTIALLY EXPLOITABLE**
Via C-01, attacker lowers liquidation_threshold for a category. Existing positions use stored thresholds (not live e-mode params) until refreshed via `update_position_threshold`. The 1.05 HF floor on threshold updates limits damage — attacker can only make accounts vulnerable to subsequent small price movements, not instant-liquidate.

### Chain C: E-Mode Manipulation → Over-Leverage → Protocol Drain — **EXPLOITABLE (CRITICAL)**
1. Attacker calls unprotected `edit_e_mode_category` to set LTV=99%, threshold=99.5%
2. Creates new account in that category, supplies $100k collateral
3. Borrows $99k at 99% LTV (passes `validate_ltv_collateral`)
4. Resets e-mode params. Position persists with $100k collateral / $99k debt
5. Any 1-2% price movement creates insolvency → bad debt socialized to suppliers
6. **With strategy/multiply**: $10k initial capital → ~$1M leveraged position → catastrophic bad debt

**Second-opinion confirmation:** Both Codex (gpt-5.3-codex, 99% confidence) and Gemini (gemini-3.1-pro) independently confirmed C-01 is genuine — no alternative auth mechanism exists.

### Chain D: Zero Oracle Price → Infinite Health Factor — **THEORETICAL (HIGH)**
If borrow asset gets price=0: `total_borrow = 0` → `HF = i128::MAX` → underwater position appears healthy, liquidation blocked. If supply asset gets price=0: `total_collateral = 0` → `HF = 0` → mass wrongful liquidation of all holders.

### Chain E: Flash Loan + Strategy Sandwich — **BLOCKED**
`require_not_flash_loaning(env)` is checked in all strategy endpoints. Cannot chain flash loans with strategy operations within the same transaction.

### Chain F: Dust Accumulation → Pool DoS — **NOT EXPLOITABLE**
Each operation introduces ≤1 RAY unit (10^-27) rounding error. To create 1-token discrepancy requires ~10^20 operations. Not feasible on any blockchain.

### Chain G: Bad Debt + Supply Index Collapse → Pool Bricked — **EXPLOITABLE (HIGH)**
1. Major bad debt event (or via Chain C) triggers `apply_bad_debt_to_supply_index`
2. Supply index drops to floor `Ray::from_raw(1)` (= 10^-27)
3. Any subsequent `calculate_scaled_supply(amount)` computes `amount * 10^47` → **overflows i128**
4. Pool permanently bricked: no new supply possible, existing withdrawals return near-zero dust
5. **Combined with Chain C**: attacker intentionally triggers bad debt to destroy a pool

**Fix:** Raise supply_index floor to `Ray::from_raw(10^18)` (10^-9 in decimal) preventing overflow for reasonable amounts. Or pause pool when bad debt exceeds threshold percentage of total supply.

---

## Certora Formal Verification Coverage Gaps

| Vulnerability | Spec Coverage | Gap Severity |
|---|---|---|
| **C-01** Missing access control | **NOT COVERED** — zero access control specs exist | CRITICAL |
| **H-03** Stale price bypass | **PARTIAL** — tolerance arithmetic tested, staleness enforcement not | HIGH |
| **M-02** Zero price | **NOT COVERED** — zero/negative price edge case absent | MEDIUM |
| **M-07** Division by zero in rates | **PARTIAL** — valid params assumed via `cvlr_assume!`, invalid params not tested at config boundary | MEDIUM |
| **M-12** Subtraction underflow | **PARTIAL** — per-op deltas tested, global `sum(user_scaled) <= total` not | MEDIUM |
| **M-14** Dust lock | **NOT COVERED** — roundtrip arithmetic tested, actual full withdrawal flow not | MEDIUM |

### Systemic Spec Gaps
1. **Zero access control verification** — No rules test that governance endpoints require auth
2. **No adversarial oracle testing** — Specs never test zero/negative/extreme prices or timestamp manipulation
3. **No end-to-end flow invariants** — Specs test arithmetic or single operations, not multi-step attack sequences

---

## Codex Second-Opinion Verification (Iteration 3)

External Codex review (gpt-5.3-codex, 2.6M tokens reasoning) independently verified all Critical/High findings against the working tree. Verdicts:

### Status Changes from Codex Review

| Finding | Original | Codex Verdict | Reason |
|---------|----------|---------------|--------|
| **C-01** | Critical | **FIXED in working tree** | `#[only_owner]` added uncommitted at `lib.rs:484`. Was genuine at HEAD commit, now resolved. User/agent patched during loop. Still needs commit. |
| **H-01** | High | **Genuine, severity overstated** | Live call sites are non-negative; exploit surface limited. Keep fix but demote to Medium. |
| **H-02** | High | **Genuine, severity overstated** | Only used in positive-rate Taylor math. Keep fix but demote to Medium. |
| **H-03** | High | **Partially genuine — attack chain blocked** | Staleness bypass exists on supply/repay, BUT borrow uses `allow_unsafe_price=false` and re-prices at LTV check. Supply doesn't use price for accounting. **Real impact:** isolated-debt USD drift on repay path (`repay.rs:119-129`, `utils.rs:74-91`), not the original Chain A scenario. Demote to Medium. |

### Codex-Confirmed Medium Findings
M-01, M-02, M-04, M-06, M-07, M-11 all confirmed genuine with exact file:line evidence.

---

## NEW FINDINGS (Missed by Iteration 1-2)

### NEW-01: HIGH — Lingering Router Allowance in Strategy Swaps

**Status:** FIXED — balance-delta spend check at `controller/src/strategy.rs:476-479` + explicit `approve(..., 0, 0)` at 483 to zero residual allowance. Regression in `fuzz_strategy_flashloan`.
**File:** `controller/src/strategy.rs:425-446`
**Module:** Controller — Strategy

**Description:** Strategy swap path approves the aggregator router at lines 425-432, but never clears the allowance or verifies exact spend after the call at lines 434-446. The mock aggregator proves the intended spend path is `transfer_from` against that allowance (`test-harness/src/mock_aggregator.rs:22-29`).

**Attack Scenario:**
1. User initiates strategy operation (multiply/swap_collateral/swap_debt)
2. Controller approves router for `amount_in`
3. Router/aggregator under-spends (e.g., takes only `amount_in / 2`)
4. Controller has `amount_in / 2` residual allowance remaining
5. Malicious aggregator (or future-listed one) later calls `transfer_from` to drain tokens the controller accumulated from other operations

**Impact:** Token drain from controller-held balances. Exploitable by any malicious listed aggregator.

**Fix:**
```rust
// After swap completes
tok_in_client.approve(&controller, &router, &0, &env.ledger().sequence()); // Zero out
// OR verify exact spend:
let balance_in_after = tok_in_client.balance(&controller);
assert_eq!(balance_in_before - balance_in_after, expected_amount_in);
```

### NEW-03: HIGH — `repay_debt_with_collateral` uses requested (not actual) withdrawal + checks debt position after transferring tokens — FIXED

**File:** `controller/src/strategy.rs:523-559` (pre-fix)
**Module:** Controller — Strategy
**Surfaced by:** Trail-of-Bits test-suite audit, 2026-04-15
**Fixed in:** commit `ddbebe4`

**Description:** Two defects in the same function:

1. `swap_tokens(... collateral_amount ...)` received the REQUESTED amount, not the actual delta withdrawn from the pool. When the pool delivered less than requested (rounding, dust floors), the router approve + `transfer_from` pulled more than the controller held, failing opaquely inside the token contract. Same pattern as M-11 (fixed for `swap_collateral` at `strategy.rs:396-400`) but never ported to `process_repay_debt_with_collateral`.

2. `debt_tok.transfer(controller -> debt_pool, swapped_debt)` ran BEFORE `borrow_positions.get(debt_token)` existence check. When a caller passed a debt token they did not owe, tokens transferred to the pool first, then the missing-position guard at line 559 was unreachable — the transfer either host-panicked or stranded tokens in the pool.

**Impact:**
- Fail-fast semantics broken: callers see opaque host errors instead of the intended `DebtPositionNotFound (120)` code.
- Dust scenarios fail unpredictably: any pool rounding causes the strategy to revert inside the router rather than surfacing a clean controller-level error.
- Stranded tokens possible in the missing-debt-position path.

**Fix:** Validate both `collateral_pos` and `debt_pos` up front, then withdraw, measure actual delta via balance snapshot, swap on the actual amount, and transfer + repay. Mirrors the ordering in `process_swap_collateral`.

**Regression test:** `test_repay_debt_with_collateral_missing_debt_rejects` in `test-harness/tests/strategy_panic_coverage_tests.rs` — now asserts `DEBT_POSITION_NOT_FOUND (120)` rather than the opaque pre-fix error.

---

### NEW-02: HIGH — Fee-on-Transfer / Rebasing Token Breaks All Accounting

**Status:** FIXED — admin allow-list (`is_token_approved`) at `controller/src/router.rs:58-63`. Only pre-approved vanilla SAC tokens can become markets. Regression in `fuzz_auth_matrix`.
**File:** `controller/src/router.rs:42-50`, `controller/src/positions/supply.rs:200-202`, `pool/src/lib.rs:92-106`
**Module:** Controller + Pool — Token Handling

**Description:** Market creation validates only `decimals()` and `symbol()`. No allow-list of known token implementations. Supply flow:
1. `supply.rs:200-202` transfers `amount` from user to pool
2. `pool/lib.rs:92-106` mints scaled claims based on **nominal** `amount`

If the token is fee-on-transfer or rebasing, the pool receives less than `amount` but mints claims for the full `amount`. The same "exact transfer" assumption exists in flash-loan and strategy flows.

**Attack Scenario:**
1. Admin lists a fee-on-transfer token (intentionally or mistakenly)
2. User supplies 1000 tokens, 10% transfer fee
3. Pool receives 900 tokens but books 1000 scaled claims
4. Pool becomes insolvent: 1000 claims against 900 reserves
5. Last 100 withdrawals fail; earlier withdrawers stole value

**Fix:** Either (a) maintain a strict allow-list of known vanilla SAC tokens, or (b) convert all transfer flows to balance-delta accounting:
```rust
let balance_before = token.balance(&pool);
token.transfer(&user, &pool, &amount);
let actual_received = token.balance(&pool) - balance_before;
// Mint claims based on actual_received, not amount
```

---

## Priority Remediation Order (REVISED after Codex)

### Before ANY Testnet Deploy
1. **C-01** — COMMIT the `#[only_owner]` fix on `edit_e_mode_category` (already in working tree)
2. **M-02** — Reject `price <= 0` centrally in oracle module
3. **M-01** — Reject future oracle timestamps (add clock skew tolerance)
4. **H-03 (demoted)** — Enforce staleness always, even for supply/repay
5. **NEW-01** — Clear router allowance after strategy swaps OR verify exact spend
6. **NEW-02** — Decide: strict token allow-list OR balance-delta accounting everywhere
7. **M-07** — Add `mid > 0`, `optimal > mid`, `optimal < 1` guards in BOTH `validation.rs:90-108` AND `pool/lib.rs:479-495`

### Before Mainnet
8. **H-01, H-02 (demoted to Medium)** — Fix signed rounding in `fp_core.rs` or wrap behind signed-safe helpers
9. **M-04** — Bound `max_price_stale_seconds` (min 60s, max 24h)
10. **M-06** — Checked arithmetic in `rescale_half_up` upscale path
11. **M-11** — Use actual withdrawn amount in `swap_collateral`, not requested
12. **M-14** — Handle withdraw dust lock
13. **M-12** — Saturating subtraction on pool totals
14. **M-03** — Minimum TWAP observation count

### Nice to Have
15. Clean up contradictory tolerance caps (`common/src/constants.rs:19-25` vs `validation.rs:134-142`)
16. Add a successful flash-loan round-trip test (current tests only verify failure cases — `test-harness/tests/flash_loan_tests.rs:10-12,28-41`)
17. Certora specs: add access control coverage, adversarial oracle tests, multi-step invariants
18. Remaining Low/Info findings

---

## ORIGINAL Priority (pre-Codex, for reference)

1. **C-01** — Add `#[only_owner]` to `edit_e_mode_category` (1 line fix, blocks Chains B, C, and amplified G)
2. **H-03** — Always enforce staleness regardless of operation type
3. **M-02** — Validate oracle prices are positive (blocks Chain D)
4. **L-06 → upgrade to M-15** — Raise supply_index floor from `Ray::from_raw(1)` to `Ray::from_raw(10^18)` (blocks Chain G)
5. **H-01, H-02** — Fix signed rounding in `rescale_half_up` and `div_by_int_half_up`
6. **M-01** — Reject future oracle timestamps
7. **M-14** — Handle withdraw dust to prevent stuck positions
8. **M-12** — Saturating subtraction on pool totals
9. **M-03** — Minimum TWAP observation count
10. **M-09, M-10** — Strategy swap safety (saturating_sub, min output)
11. **Certora** — Add access control specs, adversarial oracle specs, and multi-step invariants
12. **Remaining Medium findings** — Address before mainnet

---

## Formal Verification Coverage (Certora)

As of 2026-04-16, 13 confs / 183 rules submitted to the Certora prover.
Commit `8d04d6b` rewrote the 6 CRITICAL unsound rules to invoke
production code via `calculate_health_factor_for`, `total_*_in_usd`,
`token_price`, and `crate::oracle::is_within_anchor`.

### Follow-up work (ranked by priority)

**P1 — HIGH rule-claim-vs-assertion gaps** (~6-8h):
- Index monotonicity rules (`index_rules.rs:58,82`) need to invoke the
  public `update_indexes` keeper endpoint, not the internal accrual path.
- `flash_loan_fee_collected` (`flash_loan_rules.rs:28`) ends in
  `cvlr_satisfy!(true)`; needs pre/post revenue delta.
- `claim_revenue_transfers_to_accumulator` (`strategy_rules.rs:474`)
  asserts only `>= 0`; needs accumulator balance delta.
- `clean_bad_debt_requires_qualification` (`strategy_rules.rs:424`) uses
  HF as the predicate; real predicate is `debt > coll && coll <= $5`.
- `clean_bad_debt_zeros_positions` (`strategy_rules.rs:451`) calls
  internal helper, not keeper-only endpoint; misses auth + pause
  guards.
- `swap_debt_conserves_debt_value`, `swap_collateral_conserves_collateral`,
  `repay_with_collateral_reduces_both` — all need USD-value conservation
  instead of "decreased/exists" assertions.
- `supply_scaled_conservation` / `borrow_scaled_conservation` — need
  `delta == amount * RAY / index` equality within rounding.
- `borrow_exact_reserves` / `withdraw_more_than_position` — pure local
  tautologies; must invoke real endpoints.

**P2 — Coverage categories entirely missing** (~10-14h):
- **Access control**: zero rules for 22 `#[only_owner]` + 11 `#[only_role]`
  endpoints. C-01 class (missing auth decorator) uncovered by spec.
- **Adversarial oracle inputs**: no rule exercises zero price, future
  timestamp, or stale price through real endpoints (H-03, M-01, M-02
  uncovered by spec).
- **Token-behavior bugs in strategy**: no rule for leftover allowance
  (NEW-01), fee-on-transfer (NEW-02), rebasing, balance-delta accounting.
- **Global accounting / liveness**: no inductive invariant for
  `sum(user_scaled) <= pool_total_scaled`; no proof that a healthy user
  can always exit (M-12 / M-14 uncovered by spec).

**P3 — MEDIUM weak bounds** (~2h):
- `tolerance_bounds_valid` assumes+re-asserts; move to config validation.
- `ideal_repayment_targets_102` never computes post-liquidation HF.
- `compound_interest_bounded_output` bound is 100,000× RAY (doc says
  100×); tighten to derived max.

**P4 — 30 `*_sanity` rules using `cvlr_satisfy!(true)`**:
- Not actively harmful but don't witness reachability. Candidates for
  replacement with real-state witnesses, or deletion.

See codex rules review for full analysis. Total to reach auditor-grade
trustworthy baseline: ~25-33 hours focused work.
