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
