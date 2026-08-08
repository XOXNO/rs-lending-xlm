# Goal: Pool ops security & correctness audit

**Status:** complete (first full pass)  
**Scope:** `contracts/pool/src/ops/` (+ shared deps only as needed to judge each leg)  
**Started:** 2026-08-08  
**Completed:** 2026-08-08  
**Driver:** `.grok/workflows/pool-ops-audit.rhai`  
**Run:** `pool-ops-audit` — agents=18, budget=96, ~12m  
**Report:** `docs/audit/pool-ops/REPORT.md`  
**Confirmed findings:** 0  

## Objective

Produce a severity-ordered, adversarially verified audit of every pool **ops** module and cross-cutting **feature**, such that each unit’s logic can be signed off independently (with known shared surfaces called out).

Assumptions for single-file unit reviews:

- `ops/mod.rs` helpers, `cache/*`, `interest`, `guards`, `events`, `storage`, `time`, and `common::*` are correct unless a finding shows otherwise.
- Hub/controller trust model: pool mutators are owner-only; token movement may be pre-funded by the hub.

## Units (file-level)

| ID | File | Feature surface | Peer deps | Status |
|----|------|-----------------|-----------|--------|
| `mod` | `ops/mod.rs` | `synced_market`, `renewed_market`, `load_leg`, batch runners + events | none | **clean** |
| `supply` | `ops/supply.rs` | mint supply, credit cash, backed-market gate | `load_leg` | **clean** |
| `borrow` | `ops/borrow.rs` | mint debt, debit cash, util max, transfer out | `load_leg` | **clean** |
| `repay` | `ops/repay.rs` | burn debt, credit net, overpayment refund | `load_leg` | **clean** |
| `withdraw` | `ops/withdraw.rs` | burn supply, liq fee, util skip on liq, transfer | `load_leg` | **clean** |
| `flash` | `ops/flash.rs` | payout, callback, balance checks, fee book | `renewed_market` | **clean** |
| `strategy` | `ops/strategy.rs` | debt mint + fee, net transfer | **`borrow::mint_debt`**, `renewed_market` | **clean** |
| `seize` | `ops/seize.rs` | bad-debt index write-down / revenue absorb | `synced_market` | **clean** |
| `net_settle` | `ops/net_settle.rs` | supply↔debt offset, no cash move | `synced_market` | **clean** |
| `recapitalize` | `ops/recapitalize.rs` | shortfall fill, refund excess | `renewed_market` | **clean** |
| `revenue` | `ops/revenue.rs` | claim treasury, pay owner | `renewed_market` | **clean** |
| `market` | `ops/market.rs` | create, rate model replace, force accrue | `renewed_market` | **clean** |

## Cross-cutting features

| ID | Feature | Why separate | Status |
|----|---------|--------------|--------|
| `feat-shared-mint-debt` | `borrow::mint_debt` used by borrow + strategy | dual entry surface | **clean** |
| `feat-cash-vs-token` | accounting cash vs SAC balance | INV-ACCT-02; flash balance checks | **clean** |
| `feat-accrual-timing` | synced vs renewed vs load-without-accrue | stale index / fee window | **clean** |
| `feat-guards-placement` | util / solvency / shortfall call sites | missed gate = drain | **clean** |
| `feat-batch-orchestration` | `run_batch*` + `lib.rs` wiring | partial batch, event completeness | **clean** |

## Invariants mapped

INV-AUTH-01, INV-ACCT-01..04/06/07, INV-IDX-02/03, INV-FLASH-01..03, max util on borrow/non-liq withdraw, strategy fee ≤ principal, liq fee → revenue.

## Deliverables

1. ~~Workflow run report~~ → `docs/audit/pool-ops/REPORT.md`
2. ~~Per-unit coverage table~~ → in REPORT.md
3. ~~Update this GOAL.md statuses~~ → all **clean**
4. Follow-ups (2026-08-08) — all complete:
   - Adversarial re-pass: `pool-ops-audit-2` → **0 findings** — `ADVERSARIAL-PASS.md`
   - Regression test: `tests::test_update_params_accrues_under_old_curve_after_time_advance` (**passing**)
   - Controller INV-ACCT-03: **0 findings** — `docs/audit/controller-acct03/REPORT.md`

## How to re-run

```text
/workflow pool-ops-audit
# subset:
# args: { "units": "flash,strategy,borrow" }
# agent_budget: 48
```

## Out of scope (this goal)

- Full controller liquidation graph (only pool-side seize/withdraw liq fee)
- Governance / oracle crates
- Fixing bugs (audit + report only)
