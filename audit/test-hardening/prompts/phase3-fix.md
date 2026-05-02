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
