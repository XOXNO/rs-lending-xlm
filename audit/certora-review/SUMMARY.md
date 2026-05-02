# Certora Review — Consolidated Summary

**Date:** 2026-05-02
**Domains reviewed:** 7 (Solvency/Health/Position, Liquidation, Interest/Index/Math, Oracle, EMode/Isolation, Strategy/FlashLoan, Boundaries/Summaries/Compat)
**Files reviewed:** 13 rule files (~5,400 lines) + 218-line summaries + 84-line compat
**Per-domain reports:** `audit/certora-review/0[1-7]-*.md`

## Headline verdict

**The Certora framework as currently configured does not deliver the formal-verification confidence its surface area implies.** Across 6 of 7 domains, agents found rules that are vacuous, tautological, falsifiable on legitimate inputs, or reasoning over havoced cross-contract data. The single biggest cause is a systematic gap in the summary layer; the second is two concrete summary bugs that propagate into multiple domains.

The framework's *architecture* is sound — `summarized!` macro, `apply_summary!` boundary wrappers, `compat.rs` shims. The execution gap is in coverage and correctness of the summaries themselves.

## Cross-cutting findings (must fix before any meaningful prover run)

### F1 — Tuple-order bug in `calculate_account_totals_summary` (CROSS-DOMAIN, HIGH)

**Location:** `controller/certora/spec/summaries/mod.rs:158-162`
**Production:** `controller/src/helpers/mod.rs:184` returns `(total_collateral, total_debt, weighted_coll)`.
**Summary:** returns `(total_collateral, weighted_coll, total_debt)` — **slots 2 and 3 swapped**.
**Impact:** All 4 production callers (`liquidation.rs:168, 437, 470` and `views.rs:251`) destructure as `(coll, debt, weighted)` and observe wrong values under summarisation. The bound `weighted_coll <= total_collateral` is then placed on the wrong field.
**Verified by:** Liquidation review + Boundaries review (independent flags).

### F2 — `calculate_health_factor[_for]_summary` decouples HF from inputs (CROSS-DOMAIN, HIGH)

**Location:** `summaries/mod.rs:113-137`. The summary returns an independent nondet `i128 >= 0` — no relation to supply/borrow positions.
**Impact:** Every rule asserting a post-state HF property is checking a fresh draw, not the value any production codepath actually computed. **All 4 health rules broken** plus liquidation's `hf_improves_after_liquidation` (independent draws on both sides) plus strategy's HF gate verification.
**Concrete buggy implementation that would still pass:** `calculate_health_factor` returns `WAD - 1` for undercollateralized accounts, allowing borrow on every undercollateralized account. The prover sees no contradiction.

### F3 — Zero pool cross-contract summaries (CROSS-DOMAIN, HIGH)

**Location:** `pool-interface/src/lib.rs` declares 22 trait methods; `summaries/mod.rs` summarises **none** of them.
**Impact:** All cross-contract pool calls (`supply / borrow / withdraw / repay / seize_position / create_strategy / flash_loan_* / claim_revenue / add_rewards / update_indexes / get_sync_data`) are pure havoc to the prover. Every domain rule traversing `Controller::supply/borrow/withdraw/repay/multiply/liquidate` either:
- passes vacuously (post-state assertion against fresh nondet that prover picks favorably), or
- fails for the wrong reason (prover picks adversarial nondet that happens to violate property).
**Affected rule counts:**
- Solvency: 11 rules
- Interest/Index: 4 rules (`supply_index_above_floor`, `borrow_index_gte_ray`, both monotonic rules)
- Liquidation: 6 rules including `hf_improves_after_liquidation`, `bad_debt_supply_index_decreases`
- Strategy/FlashLoan: 6 of 24 rules unsound + 8 sound-but-weak
- Oracle: 8 of 8 rules (related but distinct issue — see F4)

### F4 — Oracle rules import summary instead of real implementation (HIGH)

**Location:** Every rule in `oracle_rules.rs` imports `crate::oracle::token_price` (the summary path) instead of the unsummarized real `token_price::token_price`. Result: `assume(P) → assert(P)` reflexivity.
**Concrete bug missed:** Permissive cache returning manipulated aggregator price ($1000) instead of safe TWAP ($1500); production correctly returns the safe price; a regression returning the aggregator passes every rule.
**0 of 8 oracle rules enforce a non-trivial production invariant.**

### F5 — Compat shims hard-code parameters (HIGH)

**Locations:**
- `compat::multiply` (`compat.rs:32-63`) — hard-codes `account_id=0`, `initial_payment=None`, `convert_steps=None`. Load-existing-account branch and initial-payment branch are entirely unverified.
- `compat::repay_debt_with_collateral` (`compat.rs:65-84`) — hard-codes `close_position=false`. The only path that deletes accounts in strategy is unverified.
- No `compat::liquidate` shim — `liquidation_rules.rs:79` calls `process_liquidation` directly, bypassing the `Controller::liquidate` public API auth path.

### F6 — `update_asset_index_summary` over-constrains `borrow_index >= supply_index` (MEDIUM)

**Location:** `summaries/mod.rs:96`.
**Impact:** This relation does NOT hold in production after `pool::seize_position` calls `apply_bad_debt_to_supply_index` (`pool/src/lib.rs:521-525`) — the supply index can drop below borrow index after bad-debt write-down. The summary falsely excludes these legitimate post-bad-debt states.

### F7 — HANDOFF.md drift (LOW but undermines audit credibility)

- `HANDOFF.md:155` claims `summaries/mod.rs` is "empty placeholder" — it is 218 lines, 9 active summaries.
- `HANDOFF.md:149` references `model.rs` "currently unused" — the file does not exist.
- `HANDOFF.md:126` lists `apply_summary!` wrappers as "Pending" — 9 exist, but for oracle / helpers / views only; **none for pool or SAC**.

## Domain-specific critical bugs

### Math (CRITICAL — falsifiable on legit input)

**`signed_mul_away_from_zero`** (`math_rules.rs:307-322`) asserts a wrong-direction inequality. Verified by hand: `a=-34, b=RAY/10` → production correctly returns `-3` (away-from-zero of `-3.4`), giving `result*RAY = -3e27`. Asserted `result*RAY <= a*b` evaluates to `-3e27 <= -3.4e27` which is **false**. Bound should be symmetric envelope `|result*d - a*b| <= d`.

### Solvency (BROKEN — wrong invariant)

**`withdraw_rejects_zero_amount`** asserts a **false invariant**: production at `withdraw.rs:96` treats `amount=0` as the documented "withdraw all" sentinel, NOT an error. Rule expects rejection that never happens.

### Liquidation (BROKEN)

- **`bonus_max_at_deep_underwater`** asserts `bonus == max`; summary returns any value in `[base, max]` — rule cannot pass under current summary.
- **`protocol_fee_on_bonus_only`** uses `mul_div_half_up`; production at `liquidation.rs:359` uses `div_floor`. Verifies a different formula than what ships.
- **`bad_debt_threshold`** is a propositional tautology — `qualifies` defined locally, checked against itself.
- **`clean_bad_debt_requires_qualification`** uses HF as gate; production uses `total_debt_usd > total_collateral_usd && total_collateral_usd <= 5*WAD` — different property.

### EMode/Isolation (4 high-severity)

1. **`emode_overrides_asset_params`** unsound — `apply_e_mode_to_asset_config` early-returns on `is_deprecated`, but rule lacks `cvlr_assume!(!category.is_deprecated)`. Assertions provably false in deprecated branch.
2. **`emode_remove_category`** misses the entire slim-storage refactor. Only asserts `is_deprecated == true`. Doesn't verify side-map drop, reverse-index cleanup, or `e_mode_enabled` flag clearing.
3. **Mutual-exclusion rules vacuous** in both `emode_rules.rs` and `isolation_rules.rs` — read-only over arbitrary persistent storage with no inductive framing. Also duplicated.
4. **`isolation_debt_ceiling_respected`** vacuously satisfied if borrow reverts; needs `isolated_asset.is_some()` precondition and counter-monotonicity sibling.

### Production-code findings surfaced by the verification review (NOT just verification gaps)

These are real protocol concerns the certora agents flagged while reading source:

1. **`apply_e_mode_to_asset_config` REPLACES `is_collateralizable`/`is_borrowable` flags** instead of tightening them. An e-mode override of `true` on a base `false` would silently *enable* the asset. Should be documented or gated.
2. **`add_asset_to_e_mode_category` does not reject `is_isolated_asset`** — admin can persist a self-conflicting config that always reverts at runtime.
3. **No production rule for `apply_bad_debt_to_supply_index` floor clamp** — the most security-critical math invariant in the system. Regression here drains supplier funds. Also no test of `add_protocol_revenue_ray`'s floor short-circuit.

## Storage-refactor coverage gap

The recent slim-`AccountMeta` + side-map refactor is structurally compatible with boundary rules (don't read `ControllerKey` variants directly), but **no Certora rule asserts the new lifecycle invariants:**
- meta + side-map atomic removal on close
- side-write → meta-TTL bump (load-bearing for keep-alive)
- no-orphan-side-map after e-mode category deprecation
- create_account writes meta + 0 supply rows + 0 borrow rows atomically

## Recommended remediation order

### P0 — fix before any prover run (the math is meaningless without these)

1. **F1 tuple-order bug** in `calculate_account_totals_summary` — single-line swap.
2. **Drop F6 over-constraint** `borrow_index >= supply_index` from `update_asset_index_summary`.
3. **Author `summaries/pool.rs`** with the 11 most-used pool methods (supply, borrow, withdraw, repay, seize_position, create_strategy, get_sync_data, claim_revenue, update_indexes, flash_loan, add_rewards). Each summary's `cvlr_assume!` bounds must match the real function's contract. Without these, F3 leaves all domain rules unsound.
4. **Author `summaries/sac.rs`** for SAC token operations (transfer, balance, mint).
5. **Fix oracle rules to use unsummarized `token_price`** (F4) — every oracle rule must call the real implementation, not the summary path.
6. **Remove HF summary OR refactor to take supplies/borrows as inputs and compute deterministically** (F2) — currently every health rule is checking independent draws.

### P1 — fix before audit handoff

1. **`signed_mul_away_from_zero`** — fix wrong-direction inequality (math_rules.rs).
2. **`withdraw_rejects_zero_amount`** — invariant is false; either delete or rewrite to test a real reject case.
3. **Liquidation bonus / fee formula mismatches** — align rules with production (`div_floor`, not `mul_div_half_up`).
4. **EMode high-severity items** — add deprecated-category precondition, expand `emode_remove_category` to verify side-map cleanup, fix mutual-exclusion framing.
5. **`compat::multiply`** — parameterize `account_id`, `initial_payment`, `convert_steps`. Add `compat::repay_debt_with_collateral` parameterizing `close_position`. Add `compat::liquidate`.
6. **Production code:** decide intent for `apply_e_mode_to_asset_config` flag-replacement; gate `add_asset_to_e_mode_category` against `is_isolated_asset`.

### P2 — coverage uplift after P0/P1

1. **Storage-refactor lifecycle rules** (4 invariants enumerated above).
2. **Bad-debt floor clamp rules** for `apply_bad_debt_to_supply_index` and `add_protocol_revenue_ray`.
3. **Liquidation invariants** — anti-rug bound on total seized vs total repaid, refund conservation, account-totals strict decrease.
4. **Oracle invariants** — 10 missing items enumerated in `04-oracle.md`.
5. **Strategy invariants** — 8 high-impact items including pool-borrow conservation, allowance-zero pre/post swap, reentrancy-during-aggregator-callback.
6. **Sync HANDOFF.md** with reality (F7).

## Aggregate counts

| Domain | Rules | Strong | Weak | Broken | Tautology | Missing |
|---|---:|---:|---:|---:|---:|---:|
| Solvency / Health / Position | 30 | ~12 | 11 | 5 | 2 | 15 |
| Liquidation | ~15 | 2 | 5 | 4 | 4 | 14 |
| Interest / Index / Math | ~30 | ~11 | 8 | 3 | 0 | 8 |
| Oracle | 8 | 0 | 1 | 5 | 2 | 10 |
| EMode / Isolation | ~20 | ~12 | 4 | 4 | 0 | 5 |
| Strategy / FlashLoan | 24 | 10 | 8 | 6 | 0 | 8 |
| Boundaries / Summaries / Compat | 38 + 9 sums | most | 5 | 5 | — | 14 |
| **Totals** | **~165** | **~47** | **~42** | **~32** | **~8** | **~74** |

(Counts are approximate — agents disagreed on borderline weak/broken cases. The summary uses each agent's own classification.)

**Strong rules represent ~28% of the total; ~50% have material issues; ~74 high-priority invariants are missing.**

## Net verdict

The certora setup gives the *appearance* of comprehensive formal verification (165 rules, 102 documented as "strong" in HANDOFF.md), but the underlying summaries layer has gaps and bugs that propagate through every domain. **A clean prover run on the current rule set would produce confidence that does not match reality.** Fixing the P0 list (single-line tuple swap, single-line bound removal, two summary-file authoring tasks, oracle import path fix, HF summary refactor) is achievable in 1-2 days and would unlock most of the existing rules' actual verification value.
