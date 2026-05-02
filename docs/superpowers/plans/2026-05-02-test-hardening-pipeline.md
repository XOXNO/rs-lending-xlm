# Test-Hardening Pipeline — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Audit every integration test in `test-harness/tests/` against a quality rubric, peer-review the audits, and apply validated patches under verification.

**Architecture:** Three-phase pipeline (audit → independent review → fix). Eight parallel agents per phase, each handling one domain (~6 files). Each phase writes structured artifacts to `audit/test-hardening/phase{N}/<domain>.md`. Phase 3 agents apply patches and run targeted tests; failed patches roll back without contaminating the run.

**Tech Stack:** Rust workspace (`controller`, `pool`, `common`, `test-harness`). Test-harness uses `LendingTest` builder API and `t.ctrl_client()` direct contract calls. Coverage tool: `cargo-llvm-cov`. Agent dispatch via Claude's `Agent` tool with `general-purpose` subagent type.

**Spec:** `docs/superpowers/specs/2026-05-02-test-hardening-pipeline-design.md`

---

## File Structure

This plan produces these new artifacts (no production code changes; test files in `test-harness/tests/` are the only mutated source):

```
audit/test-hardening/
├── README.md                        # Pipeline overview + how to resume
├── SCHEMA.md                        # Markdown schema each phase output follows
├── prompts/
│   ├── phase1-audit-pragmatic.md    # Phase 1 prompt for domains 1–7
│   ├── phase1-audit-fuzz.md         # Phase 1 prompt for domain 8
│   ├── phase2-review.md             # Phase 2 prompt (same for all 8 domains)
│   └── phase3-fix.md                # Phase 3 prompt (same for all 8 domains)
├── phase1/                          # Populated by Task 6
│   ├── 01-supply-emode-isolation.md
│   ├── 02-borrow-repay.md
│   ├── ... (8 files)
├── phase2/                          # Populated by Task 8
│   └── ... (same 8 file names)
├── phase3/                          # Populated by Task 10
│   └── ... (same 8 file names)
└── SUMMARY.md                       # Final aggregate, written in Task 11
```

`test-harness/tests/*.rs` files are mutated by Phase 3 agents only. Production source code (`controller/`, `pool/`, `common/`) is read-only throughout this plan.

---

## Task 1: Scaffold the artifact directory + README

**Files:**
- Create: `audit/test-hardening/README.md`
- Create: `audit/test-hardening/phase1/.gitkeep`
- Create: `audit/test-hardening/phase2/.gitkeep`
- Create: `audit/test-hardening/phase3/.gitkeep`
- Create: `audit/test-hardening/prompts/.gitkeep`

- [ ] **Step 1: Create the directory tree**

```bash
mkdir -p /Users/mihaieremia/GitHub/rs-lending-xlm/audit/test-hardening/{phase1,phase2,phase3,prompts}
touch /Users/mihaieremia/GitHub/rs-lending-xlm/audit/test-hardening/{phase1,phase2,phase3,prompts}/.gitkeep
```

Expected: directories exist, `.gitkeep` files present.

- [ ] **Step 2: Write the README**

Create `audit/test-hardening/README.md` with this exact content:

```markdown
# Test Hardening Pipeline — Artifacts

Three-phase audit/review/fix pipeline for integration tests in `test-harness/tests/`. See [the design spec](../../docs/superpowers/specs/2026-05-02-test-hardening-pipeline-design.md) for the full design rationale and [the implementation plan](../../docs/superpowers/plans/2026-05-02-test-hardening-pipeline.md) for execution steps.

## Layout

- `prompts/` — agent prompt templates used by each phase.
- `phase1/` — Phase 1 audit reports (one per domain).
- `phase2/` — Phase 2 peer-review reports with dispositions on every Phase 1 finding.
- `phase3/` — Phase 3 fix reports listing applied / failed / skipped patches.
- `SCHEMA.md` — the markdown schema each phase output follows.
- `SUMMARY.md` — aggregate written after Phase 3 completes.

## Resuming

Each phase writes to disk before the next phase starts. To resume after an interrupted run:
- Find the latest phase with a complete set of 8 domain reports.
- Re-run only the next phase.
- The fix-phase agents skip patches already applied (they look for the literal `before` text and no-op if it's gone).
```

- [ ] **Step 3: Commit**

```bash
cd /Users/mihaieremia/GitHub/rs-lending-xlm
git add audit/test-hardening/
git commit -m "chore: scaffold test-hardening pipeline artifacts dir"
```

---

## Task 2: Write the artifact schema

**Files:**
- Create: `audit/test-hardening/SCHEMA.md`

- [ ] **Step 1: Write the schema document**

Create `audit/test-hardening/SCHEMA.md` with this exact content:

````markdown
# Phase Output Schema

Every phase writes one markdown file per domain following this exact structure. The fixed structure makes the next phase's parsing trivial.

## Domain header (top of every file)

```markdown
# Domain N — <domain name>

**Phase:** 1 | 2 | 3
**Files in scope:**
- `test-harness/tests/<file_a>.rs`
- `test-harness/tests/<file_b>.rs`
- ...

**Totals:** broken=X weak=Y nit=Z (Phase 1)
            confirmed=A refuted=B refined=C new=D (Phase 2)
            applied=A failed=F skipped=S (Phase 3)
```

## Per-test entry (Phase 1)

````markdown
### `<test_file>.rs::<test_name>`

**Severity:** broken | weak | nit | none
**Rubric items failed:** [1, 3, 4]   (or "none" if severity = none)
**Why:** one-paragraph explanation citing line numbers.

**Patch (suggested):**
```diff
--- before
+++ after
@@ ... @@
-old code
+new code
```
````

If `Severity: none`, omit `Rubric items failed`, `Why`, and `Patch` blocks — just the heading.

## Per-test entry (Phase 2)

Same as Phase 1 plus:

```markdown
**Disposition:** confirmed | refuted | refined | new
**Reviewer note:** required when disposition = refuted or refined; one-paragraph justification or replacement-patch rationale.
```

If `disposition = refined`, the `Patch (suggested)` block is replaced with the reviewer's rewritten patch.
If `disposition = new`, the entire entry is added by the reviewer (Phase 1 didn't flag it).

## Per-test entry (Phase 3)

Same as Phase 2 plus:

```markdown
**Result:** applied | failed | skipped
**Test run:** `cargo test --test <file_stem>` -> X passed, Y failed
**Rollback reason:** required when result = failed; one-line description of why the patch broke a test.
```

`skipped` is used only for `disposition = refuted` from Phase 2.
````

- [ ] **Step 2: Commit**

```bash
git add audit/test-hardening/SCHEMA.md
git commit -m "docs: artifact schema for test-hardening phases"
```

---

## Task 3: Write the Phase 1 audit prompt (pragmatic rubric, domains 1–7)

**Files:**
- Create: `audit/test-hardening/prompts/phase1-audit-pragmatic.md`

- [ ] **Step 1: Write the prompt template**

Create `audit/test-hardening/prompts/phase1-audit-pragmatic.md`:

````markdown
# Phase 1 Audit Prompt — Pragmatic Rubric (Domains 1–7)

Substitute `{{DOMAIN_NUM}}`, `{{DOMAIN_NAME}}`, `{{DOMAIN_FILES}}`, `{{OUTPUT_PATH}}` before passing to `Agent`.

---

Repo: /Users/mihaieremia/GitHub/rs-lending-xlm

You are auditing integration tests for domain {{DOMAIN_NUM}} ({{DOMAIN_NAME}}) of a Soroban Rust lending protocol. Your job is to identify weak or broken tests and propose fixes — but **make NO code changes**. This is audit-only.

## Files in scope

{{DOMAIN_FILES}}

## Pragmatic Rubric (5 items)

For each `#[test]` function in the files above, check:

1. **Specific error code.** `#[should_panic(expected = "Error(Contract, #N)")]` with the exact contract error code. Bare `#[should_panic]` is **broken**. A substring like `expected = "Error"` is **weak** unless that's the full panic message.
2. **Panic origin = action under test.** The panic must come from the contract instruction the test claims to test, not from setup. If a test named `test_supply_rejects_isolated_mix` actually panics inside `set_market_config` because the test fixture is misconfigured, that's **broken** — the test passes vacuously without exercising the supply path.
3. **Post-state asserted (success path).** After the action returns, the test reads storage / view functions and asserts position state, account meta, isolated-debt counters, etc. Reads via harness helpers count — `t.supply_balance(USER, ASSET)`, `t.borrow_balance(USER, ASSET)`, `t.assert_no_positions(USER)`, `t.health_factor(USER)` are all valid post-state assertions. **A test that calls `supply()` and only checks the function didn't panic is weak**.
4. **Token-balance deltas (success path).** When tokens move (supply / repay / withdraw / borrow), the test asserts the caller's wallet balance changed by the expected amount. Helpers like `t.token_balance(USER, ASSET)` count.
5. **Test name describes scenario + outcome.** `test_<action>_<scenario>_<outcome>` — `test_supply_isolated_account_rejects_second_asset` is good; `test_supply_works` is bad (nit).

**Severity tagging:**
- `broken`: rubric items 1 or 2 fail (test passes vacuously or asserts the wrong thing).
- `weak`: items 3 or 4 fail (test does what it claims but misses key post-state).
- `nit`: only item 5 fails.
- `none`: all items pass.

**Free-rider simplification (does NOT trigger its own patch):** when you're already producing a patch for items 1–5, you may also tighten obvious verbosity (copy-pasted setup that should be a helper, redundant assertions). Do not propose a patch *only* to simplify a passing test.

## False-positive guidance

- **Helper-based assertions count.** Read `test-harness/src/{user,view,assert,context}.rs` to learn what `t.supply_balance`, `t.assert_no_positions`, `t.total_collateral`, `t.health_factor`, etc. do. Don't flag a test as missing post-state when it asserts via a helper.
- **Multi-step setup is fine if the assertion is real.** A test that supplies + borrows then asserts both is a single behavior (open-position) — not a multi-test mash-up.
- **`#[should_panic]` without `expected = ...`** is always broken, even if the function only has one possible panic — the protocol may add another panic later and the test would falsely pass.

## Output

Write your full report to `{{OUTPUT_PATH}}` following the schema in `audit/test-hardening/SCHEMA.md`. Required structure:

1. Domain header with file list and totals (broken=X weak=Y nit=Z).
2. One section per `#[test]` function in scope. Tests with `severity: none` get only the heading; everything else gets the full entry (severity, rubric items failed, why with line numbers, suggested patch as a unified diff).
3. End with a one-paragraph summary of cross-cutting patterns (e.g., "all 30 supply tests assert `supply_balance` but none verify `account_meta.is_isolated` after isolated supply").

## Constraints

- Make NO code changes. Audit only.
- Cite line numbers from the actual source files.
- Each suggested patch must be a complete, applicable unified diff (not pseudo-code).
- For helper-based reads that you propose adding: use the existing harness helper if one exists; only suggest a new helper if you confirm none exists.
- Do not propose changes to test names that would conflict with `cargo test --test <file>` invocations elsewhere in the repo (`grep -r "test_name" .` first if you're going to rename).

## Verification

After writing your report, confirm:
- All `#[test]` functions in the listed files appear in the report.
- The totals match the sum of severities you tagged.
- Every patch with `severity = broken | weak` has a complete diff.
````

- [ ] **Step 2: Commit**

```bash
git add audit/test-hardening/prompts/phase1-audit-pragmatic.md
git commit -m "docs: phase 1 audit prompt (pragmatic rubric)"
```

---

## Task 4: Write the Phase 1 audit prompt (fuzz rubric, domain 8)

**Files:**
- Create: `audit/test-hardening/prompts/phase1-audit-fuzz.md`

- [ ] **Step 1: Write the fuzz prompt template**

Create `audit/test-hardening/prompts/phase1-audit-fuzz.md`:

````markdown
# Phase 1 Audit Prompt — Fuzz Rubric (Domain 8)

Substitute `{{DOMAIN_FILES}}`, `{{OUTPUT_PATH}}` before passing to `Agent`.

---

Repo: /Users/mihaieremia/GitHub/rs-lending-xlm

You are auditing fuzz / proptest / invariant / chaos / stress tests for a Soroban Rust lending protocol. Your job is to identify weak fuzz tests — **make NO code changes**.

## Files in scope

{{DOMAIN_FILES}}

## Fuzz Rubric (5 items)

For each `#[test]` (proptest macro `proptest! { ... }` or chaos / invariant / stress test) function in the files above:

1. **F1. Clear invariant.** The test asserts a named invariant — not just "no panic". Examples of clear invariants: HF >= 1 after all healthy operations; supplied_ray >= sum(scaled supply positions); total_debt_usd never decreases without a corresponding repay event. A `proptest!` block that just runs operations and exits successfully is **broken** — it tests nothing the type system doesn't already prove.
2. **F2. Generators have tight bounds.** `prop::collection::vec(..., 0..=N)` with N matching the invariant's domain. `0u32..1_000_000_000` for amounts when the protocol caps amounts at i128::MAX is over-generous and dilutes coverage.
3. **F3. Regression files committed.** Each `proptest!` test has a sibling `.proptest-regressions` file checked into git (look for it next to the source file). Missing regression file = `weak`.
4. **F4. Failure cases reproduce.** Pick the most recent regression (if any) and confirm the proptest config (`cases = N`, `seed`) makes failures replayable. Tests that disable shrinking are **weak**.
5. **F5. Invariant documented.** A doc comment `///` above the proptest block explains the invariant in English, citing the protocol property it protects.

**Severity tagging:**
- `broken`: F1 fails (no invariant, just "ran without panic").
- `weak`: F2 / F3 / F4 fails.
- `nit`: F5 fails.
- `none`: all items pass.

## Output

Same schema as the pragmatic audit. Write your report to `{{OUTPUT_PATH}}`.

## Constraints

- Make NO code changes.
- Cite the proptest config block + invariant assertion line numbers.
- For F2 patches: propose specific bound numbers grounded in the protocol's actual limits (read `common/src/constants.rs` and `controller/src/access.rs` for the live values).
- For F3 missing-regression: propose `git add` of any local regression files; if none exist, note that the proptest hasn't found a counterexample yet (which is fine — F3 is about *committing* existing regression files, not generating new ones).
````

- [ ] **Step 2: Commit**

```bash
git add audit/test-hardening/prompts/phase1-audit-fuzz.md
git commit -m "docs: phase 1 audit prompt (fuzz rubric)"
```

---

## Task 5: Write the Phase 2 review prompt

**Files:**
- Create: `audit/test-hardening/prompts/phase2-review.md`

- [ ] **Step 1: Write the prompt template**

Create `audit/test-hardening/prompts/phase2-review.md`:

````markdown
# Phase 2 Review Prompt

Substitute `{{DOMAIN_NUM}}`, `{{DOMAIN_NAME}}`, `{{PHASE1_PATH}}`, `{{OUTPUT_PATH}}`, `{{RUBRIC}}` before passing to `Agent`.

`{{RUBRIC}}` is either `pragmatic` (domains 1–7) or `fuzz` (domain 8). The reviewer reads the rubric definition from the corresponding file in `audit/test-hardening/prompts/phase1-audit-{{RUBRIC}}.md`.

---

Repo: /Users/mihaieremia/GitHub/rs-lending-xlm

You are an **independent reviewer** of an audit produced by another agent. Your job is to validate every finding the auditor reported and surface any findings the auditor missed. Your output is the final agreed list of patches — Phase 3 will apply only what you confirm.

## Inputs

- The auditor's report: `{{PHASE1_PATH}}`
- The same source files the auditor reviewed (test files in `test-harness/tests/` for domain {{DOMAIN_NUM}} — {{DOMAIN_NAME}}).
- The rubric: `audit/test-hardening/prompts/phase1-audit-{{RUBRIC}}.md`.

## Process — fresh eyes

**Critical:** read the source files BEFORE you read the auditor's report. Form your own view of each test against the rubric. Then compare against the auditor's findings. This catches both false positives (auditor flagged something that's actually fine) and false negatives (auditor missed real issues).

For each finding the auditor reported, assign a disposition:
- **`confirmed`** — finding is real and the suggested patch is correct.
- **`refuted`** — finding is wrong. Common causes: auditor missed a helper-based assertion (`t.assert_no_positions` etc.), misread the panic origin, or flagged a fuzz test that *is* asserting an invariant via `prop_assert!`. Justify the refutation in the `Reviewer note`.
- **`refined`** — finding is real but the patch is wrong, incomplete, or breaks something. Rewrite the patch in the `Patch (suggested)` block and explain in `Reviewer note` what was wrong with the original.

For findings the auditor missed, add a new entry with `Disposition: new`.

## Output

Write to `{{OUTPUT_PATH}}` following the schema in `audit/test-hardening/SCHEMA.md`. Required:

1. Domain header with totals: `confirmed=A refuted=B refined=C new=D`.
2. One section per test entry from Phase 1, **plus any `new` entries you add**, in the order they appear in source files.
3. Every entry has a `Disposition` line. `refuted` and `refined` entries also have a `Reviewer note`.

## Constraints

- Make NO code changes.
- Read the source files first. Do not read the auditor's report until you've formed your own view.
- A `refuted` disposition without a written justification is invalid — Phase 3 will skip refuted entries, so the user's only signal that a fix isn't being applied is your justification.
- For `refined` entries: the rewritten patch must be a complete, applicable unified diff. Do not leave `// TODO` placeholders.

## Common false positives the auditor may have produced

- **Helper-based reads.** `t.supply_balance(USER, "USDC")` returns the user's actual on-chain supply position via `borrow_amount_for_token` view. Tests that call helpers like this DO assert post-state.
- **Helper-based events.** `t.assert_emit(...)` style helpers in `test-harness/src/assert.rs` count as event verification.
- **Setup panics that ARE the test.** A few tests in admin paths panic during a configuration call — that IS the action under test (the configuration call itself), so the panic origin is correct.
- **`proptest! { #![proptest_config(...)] ... }`** with `prop_assert!` inside DOES assert an invariant. Don't refute as "no invariant" without reading the assertion macros.

## Verification

After writing your report:
- Every Phase 1 entry has a disposition.
- Totals match the disposition counts.
- Every `refuted` entry has a `Reviewer note`.
- Every `refined` and `new` entry has a complete patch.
````

- [ ] **Step 2: Commit**

```bash
git add audit/test-hardening/prompts/phase2-review.md
git commit -m "docs: phase 2 peer-review prompt"
```

---

## Task 6: Write the Phase 3 fix prompt

**Files:**
- Create: `audit/test-hardening/prompts/phase3-fix.md`

- [ ] **Step 1: Write the prompt template**

Create `audit/test-hardening/prompts/phase3-fix.md`:

````markdown
# Phase 3 Fix Prompt

Substitute `{{DOMAIN_NUM}}`, `{{DOMAIN_NAME}}`, `{{PHASE2_PATH}}`, `{{OUTPUT_PATH}}`, `{{TEST_FILES}}` before passing to `Agent`.

`{{TEST_FILES}}` is a space-separated list of file stems (without `.rs`) for `cargo test --test <stem>` — e.g., `supply_tests emode_tests isolation_tests account_tests decimal_diversity_tests`.

---

Repo: /Users/mihaieremia/GitHub/rs-lending-xlm

You are applying validated test patches for domain {{DOMAIN_NUM}} ({{DOMAIN_NAME}}). The peer reviewer has confirmed which patches to apply. Your job is to apply them, run the targeted tests, and roll back any patch that breaks a test.

## Inputs

- The validated report: `{{PHASE2_PATH}}`
- Test files in scope: {{TEST_FILES}} (in `test-harness/tests/`)

## Process

1. Read `{{PHASE2_PATH}}` and collect every entry with `Disposition: confirmed | refined | new`. Skip entries with `Disposition: refuted`.
2. Group entries by source file (the `<test_file>.rs` from each entry's heading).
3. For each file, in source-file line order (top-to-bottom):
   a. Apply the patch using the `Edit` tool. Each patch's `Patch (suggested)` block is a unified diff — use the `before` text as `old_string` and the `after` text as `new_string`.
   b. After applying ALL patches in the file, run `cargo test --test <file_stem>` from the repo root.
   c. If the test run is green: continue to the next file.
   d. If any test in the file fails: identify which patch caused it (the failing test name should match a patch you applied). Roll that patch back using a reverse `Edit` (swap `old_string` / `new_string`). Re-run `cargo test --test <file_stem>` and confirm green. Mark that one patch `Result: failed` with a `Rollback reason`. Continue with the rest of the file.
4. After all files in scope are processed, run `cargo test --test <file_stem>` once more for each file to confirm final state.

## Output

Write to `{{OUTPUT_PATH}}` following the schema in `audit/test-hardening/SCHEMA.md`. Required:

1. Domain header with totals: `applied=A failed=F skipped=S`.
2. One section per Phase 2 entry, in the same order, with `Result: applied | failed | skipped` and (for failures) a `Rollback reason`.
3. End with a `## Test runs` section listing the final `cargo test --test <stem>` output for each file in scope.

## Constraints

- Apply patches ONLY for `confirmed | refined | new` dispositions.
- Skip `refuted` entries (mark `Result: skipped`).
- Roll back any patch that breaks a test. Never leave the suite red.
- Do not modify production code (`controller/`, `pool/`, `common/`). Only `test-harness/tests/<file>.rs` is in scope.
- Do not modify the harness library (`test-harness/src/`); if a fix needs a new helper, mark the patch `failed` with reason "needs harness helper, out of scope" and continue.
- One commit per source file at the end of step 3 for that file (so each commit is atomically reviewable per-file).

## Verification

After writing your report:
- Every Phase 2 entry from your domain has a `Result` line.
- For every file in scope, the latest `cargo test --test <stem>` output shows `0 failed`.
- Totals match.
````

- [ ] **Step 2: Commit**

```bash
git add audit/test-hardening/prompts/phase3-fix.md
git commit -m "docs: phase 3 fix prompt"
```

---

## Task 7: Dispatch Phase 1 — 8 parallel audit agents

**Files:**
- Create: `audit/test-hardening/phase1/01-supply-emode-isolation.md`
- Create: `audit/test-hardening/phase1/02-borrow-repay.md`
- Create: `audit/test-hardening/phase1/03-withdraw-liquidation-bad-debt.md`
- Create: `audit/test-hardening/phase1/04-strategy.md`
- Create: `audit/test-hardening/phase1/05-admin-keeper-config.md`
- Create: `audit/test-hardening/phase1/06-views-revenue-interest-math.md`
- Create: `audit/test-hardening/phase1/07-events-flashloan-smoke-footprint.md`
- Create: `audit/test-hardening/phase1/08-fuzz-invariant-chaos-stress.md`

- [ ] **Step 1: Construct the 8 agent prompts**

Take the prompt template from `audit/test-hardening/prompts/phase1-audit-pragmatic.md` (or `phase1-audit-fuzz.md` for domain 8). Substitute the placeholders for each domain. Each domain's substitutions:

```text
Domain 1 — Supply + EMode + Isolation
{{DOMAIN_NUM}}: 1
{{DOMAIN_NAME}}: Supply + EMode + Isolation
{{DOMAIN_FILES}}:
- test-harness/tests/supply_tests.rs
- test-harness/tests/emode_tests.rs
- test-harness/tests/isolation_tests.rs
- test-harness/tests/account_tests.rs
- test-harness/tests/decimal_diversity_tests.rs
{{OUTPUT_PATH}}: audit/test-hardening/phase1/01-supply-emode-isolation.md

Domain 2 — Borrow + Repay
{{DOMAIN_NUM}}: 2
{{DOMAIN_NAME}}: Borrow + Repay
{{DOMAIN_FILES}}:
- test-harness/tests/borrow_tests.rs
- test-harness/tests/repay_tests.rs
- test-harness/tests/oracle_tolerance_tests.rs
{{OUTPUT_PATH}}: audit/test-hardening/phase1/02-borrow-repay.md

Domain 3 — Withdraw + Liquidation + Bad Debt
{{DOMAIN_NUM}}: 3
{{DOMAIN_NAME}}: Withdraw + Liquidation + Bad Debt
{{DOMAIN_FILES}}:
- test-harness/tests/withdraw_tests.rs
- test-harness/tests/liquidation_tests.rs
- test-harness/tests/liquidation_coverage_tests.rs
- test-harness/tests/liquidation_math_tests.rs
- test-harness/tests/liquidation_mixed_decimal_tests.rs
- test-harness/tests/bad_debt_index_tests.rs
- test-harness/tests/lifecycle_regression_tests.rs
{{OUTPUT_PATH}}: audit/test-hardening/phase1/03-withdraw-liquidation-bad-debt.md

Domain 4 — Strategy
{{DOMAIN_NUM}}: 4
{{DOMAIN_NAME}}: Strategy
{{DOMAIN_FILES}}:
- test-harness/tests/strategy_tests.rs
- test-harness/tests/strategy_bad_router_tests.rs
- test-harness/tests/strategy_coverage_tests.rs
- test-harness/tests/strategy_edge_tests.rs
- test-harness/tests/strategy_happy_tests.rs
- test-harness/tests/strategy_panic_coverage_tests.rs
{{OUTPUT_PATH}}: audit/test-hardening/phase1/04-strategy.md

Domain 5 — Admin + Keeper + Config
{{DOMAIN_NUM}}: 5
{{DOMAIN_NAME}}: Admin + Keeper + Config
{{DOMAIN_FILES}}:
- test-harness/tests/admin_config_tests.rs
- test-harness/tests/keeper_tests.rs
- test-harness/tests/keeper_admin_tests.rs
- test-harness/tests/validation_admin_tests.rs
{{OUTPUT_PATH}}: audit/test-hardening/phase1/05-admin-keeper-config.md

Domain 6 — Views + Revenue + Interest + Math
{{DOMAIN_NUM}}: 6
{{DOMAIN_NAME}}: Views + Revenue + Interest + Math
{{DOMAIN_FILES}}:
- test-harness/tests/views_tests.rs
- test-harness/tests/revenue_tests.rs
- test-harness/tests/interest_tests.rs
- test-harness/tests/interest_rigorous_tests.rs
- test-harness/tests/rewards_rigorous_tests.rs
- test-harness/tests/pool_revenue_edge_tests.rs
- test-harness/tests/pool_coverage_tests.rs
- test-harness/tests/math_rates_tests.rs
- test-harness/tests/utils_tests.rs
{{OUTPUT_PATH}}: audit/test-hardening/phase1/06-views-revenue-interest-math.md

Domain 7 — Events + FlashLoan + Smoke + Footprint
{{DOMAIN_NUM}}: 7
{{DOMAIN_NAME}}: Events + FlashLoan + Smoke + Footprint
{{DOMAIN_FILES}}:
- test-harness/tests/events_tests.rs
- test-harness/tests/flash_loan_tests.rs
- test-harness/tests/footprint_test.rs
- test-harness/tests/smoke_test.rs
{{OUTPUT_PATH}}: audit/test-hardening/phase1/07-events-flashloan-smoke-footprint.md

Domain 8 — Fuzz + Invariant + Chaos + Stress (FUZZ RUBRIC)
{{DOMAIN_FILES}}:
- test-harness/tests/fuzz_auth_matrix.rs
- test-harness/tests/fuzz_budget_metering.rs
- test-harness/tests/fuzz_conservation.rs
- test-harness/tests/fuzz_liquidation_differential.rs
- test-harness/tests/fuzz_multi_asset_solvency.rs
- test-harness/tests/fuzz_strategy_flashloan.rs
- test-harness/tests/fuzz_ttl_keepalive.rs
- test-harness/tests/invariant_tests.rs
- test-harness/tests/chaos_simulation_tests.rs
- test-harness/tests/stress_simulation_tests.rs
- test-harness/tests/bench_liquidate_max_positions.rs
{{OUTPUT_PATH}}: audit/test-hardening/phase1/08-fuzz-invariant-chaos-stress.md
```

- [ ] **Step 2: Dispatch all 8 agents in parallel (one message, 8 Agent tool calls)**

```text
Agent({
  description: "Phase 1 audit — Supply/EMode/Isolation",
  subagent_type: "general-purpose",
  prompt: <template from phase1-audit-pragmatic.md with domain 1 substitutions>,
  run_in_background: true
})
Agent({
  description: "Phase 1 audit — Borrow/Repay",
  subagent_type: "general-purpose",
  prompt: <template with domain 2 substitutions>,
  run_in_background: true
})
... (6 more)
Agent({
  description: "Phase 1 audit — Fuzz/Invariant/Chaos/Stress",
  subagent_type: "general-purpose",
  prompt: <template from phase1-audit-fuzz.md with domain 8 substitutions>,
  run_in_background: true
})
```

Expected: 8 agents start in background. Each writes its output file. Wait for all 8 completion notifications.

- [ ] **Step 3: Verify all 8 phase1 outputs exist and follow the schema**

```bash
ls -la /Users/mihaieremia/GitHub/rs-lending-xlm/audit/test-hardening/phase1/
```

Expected: 8 `.md` files (excluding `.gitkeep`).

```bash
for f in /Users/mihaieremia/GitHub/rs-lending-xlm/audit/test-hardening/phase1/*.md; do
  echo "=== $f ==="
  head -20 "$f"
  echo ""
done
```

Expected: each file starts with `# Domain N — <name>` and has a `**Totals:** broken=X weak=Y nit=Z` line.

- [ ] **Step 4: Aggregate Phase 1 totals into a single line**

```bash
grep "^\*\*Totals:" /Users/mihaieremia/GitHub/rs-lending-xlm/audit/test-hardening/phase1/*.md
```

Expected: 8 lines, one per domain, with totals to eyeball before continuing.

- [ ] **Step 5: Commit Phase 1 outputs**

```bash
cd /Users/mihaieremia/GitHub/rs-lending-xlm
git add audit/test-hardening/phase1/
git commit -m "audit(phase1): test-harness audit reports across 8 domains"
```

---

## Task 8: Dispatch Phase 2 — 8 parallel review agents

**Files:**
- Create: `audit/test-hardening/phase2/01-supply-emode-isolation.md`
- Create: `audit/test-hardening/phase2/02-borrow-repay.md`
- Create: `audit/test-hardening/phase2/03-withdraw-liquidation-bad-debt.md`
- Create: `audit/test-hardening/phase2/04-strategy.md`
- Create: `audit/test-hardening/phase2/05-admin-keeper-config.md`
- Create: `audit/test-hardening/phase2/06-views-revenue-interest-math.md`
- Create: `audit/test-hardening/phase2/07-events-flashloan-smoke-footprint.md`
- Create: `audit/test-hardening/phase2/08-fuzz-invariant-chaos-stress.md`

- [ ] **Step 1: Construct the 8 review agent prompts**

Use the template from `audit/test-hardening/prompts/phase2-review.md` for all 8. Substitutions for each domain follow the same pattern as Task 7. The `{{RUBRIC}}` placeholder is `pragmatic` for domains 1–7 and `fuzz` for domain 8.

Domain N substitutions:
- `{{DOMAIN_NUM}}`: same as Task 7
- `{{DOMAIN_NAME}}`: same as Task 7
- `{{PHASE1_PATH}}`: `audit/test-hardening/phase1/0N-<slug>.md` (the file Phase 1 created for this domain)
- `{{OUTPUT_PATH}}`: `audit/test-hardening/phase2/0N-<slug>.md`
- `{{RUBRIC}}`: `pragmatic` (domains 1–7) or `fuzz` (domain 8)

- [ ] **Step 2: Dispatch all 8 review agents in parallel (one message, 8 Agent tool calls)**

```text
Agent({
  description: "Phase 2 review — <domain N>",
  subagent_type: "general-purpose",
  prompt: <phase2-review.md template with domain N substitutions>,
  run_in_background: true
})
... (× 8)
```

Wait for all 8 completion notifications.

- [ ] **Step 3: Verify Phase 2 schema + dispositions**

```bash
ls -la /Users/mihaieremia/GitHub/rs-lending-xlm/audit/test-hardening/phase2/
grep "^\*\*Totals:" /Users/mihaieremia/GitHub/rs-lending-xlm/audit/test-hardening/phase2/*.md
```

Expected: 8 `.md` files, each with totals line `confirmed=A refuted=B refined=C new=D`.

```bash
grep -c "^\*\*Disposition:\*\* refuted" /Users/mihaieremia/GitHub/rs-lending-xlm/audit/test-hardening/phase2/*.md
grep -B 2 "^\*\*Disposition:\*\* refuted" /Users/mihaieremia/GitHub/rs-lending-xlm/audit/test-hardening/phase2/*.md | head -50
```

Expected: every `refuted` entry has a `Reviewer note` immediately below. Spot-check a few — if any `refuted` entry has no `Reviewer note`, the schema is violated and Phase 3 must wait.

- [ ] **Step 4: Commit Phase 2 outputs**

```bash
git add audit/test-hardening/phase2/
git commit -m "audit(phase2): peer-reviewed test audit with dispositions"
```

---

## Task 9: Surface Phase 2 totals to the user before Phase 3

**Files:** none (read-only step).

- [ ] **Step 1: Compute aggregate totals across all 8 domains**

```bash
echo "=== Phase 2 aggregate ==="
awk '/^\*\*Totals:\*\*/ {
  for (i=2; i<=NF; i++) {
    split($i, kv, "=")
    sum[kv[1]] += kv[2]
  }
}
END {
  for (k in sum) printf "%s=%d ", k, sum[k]
  print ""
}' /Users/mihaieremia/GitHub/rs-lending-xlm/audit/test-hardening/phase2/*.md
```

Expected: one line like `confirmed=N refuted=M refined=K new=L`.

- [ ] **Step 2: Surface the aggregate to the user with a one-line summary**

The summary should be ≤ 5 lines and tell the user:
1. Total findings to apply (`confirmed + refined + new`).
2. Total findings rejected by review (`refuted`).
3. The path to inspect any specific domain report.
4. A "proceed to Phase 3?" checkpoint.

This is a hard checkpoint. Phase 3 mutates test files; do not proceed until the user confirms.

---

## Task 10: Dispatch Phase 3 — 8 parallel fix agents (CODE-CHANGING)

**Files:**
- Create: `audit/test-hardening/phase3/01-supply-emode-isolation.md` (and 7 more)
- Modify: any number of `test-harness/tests/*.rs` files in scope

- [ ] **Step 1: Capture pre-fix test counts**

```bash
cd /Users/mihaieremia/GitHub/rs-lending-xlm
cargo test 2>&1 | awk '/^test result/ { p+=$4; f+=$6 } END { print "PRE: passed=" p " failed=" f }'
```

Expected: `passed=737 failed=0` (the current baseline).

- [ ] **Step 2: Construct the 8 fix prompts**

Use the template from `audit/test-hardening/prompts/phase3-fix.md`. Substitutions:
- `{{DOMAIN_NUM}}`: same as Task 7
- `{{DOMAIN_NAME}}`: same as Task 7
- `{{PHASE2_PATH}}`: `audit/test-hardening/phase2/0N-<slug>.md`
- `{{OUTPUT_PATH}}`: `audit/test-hardening/phase3/0N-<slug>.md`
- `{{TEST_FILES}}`: space-separated `<file_stem>` list — e.g., for domain 1: `supply_tests emode_tests isolation_tests account_tests decimal_diversity_tests`

- [ ] **Step 3: Dispatch all 8 fix agents in parallel**

```text
Agent({
  description: "Phase 3 fix — <domain N>",
  subagent_type: "general-purpose",
  prompt: <phase3-fix.md template with domain N substitutions>,
  run_in_background: true
})
... (× 8)
```

Wait for all 8 completion notifications.

- [ ] **Step 4: Capture post-fix test counts**

```bash
cargo test 2>&1 | awk '/^test result/ { p+=$4; f+=$6 } END { print "POST: passed=" p " failed=" f }'
```

Expected: `failed=0`. The `passed=` count must be **>= 737** (the pre-fix baseline). Patches that broke tests should have been rolled back by the fix agents.

If `failed > 0`: do not proceed. The fix agents failed to roll back something. Identify the failing test, manually roll back the offending change, re-run, and document the incident in the SUMMARY.

- [ ] **Step 5: Verify Phase 3 reports**

```bash
ls -la /Users/mihaieremia/GitHub/rs-lending-xlm/audit/test-hardening/phase3/
grep "^\*\*Totals:" /Users/mihaieremia/GitHub/rs-lending-xlm/audit/test-hardening/phase3/*.md
```

Expected: 8 reports with totals like `applied=A failed=F skipped=S`.

- [ ] **Step 6: Commit Phase 3 reports + applied test changes**

```bash
git add audit/test-hardening/phase3/ test-harness/tests/
git commit -m "audit(phase3): apply validated test patches across 8 domains"
```

---

## Task 11: Final aggregation + verification

**Files:**
- Create: `audit/test-hardening/SUMMARY.md`

- [ ] **Step 1: Run full verification**

```bash
cd /Users/mihaieremia/GitHub/rs-lending-xlm
cargo check --all-targets 2>&1 | tail -5
cargo test 2>&1 | awk '/^test result/ { p+=$4; f+=$6 } END { print "passed=" p " failed=" f }'
cargo clippy --all-targets -- -D warnings 2>&1 | tail -10
```

Expected:
- `cargo check`: clean.
- `cargo test`: `failed=0`, `passed >= 737`.
- `cargo clippy`: zero new warnings introduced (any pre-existing ones are out of scope).

- [ ] **Step 2: Re-run coverage**

```bash
cargo llvm-cov --workspace --no-report --quiet 2>&1 | tail -3
cargo llvm-cov report --ignore-filename-regex="vendor/|test-harness/|tests\.rs|/tests/" --summary-only 2>&1 | tail -3
```

Expected: production region coverage stays at or above the 96.80% baseline.

- [ ] **Step 3: Write the SUMMARY**

Create `audit/test-hardening/SUMMARY.md` with this structure:

```markdown
# Test Hardening — Final Summary

**Date completed:** 2026-MM-DD

## Aggregate counts

- Phase 1: broken=X weak=Y nit=Z (across 8 domains)
- Phase 2: confirmed=A refuted=B refined=C new=D
- Phase 3: applied=P failed=F skipped=S

## Test suite

- Pre-fix: 737 passed, 0 failed
- Post-fix: <new count> passed, 0 failed

## Coverage

- Pre-fix production region coverage: 96.80%
- Post-fix production region coverage: <new>%

## Per-domain breakdown

| # | Domain | Phase 1 (b/w/n) | Phase 2 (c/r/r/n) | Phase 3 (a/f/s) |
|---|--------|------|------|------|
| 1 | Supply + EMode + Isolation | ... | ... | ... |
| 2 | Borrow + Repay | ... | ... | ... |
| 3 | Withdraw + Liquidation + Bad Debt | ... | ... | ... |
| 4 | Strategy | ... | ... | ... |
| 5 | Admin + Keeper + Config | ... | ... | ... |
| 6 | Views + Revenue + Interest + Math | ... | ... | ... |
| 7 | Events + FlashLoan + Smoke + Footprint | ... | ... | ... |
| 8 | Fuzz + Invariant + Chaos + Stress | ... | ... | ... |

## Notable findings

A short list (≤ 10 bullets) of cross-cutting patterns surfaced by the audit — e.g., "the entire borrow_tests file relied on `t.health_factor` checks but never asserted `t.borrow_balance` after operations" or "fuzz_strategy_flashloan was missing `prop_assert!` invariants".

## Failed patches (if any)

If Phase 3 reports any `failed` patches, list them here with the rollback reason and a follow-up issue link or TODO.
```

Fill in all values from the actual phase outputs and verification numbers.

- [ ] **Step 4: Commit final summary**

```bash
git add audit/test-hardening/SUMMARY.md
git commit -m "audit: test-hardening pipeline final summary"
```

- [ ] **Step 5: Surface to user**

Report to the user: number of patches applied, test count delta, coverage delta, and the path to `SUMMARY.md`. Done.

---

## Self-review (executed by plan author before handoff)

- **Spec coverage:** every spec section mapped — pipeline overview (Tasks 7–11), domain decomposition (Task 7 list), pragmatic rubric (Task 3), fuzz rubric (Task 4), Phase 2 disposition enum (Task 5), Phase 3 rollback semantics (Task 6), artifact layout (Task 1), final consolidation (Task 11). No spec section orphaned.
- **Placeholder scan:** no "TBD"/"TODO" left. Every prompt template has the literal text the agent will execute. The only `{{...}}` substitution markers are explicitly listed at the top of each prompt template along with their values.
- **Type consistency:** disposition enum (`confirmed`/`refuted`/`refined`/`new`/`failed`/`skipped`) used identically across SCHEMA.md, all 4 prompt templates, and Tasks 8–11. Severity tags (`broken`/`weak`/`nit`/`none`) consistent. File-naming convention (`0N-<slug>.md`) consistent across phases.
- **No duplicate work:** prompts live in one place, Tasks 7–10 reference them rather than embedding their full text.
