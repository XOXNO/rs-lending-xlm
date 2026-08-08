# Pool ops adversarial re-pass (flash / strategy / seize)

**Run:** `pool-ops-audit-2`  
**Mode:** `adversarial`  
**Units:** `flash`, `strategy`, `seize`  
**Features:** shared mint_debt, cash vs token, accrual timing, guards placement, batch orchestration  
**Elapsed:** ~14m  
**Confirmed findings:** **0**  
**Parent audit:** `docs/audit/pool-ops/REPORT.md` (first pass also 0 findings)

## Executive summary

Second-pass auditors were instructed to break the units (rounding, CEI, cash/token, fee double-booking, reentrancy, malicious hub scaled amounts, util-skip abuse). All unit and feature agents returned zero raw findings. Several attack ideas were explicitly **refuted** with code evidence.

This remains a code-review audit, not a formal proof.

## Coverage

| Unit / feature | Inspected | Raw / kept | Status |
|----------------|----------:|------------|--------|
| flash | 19 | 0 / 0 | clean |
| strategy | 20 | 0 / 0 | clean |
| seize | 18 | 0 / 0 | clean |
| feat-shared-mint-debt | 18 | 0 / 0 | clean |
| feat-cash-vs-token | 20 | 0 / 0 | clean |
| feat-accrual-timing | 20 | 0 / 0 | clean |
| feat-guards-placement | 20 | 0 / 0 | clean |
| feat-batch-orchestration | 20 | 0 / 0 | clean |

## Refutations (high signal)

### flash
- Checked: flashloanable gate, exact SAC balance bracketing, allowance-pull repay, fee → revenue+cash, single pre-callback accrual + late commit, WASM receiver, owner-only.
- **Refuted:** push-repay, underpay free fee, nested controller reentry, cash/token flash sizing mismatch, failed-callback state leak.
- Residual: malicious/rebasing listed asset; fee=0 governance; Certora covers terms+book_fee more than full `apply` path.

### strategy
- Checked: fee / `charge_fee`, fee ≤ principal, cash debit `amount−fee`, revenue mint once, shared `mint_debt` util+reserves(gross), CEI transfer.
- **Refuted:** double fee, fee>principal under 500 bps, cash/token desync, util skip via fee shares, zero-share debt, reentrancy.
- Residual: util skip when `supplied==0` (shared guard); limited strategy dust util tests.

### seize
- Checked: ceil write-down + floor residual, deposit absorb no cash, double/zero seize, batch reload, negative scaled (hub-only).
- **Refuted:** residual drain (backed_market+recap), over-cap socialization, cash out on deposit seize, partial commit.
- Residual: no nonneg check on hub-supplied `scaled_amount` (malicious hub only).

### feat-shared-mint-debt
- Only call sites: `borrow::accounting`, `strategy::accounting`.
- Borrow: debit/transfer full principal. Strategy: fee as revenue (no `credit_cash`), debit/transfer net — intentional vs flash `book_fee` (which credits cash because principal is untracked during the loan).
- Util checked pre-fee supply mint (stricter than post-fee). No confirmed dual-path break.

### feat-cash-vs-token / accrual / guards / batch
- All mutators accrue before mutation; `replace_rate_model` uses old curve (pinned by `test_update_params_accrues_under_old_curve_after_time_advance`).
- Guard placement matches intentional policy (util skip on liq / net_settle / seize / repay / flash / recap).
- Batch: renew once, commit per leg, `#[only_owner]` on mutators, event topic family consistent for single vs multi-leg.

## Hardening status (implemented 2026-08-08)

1. **Hub nonneg `scaled_amount`** — `seize::apply` and `net_settle::apply` call `require_nonneg_amount`; tests `test_seize_positions_rejects_negative_scaled_amount`, `test_net_settle_rejects_negative_scaled_positions`.
2. **Strategy dust + near-cap util** — `test_create_strategy_dust_fee_consumes_entire_payout`, `test_create_strategy_above_max_utilization_panics`, `test_create_strategy_near_max_utilization_succeeds`, `test_create_strategy_fee_cannot_bypass_max_utilization`.
3. **Certora flash apply accounting** — production helpers `prepare` / `prepare_with_balance` / `finalize`; rules `flash_apply_accounting_books_fee_without_principal_cash`, `flash_apply_accounting_zero_fee_is_cash_noop` (SAC/callback still out of scope).

## Relation to other follow-ups

| Item | Status |
|------|--------|
| First pool-ops full pass | clean — `REPORT.md` |
| This adversarial pass | clean — this file |
| Rate-model regression test | **passing** |
| Controller INV-ACCT-03 | clean — `../controller-acct03/REPORT.md` |
