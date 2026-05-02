# Test-Hardening Pipeline — Design

**Date:** 2026-05-02
**Status:** Approved (brainstorming phase). Implementation plan to follow.
**Scope:** Phase 1 — integration tests in `test-harness/tests/`. Phase 2 (inline tests in source crates) is a separate spec.

## Context

The repo has 737 passing tests (711 baseline + 26 added during the recent coverage push) at 96.80% production region coverage. Tests fall into three buckets:

- **Integration tests** — 49 files in `test-harness/tests/`, hundreds of `#[test]` functions exercising the controller + pool through the harness.
- **Inline source tests** — ~210 `#[test]` functions in `controller/`, `pool/`, `common/` source files (mostly math/utility unit tests).
- **Fuzz / proptest** — 8 files with property-based tests over random inputs.

Coverage tells us *what* runs; it does not tell us *whether the assertions verify intent*. A test can run, panic, and pass `#[should_panic]` without ever reaching the action it claims to test (the panic could come from setup). A success test can call `supply()` without ever reading storage to verify the supply landed correctly.

The protocol's correctness depends on tests that assert **post-state**, not just **non-panic**. This pipeline audits every integration test against a fixed rubric, has the audit independently reviewed, then applies the agreed patches under verification.

## Goals

1. Audit every integration test in `test-harness/tests/` against a quality rubric.
2. Have each audit independently peer-reviewed before any code is touched.
3. Apply only validated patches; verify each domain's tests stay green after fixes.
4. Produce a final summary: per-domain findings + applied patches + coverage delta.

Non-goals (separate work):
- Inline source tests (`#[cfg(test)] mod tests` in controller/pool/common). Get their own pipeline later.
- Brand-new test additions (this is a hardening pass, not a coverage-adding pass — the prior round handled that).
- Production code refactors. Test changes only.

## Quality Rubric (Pragmatic — 5 items)

For each `#[test]` function in scope:

1. **Specific error code.** `#[should_panic(expected = "Error(Contract, #N)")]` — never bare `#[should_panic]` and never a substring match unless the substring is the full error message.
2. **Panic origin = action under test.** The panic must come from the contract instruction the test claims to test, not from setup. (A test claiming to test "supply rejects isolated mix" but panicking during `set_market_config` is a false positive.)
3. **Post-state asserted (success path).** After the action returns, the test reads storage / view functions and asserts position state, account meta, isolated-debt counters, etc. match the expected post-state.
4. **Token-balance deltas asserted (success path).** When tokens move, the test asserts the caller's wallet balance changed by the expected amount.
5. **Test name describes scenario + outcome.** `test_<action>_<scenario>_<outcome>` — e.g., `test_supply_isolated_account_rejects_second_asset` is good; `test_supply_works` is bad.

**Free-rider** (not a separate finding, only piggy-backs on already-needed patches): when an agent is already producing a patch for items 1-5, it may also tighten obvious verbosity (copy-pasted setup blocks, redundant assertions). Does not trigger a patch on its own.

**Fuzz / proptest rubric** (replaces 1-5 for domain 8):
- F1. Each fuzz target asserts a clear invariant (not just "no panic").
- F2. Generators have tight bounds matched to the invariant.
- F3. Regression files exist (`*.proptest-regressions`) and are committed.
- F4. Failure cases reproduce deterministically when re-run.
- F5. The invariant under test is named + documented in a comment.

## Pipeline (3 phases, 8 parallel agents per phase)

```text
Phase 1: Domain Audit          Phase 2: Independent Review        Phase 3: Fix Team
8 agents (parallel)            8 agents (parallel, 1:1 mapping)   8 agents (parallel)
        |                              |                                 |
   audit reports     ───────►  validated reports (confirm/      ───────►  applied patches +
   + patches                   refute/refine each finding)               cargo test green
```

Each phase writes structured artifacts to `audit/test-hardening/phase{N}/<domain>.md`.

### Domain decomposition (~48 files, 8 domains)

| # | Domain | Files |
|---|---|---|
| 1 | Supply + EMode + Isolation | supply_tests, emode_tests, isolation_tests, account_tests, decimal_diversity_tests |
| 2 | Borrow + Repay | borrow_tests, repay_tests, oracle_tolerance_tests |
| 3 | Withdraw + Liquidation + Bad Debt | withdraw_tests, liquidation_tests, liquidation_coverage_tests, liquidation_math_tests, liquidation_mixed_decimal_tests, bad_debt_index_tests, lifecycle_regression_tests |
| 4 | Strategy | strategy_tests, strategy_bad_router_tests, strategy_coverage_tests, strategy_edge_tests, strategy_happy_tests, strategy_panic_coverage_tests |
| 5 | Admin + Keeper + Config | admin_config_tests, keeper_tests, keeper_admin_tests, validation_admin_tests |
| 6 | Views + Revenue + Interest + Math | views_tests, revenue_tests, interest_tests, interest_rigorous_tests, rewards_rigorous_tests, pool_revenue_edge_tests, pool_coverage_tests, math_rates_tests, utils_tests |
| 7 | Events + FlashLoan + Smoke + Footprint | events_tests, flash_loan_tests, footprint_test, smoke_test |
| 8 | Fuzz + Invariant + Chaos + Stress | fuzz_auth_matrix, fuzz_budget_metering, fuzz_conservation, fuzz_liquidation_differential, fuzz_multi_asset_solvency, fuzz_strategy_flashloan, fuzz_ttl_keepalive, invariant_tests, chaos_simulation_tests, stress_simulation_tests, bench_liquidate_max_positions |

### Phase 1 — Domain audit

**Each Phase 1 agent**:
- Reads every `#[test]` function in its domain's files.
- Applies the pragmatic rubric (or fuzz rubric for domain 8) to each test.
- Tags findings: `broken` / `weak` / `nit`.
  - `broken`: test asserts the wrong thing or panics in setup, so it passes vacuously.
  - `weak`: test does what it claims but misses key post-state assertions; would still catch a wrong-direction regression but would silently allow a no-op regression.
  - `nit`: naming, redundant code, missing event check.
- Produces a concrete suggested patch per finding (full before/after — `Edit`-tool ready).
- Distinguishes "the test reads storage via a harness helper (e.g., `t.supply_balance(USER, ASSET)`)" from "the test asserts nothing" — helpers can hide post-state assertions and must not be flagged as missing.
- Outputs: `audit/test-hardening/phase1/<domain>.md` — markdown with one section per file, one entry per `#[test]`.
- **Makes NO code changes.**

### Phase 2 — Independent peer review

**Each Phase 2 agent**:
- Reads `audit/test-hardening/phase1/<domain>.md`.
- Reads the same source files **with fresh eyes** — does not see Phase 1 agent's reasoning trace, only its conclusions.
- For each finding the Phase 1 agent reported, assigns a disposition:
  - **`confirmed`** — finding is real and the patch is correct.
  - **`refuted`** — finding is wrong (test is fine; auditor missed a helper-based assertion or misread the panic origin). Reviewer must justify the refutation in writing.
  - **`refined`** — finding is real but the patch is wrong/incomplete; reviewer rewrites the patch.
- Adds **`new`** findings — issues Phase 1 missed that the reviewer caught.
- Outputs: `audit/test-hardening/phase2/<domain>.md` — same structure as Phase 1 with `disposition` and final agreed patch on every entry.
- **Makes NO code changes.**

### Phase 3 — Fix team

**Each Phase 3 agent**:
- Reads `audit/test-hardening/phase2/<domain>.md`.
- Applies every patch with disposition `confirmed`, `refined`, or `new` (skips `refuted`).
- After applying patches in a file: runs `cargo test --test <test_file>` and confirms green.
- If any test fails post-fix: rolls back the failing patch, marks it `failed` in the output, continues with the rest of the patches in the file.
- Outputs: `audit/test-hardening/phase3/<domain>.md` — list of applied / failed / skipped patches, per-file test count delta.
- **Makes code changes** — but only ones the peer reviewer already validated.

### Final consolidation (parent agent)

After all 8 Phase 3 agents finish:
1. Aggregate the 8 Phase 3 reports into `audit/test-hardening/SUMMARY.md`: total patches applied, failed, skipped, per-domain breakdown.
2. Run full `cargo test` workspace-wide; assert no regression below the pre-pipeline 737-passing baseline.
3. Run `cargo llvm-cov` and report the coverage delta.
4. Surface to the user: aggregated summary + reference to the diff range (not per-test review — the peer-review gate already did that).

## Artifact Layout

```
audit/test-hardening/
├── phase1/
│   ├── 01-supply-emode-isolation.md
│   ├── 02-borrow-repay.md
│   ├── ...
│   └── 08-fuzz-invariant.md
├── phase2/
│   └── ... (same 8 domain files, with dispositions)
├── phase3/
│   └── ... (applied/failed/skipped per domain)
└── SUMMARY.md
```

Each `<domain>.md` follows a fixed schema documented in the implementation plan.

## Why This Works

- **Two independent eyes per finding before any code is touched.** Phase 2 reviewer doesn't see Phase 1's reasoning, only its claims, so it's a real audit not a rubber-stamp.
- **Bounded blast radius per Phase 3 agent.** Each only touches its domain's files. Failed patches roll back without contaminating the rest.
- **Parallelism.** All three phases run 8 agents at once. Total wall-clock ≈ 3 × (longest agent in phase) — order of 30-40 minutes for the whole pipeline.
- **Resumable.** Each phase writes its output to disk. A failed phase can be re-run without redoing earlier phases.
- **Self-verifying.** Phase 3 runs `cargo test` per file, so a fix that breaks a test never lands.

## Risks

1. **Fuzz rubric divergence.** Domain 8 uses a different rubric. The Phase 2 reviewer for domain 8 must be told explicitly. Mitigated: domain-specific prompts.
2. **Helper-based assertions.** Tests that read storage through `LendingTest` helpers (`t.supply_balance()`, `t.borrow_balance()`, `t.assert_no_positions()`) can look like they assert nothing if the auditor doesn't follow the helper. Mitigated: rubric item 3 explicitly notes "via helper or direct" and Phase 2 reviewer's job is to catch this kind of false positive.
3. **Phase 3 patch dependencies.** Two patches in the same file might depend on each other; if patch A is applied first and B depends on A's hunk, ordering matters. Mitigated: Phase 3 agent applies patches in declared order (top-to-bottom in the source file) and recomputes line numbers between patches.
4. **Test names changing.** Rubric item 5 may rename tests. Renames must be applied alongside any other patches to the same test in one atomic edit. Mitigated: Phase 2 reviewer combines name-change + content-change patches into a single agreed patch per test.

## Out of Scope

- Inline tests in `controller/`, `pool/`, `common/` source crates. Get their own pipeline later (smaller because ~17 source files contain the inline tests, mostly self-asserting math).
- New tests / coverage uplift. The previous coverage push handled that; this is hardening only.
- Production code changes. Tests only.
- Performance / runtime improvements to tests.

## Verification

End-to-end:
1. `cargo test` workspace-wide — must stay at 737 passing or higher (failed patches are rolled back, never reduce the pass count).
2. `cargo llvm-cov` — production region coverage must stay at or above 96.80%.
3. `cargo check --all-targets` — clean.
4. `cargo clippy --all-targets -- -D warnings` — no new warnings introduced by test changes.

## Next Step

Invoke the `writing-plans` skill to break this design into an executable implementation plan with concrete agent prompts, artifact schemas, and verification steps.
