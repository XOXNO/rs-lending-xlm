# Pool ops audit report

**Run:** `pool-ops-audit` (workflow `.grok/workflows/pool-ops-audit.rhai`)  
**Date:** 2026-08-08  
**Elapsed:** ~12m  
**Agents:** 18 (budget 96)  
**Confirmed findings:** **0**  
**Units:** 12 file units + 5 cross-cutting features (all agents succeeded)

This is a **code-review audit**, not a formal proof. Empty findings mean no
agent produced a finding that survived collection (all units returned zero raw
findings before verification). Residual notes below are trust-model or design
edges, not confirmed bugs.

## Executive summary

Under the stated trust model (controller/hub is sole pool owner; measured
inbound at the hub; cash book is authoritative for reserves), auditors did not
find a reachable accounting, authorization, flash-repay, or guard-placement bug
in `contracts/pool/src/ops/`.

The only ops peer edge is **`strategy` → `borrow::mint_debt`**. That path was
audited both as a unit and as feature `feat-shared-mint-debt`.

## Coverage

| Unit | Agent OK | Files inspected | Findings (raw / kept) | Status |
|------|----------|----------------:|-----------------------|--------|
| mod | yes | 15 | 0 / 0 | clean |
| supply | yes | 20 | 0 / 0 | clean |
| borrow | yes | 16 | 0 / 0 | clean |
| repay | yes | 19 | 0 / 0 | clean |
| withdraw | yes | 20 | 0 / 0 | clean |
| flash | yes | 12 | 0 / 0 | clean |
| strategy | yes | 20 | 0 / 0 | clean |
| seize | yes | 20 | 0 / 0 | clean |
| net_settle | yes | 15 | 0 / 0 | clean |
| recapitalize | yes | 17 | 0 / 0 | clean |
| revenue | yes | 19 | 0 / 0 | clean |
| market | yes | 12 | 0 / 0 | clean |
| feat-shared-mint-debt | yes | 20 | 0 / 0 | clean |
| feat-cash-vs-token | yes | 15 | 0 / 0 | clean |
| feat-accrual-timing | yes | 20 | 0 / 0 | clean |
| feat-guards-placement | yes | 14 | 0 / 0 | clean |
| feat-batch-orchestration | yes | 20 | 0 / 0 | clean |

## Context (from inventory agent)

- **Trust:** Controller owns the pool; mutators are `#[only_owner]`. Users do not call pool mutators in production.
- **Cash vs token:** Tracked `cash` is authority for reserves (INV-ACCT-02). Flash uniquely brackets SAC balance.
- **Shares:** RAY-scaled supply/debt; protocol revenue ⊂ supplied until claim (INV-ACCT-01).
- **Peer edges:** `strategy → borrow::mint_debt` only.

## Per-unit residual notes (not findings)

| Unit | What was checked | Residual / design edges |
|------|------------------|-------------------------|
| **mod** | Renew-before-legs; sync via `synced_market`/`load_leg`; batch snapshots; empty emit no-op | Compromised hub can pass bad positions / huge batches (budget); not external |
| **supply** | Backed-market pre-mint; cash credit; zero-share reject; first-deposit / floor index | Pool does not remeasure SAC (hub INV-ACCT-03) |
| **borrow** | Reserves, util post-mint, zero-share, debit = transfer, commit-before-transfer | Util skipped when `supplied == 0`; hub position forgery out of scope |
| **repay** | Full/partial, overpayment refund, zero-share, burn-then-credit | Hub must pre-fund; half-up underpay can leave dust by design |
| **withdraw** | Liq fee → revenue; util skip only on liq; solvent always; reserves on **net** | Full last-supply exit blocked if debt remains; non-liq ignores `protocol_fee` |
| **flash** | Flashloanable gate; balance bracketing; allowance pull; fee → revenue+cash | Malicious listed token; owner/direct pool calls if owner compromised |
| **strategy** | Fee ≤ principal; cash debit `amount-fee`; shared `mint_debt` | Dust fee shares may floor to 0; hub-only |
| **seize** | Bad-debt ceil + index floor; deposit absorb; double-seize | No nonneg check on `scaled_amount` if hub is malicious |
| **net_settle** | Debt-ceil cap; cash immutable; dual-leg zero-share; solvent | Util gate intentionally absent; trust hub positions |
| **recapitalize** | `min(amount, shortfall)`; refund excess; cash += applied only | Trust hub pre-fund; no SAC bracketing on this path |
| **revenue** | `min(cash, floor revenue)`; util+solvent after burn; pay Ownable owner | Sub-unit revenue dust until index grows; custom-token reentrancy not fully modeled |
| **market** | Create dup reject; RAY init; replace accrues under **old** curve first | `update_params` accrues state but emits params event only (observability) |

## Cross-cutting residuals

| Feature | Residual |
|---------|----------|
| **shared mint_debt** | Dual-path cash handling intentional (borrow full vs strategy net-of-fee); util gate before fee supply mint is stricter than final state |
| **cash vs token** | Concurrent multi-market same-SAC conservation under adversarial hub not fully modeled |
| **accrual timing** | Views lag stored indexes; multi-year dormancy chunk budget not re-tested in this run |
| **guards placement** | `require_backed_market` only on supply; util skip on liq/net_settle/seize/repay/flash/recap is intentional |
| **batch orchestration** | Empty findings under confirmed-reachability standard |

## Out of scope

- Controller liquidation graph (except pool-side seize / withdraw liq fee)
- Governance, oracles, swap venues
- Formal Certora re-runs (rules referenced by agents but not re-executed here)
- Remediation / code changes

## Artifacts

| Artifact | Location |
|----------|----------|
| Goal tracker | `docs/audit/pool-ops/GOAL.md` |
| This report | `docs/audit/pool-ops/REPORT.md` |
| Workflow definition | `.grok/workflows/pool-ops-audit.rhai` |
| Session scratch report | workflow run scratch `pool-ops-audit-report.md` |

## Follow-ups (kicked off 2026-08-08)

1. **Adversarial re-pass** — `/workflows` run **`pool-ops-audit-2`**: `mode=adversarial`, units `flash,strategy,seize` (+ related features).
2. **Regression test** — `tests::test_update_params_accrues_under_old_curve_after_time_advance` in `contracts/pool/tests/flows.rs` (**passing**): proves `update_params` commits indexes under the pre-update curve, then subsequent accrual follows the new curve.
3. **Controller INV-ACCT-03** — **complete, 0 findings** — `docs/audit/controller-acct03/REPORT.md`.
4. **Adversarial re-pass** — **complete, 0 findings** — `docs/audit/pool-ops/ADVERSARIAL-PASS.md` (run `pool-ops-audit-2`).
5. Hardening **implemented** (2026-08-08):
   - Hub nonneg `scaled_amount` on seize/net_settle (`require_nonneg_amount`)
   - Strategy dust + near-cap util tests in `contracts/pool/tests/flows.rs`
   - Certora full flash apply accounting: `prepare` / `prepare_with_balance` / `finalize` + rules in `flash_loan_accounting_rules.rs`
