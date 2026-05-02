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
