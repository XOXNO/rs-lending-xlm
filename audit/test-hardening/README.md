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
