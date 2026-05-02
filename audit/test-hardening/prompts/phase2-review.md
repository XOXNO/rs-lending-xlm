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
