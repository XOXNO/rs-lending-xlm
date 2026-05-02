# Certora Efficiency Review — Consolidated Summary

**Date:** 2026-05-02
**Domains audited:** 7 (Solvency/Health/Position, Liquidation, Interest/Index/Math, Oracle, EMode/Isolation, Strategy/FlashLoan, Boundary/Summaries/Compat)
**Per-domain reports:** `audit/certora-efficiency/0[1-7]-*.md`
**Predecessor:** `audit/certora-review/SUMMARY.md` (soundness; partly remediated in P0/P1)

## Headline verdict

**Five of seven agents independently flagged the same blocker: the pool and SAC summaries we authored in P0a are inert.** They define correct contracts but no `summarized!` wrapper binds them at the ~14 cross-contract call sites in `controller/src/{positions,cache,router,utils,flash_loan,strategy}/...`. Until those wrappers are added, every domain rule that traverses `pool_client.X(...)` or `token::Client::X(...)` reasons over fully havoced returns — *simultaneously* expensive (full prover exploration of cross-contract paths) AND verification-vacuous (post-state assertions against fresh nondets prove nothing).

The framework as it stands cannot deliver useful PASS verdicts on most of the protocol's safety properties. The good news: wiring ~14 one-line wrappers is a small mechanical change that unlocks the entire summary layer.

## P0 — single highest-leverage fix (must land before next prover run)

### Wire the pool + SAC summaries into production call sites

The 16 summaries authored in `controller/certora/spec/summaries/{pool,sac}.rs` are not currently used by any rule. Each cross-contract call site needs a `summarized!` wrapper analogous to those already in `controller/src/lib.rs:13-22` and `controller/src/helpers/mod.rs:55,118`.

| Production site | Cross-contract call | Summary to bind |
|---|---|---|
| `controller/src/positions/supply.rs:370` | `pool_client.supply` | `supply_summary` |
| `controller/src/positions/borrow.rs:55` | `pool_client.create_strategy` | `create_strategy_summary` |
| `controller/src/positions/borrow.rs:262` | `pool_client.borrow` | `borrow_summary` |
| `controller/src/positions/repay.rs:104` | `pool_client.repay` | `repay_summary` |
| `controller/src/positions/withdraw.rs:108` | `pool_client.withdraw` | `withdraw_summary` |
| `controller/src/positions/liquidation.rs:*` | `pool_client.seize_position` | `seize_position_summary` |
| `controller/src/cache/mod.rs:152` | `pool_client.get_sync_data` | `get_sync_data_summary` |
| `controller/src/router.rs:*` | `pool_client.update_indexes`, `claim_revenue` | corresponding summaries |
| `controller/src/flash_loan.rs:*` | `pool_client.flash_loan_begin`, `flash_loan_end` | corresponding summaries |
| `controller/src/utils.rs::transfer_and_measure_received` | `token::Client::balance`, `transfer` | `balance_summary`, `transfer_summary` |

Each wrapper is a one-line `summarized!(summary_path, original_call(...))` macro invocation around the existing call. No semantic change when the certora feature is off (the macro is a no-op).

**This is a production code change** — strictly out of the "no implementation changes" scope you set earlier. Flagging explicitly so you can decide whether to:
- (a) Land the wrappers as a one-time exception (high-leverage, low-risk because the macro is no-op without the feature flag), OR
- (b) Have the certora team land them as part of their engagement (longer turnaround), OR
- (c) Refactor `LiquidityPoolClient` itself (most invasive — `soroban_sdk::contractclient` generates the trait; `cvlr_mock_client` would replace it but that's a vendor-level change).

**Recommendation: (a).** The wrapper pattern is already established in helpers/views; extending it to pool/SAC sites preserves the established convention.

## P0 — two summary contracts still broken post-P0/P1 remediation

Both flagged independently by 2+ agents:

### F1 — `calculate_health_factor[_for]_summary` central case unconstrained

**Location:** `controller/certora/spec/summaries/mod.rs:131-148, 157-172`.

**Status post-P0b:** my refactor added input-tied bounds for the edge cases (empty borrows → MAX, empty supply with debt → 0). The **central case (both maps non-empty)** is still a free nondet `i128 >= 0`.

**Impact:** rules of the form `hf_after >= hf_before` are still vacuously refutable because the prover can pick `hf_before = 100, hf_after = 0` for the same pre/post state. Affects `health_rules::supply_cannot_decrease_hf`, `liquidation_rules::hf_improves_after_liquidation`, boundary rules 6-7.

**Fix options:**
- **A**: Add a same-run-must-be-deterministic constraint. The prover supports it but adds branching.
- **B**: Refactor rules to inline math against unsummarised `helpers::position_value` / `weighted_collateral` / `calculate_ltv_collateral_wad`. Heavier per rule but eliminates the summary-roundtrip entirely. Solvency agent's preferred approach.
- **C**: Have the summary additionally assume that within the same proof, two calls with the same inputs return the same value (function-purity contract).

### F2 — `calculate_linear_bonus_summary` admits a value production never returns

**Location:** `controller/certora/spec/summaries/mod.rs:209-219`.

**Status:** admits all values in `[base_bonus, max_bonus]`. Production at `helpers/mod.rs::calculate_linear_bonus` returns *exactly `base_bonus`* when `HF >= target_hf` (= 1.02 WAD); the prover-allowed range is wider.

**Impact:** `boundary_rules::bonus_at_hf_exactly_102` will FAIL the prover because it asserts `bonus == base_bonus` at the boundary, but the summary admits `bonus = max_bonus`.

**Fix:** add the boundary case to the summary:
```rust
if hf.raw() >= 102 * WAD / 100 {
    cvlr_assume!(bonus_raw == base_bonus.raw());
}
```

## P0 — pool view methods missing from summaries

The Solvency agent flagged 5 pool view methods we did NOT add to `summaries/pool.rs`:

- `reserves(asset) -> i128`
- `supplied_amount(asset) -> i128`
- `borrowed_amount(asset) -> i128`
- `protocol_revenue(asset) -> i128`
- `capital_utilisation(asset) -> Wad`

Several rules call 2-4 of these per body. Without summaries each return is independent havoc → joint identities (e.g., `borrowed <= supplied`) are unprovable.

**Fix:** extend `summaries/pool.rs` with these 5 read-only summaries. Each is small (~10 lines) with bounds derived from production (all >= 0, the borrowed/supplied/revenue ratios bounded by storage state).

## P0 — Reflector summary (highest-leverage missing piece for oracle)

The Oracle agent's #1 finding: **author `controller/certora/spec/summaries/reflector.rs`**.

The P0b rewrite of oracle rules to use unsummarised `crate::oracle::token_price::token_price` was correct in intent but heavy in execution: Reflector contract calls inside `token_price` are still havoced. With a Reflector summary (`lastprice` and `prices`), the cache-consistency rule's 18-path traversal collapses to 1.

**Wiring:** Reflector requires thin Rust wrappers around the `#[contractclient]` trait so `summarized!` can intercept (~20-line production refactor). Same scope question as the P0 wiring above.

## P1 — efficiency wins per domain (no production code needed)

These are pure spec-side changes. Combined estimated impact: **5–30× speedup on math/index rules; multiple TAC-explosion sources eliminated**.

### Math/Interest/Index

- **Tighten `nondet_valid_params`** (lines around `interest_rules.rs:24`): caps slopes/base at `RAY * 10` (5× production `MAX_BORROW_RATE_RAY`). Lowering to production cap + adding slope-monotonicity gives **5–30× speedup on 13/14 rules**.
- **Prune 7 `*_sanity` rules** in `math_rules.rs` that duplicate the assertion-rule preconditions with `cvlr_satisfy!`. Pure solver tax. **~40–50% file speedup.**
- **Add `cvlr_assume!(t1 > 0)`** to compound-interest two-call rules. Prunes the `t1 == 0` branch; ~0% efficiency cost, real soundness gain.
- **Pin compound-interest to one rate region per rule** (slope1 OR slope2 OR slope3, not all three). Each region has its own arithmetic; combining them blows up the Taylor expansion.

### Liquidation

- **Delete** `bad_debt_threshold` and `bad_debt_supply_index_decreases` (the former is tautological, the latter reads from wrong contract). **~70% conf wall-time reduction.**
- **Replace** `hf_improves_after_liquidation` with single-asset rules R5/R6 (proposed in domain report).
- **Add** `liquidation-light.conf` with `loop_iter: 1` for action-focused rules; keep the heavyweight conf for e2e validation.
- **Estimated**: conf fits inside `-maxBlockCount 200000` after deletions.

### Solvency / Health / Position

- **Pin `account_id = 1`** across ~14 rules (currently fully symbolic u64).
- **Delete** 6 unsummarised-pool-view rules (zero verification value): `pool_reserves_cover_net_supply`, `revenue_subset_of_supplied`, `borrowed_lte_supplied`, `borrow_index_gte_supply_index`, the three `*_scaled_conservation` rules.
- **Rewrite** all 4 health rules + `ltv_borrow_bound_enforced` as inline math against unsummarised `position_value` / `weighted_collateral`.
- **Fix** unbounded `for i in 0..len()` loop in `supply_position_limit_enforced:356` and the borrow analog.

### EMode/Isolation

- **`emode_remove_category`**: bound the side-map size, add `e_mode_enabled` post-condition.
- **`emode_isolation_mutual_exclusion_after_supply`**: add `cvlr_assume!(account_id == 0)` (exercise create-new branch only); add multiply-path sibling.
- **Tighten** weak post-conditions on isolation ceiling/repay rules (still vacuous on revert; strict-decrease wrong on dust-floor edge).

### Oracle

- **Pin** `(MarketStatus, ExchangeSource, allow_unsafe_price)` configurations per rule (currently 36 paths through `token_price`; pin to one config per rule = 4× speedup minimum).
- **Refactor** `tolerance_bounds_valid` and similar to call `calculate_final_price` directly (unsummarised, scalar inputs, ~5× cheaper than the full `token_price`).
- **Author** Reflector summary (P0 above).

### Strategy/FlashLoan

- **Split** `multiply_creates_both_positions` into 3 per-branch rules (initial_payment None / Some / convert_steps None / Some — currently 32 entry-shape paths × 11 free symbols).
- **Split** `repay_with_collateral_reduces_both` into close/no-close variants (the close path has unbounded `execute_withdraw_all` loop).
- **Delete** the 4 `strategy_blocked_during_flash_loan_*` rules — duplicates of `flash_loan_guard_blocks_callers`.
- **Add** `compat::multiply_minimal` (no nondet args) for negative-path rules where the panic fires before any branching matters.
- **Fix** `flash_loan_guard_cleared_after_completion` revert-vacuity (P1 audit flagged this; not addressed in remediation).
- **Fix** `clean_bad_debt_requires_qualification` (uses HF summary; should use raw `total_debt > total_coll`).

### Boundary

- **Delete** ~13 of 19 boundary assertion rules (5 strict-stronger duplicates of interest/math; 6 logical tautologies; 2 vacuously refutable under input-tied HF; 2 effectively tautological after ceremony).
- **Keep** `mul_at_max_i128`, `compound_taylor_accuracy`, both `rescale_*_to_*` — these give unique signal.

## Action-focused replacement set (overall direction)

Each domain agent proposed its own replacement set. The consolidated direction:

**Per entry point, write 2-4 tightly-scoped rules:**

1. **Happy-path action rule** — fixed `account_id`, single asset, bounded amount, concrete pre-state. Asserts the canonical post-state delta (e.g., "after supply, scaled_amount increased by ≈amount/index").
2. **Reject-path rule** per panic site — pin the exact panic preconditions, assert `panic_with_error`. Cheap, high-signal.
3. **One state-invariant rule per safety property** — minimal preconditions, asserts the invariant. Heavier; reserve for properties that can't be reduced to action-deltas.

Total rule count would shrink from ~190 to ~80-100 across all domains, with most rules running in ≤30s.

## Estimated prover budget impact (rough order-of-magnitude)

Combining the recommendations:

| Source | Estimated change |
|---|---|
| Wiring pool/SAC summaries (P0) | every rule traversing pool ops becomes 5–10× faster + actually meaningful |
| Tightening `nondet_valid_params` | 5–30× speedup on math/interest/index |
| Pruning sanity duplicates in math | ~40–50% math file speedup |
| Pinning oracle config combinations | 4–10× per oracle rule |
| Deleting heavyweight liquidation rules | ~70% liquidation conf wall-time |
| Pruning redundant boundary rules | ~16% → ~80% signal-to-budget |
| Splitting multiply / repay-with-collateral | each split: 3× speedup, +better coverage |

**Net estimate: prover total wall time should fit comfortably under the engagement's `-maxBlockCount` cap with rules that deliver concrete PASS verdicts on real safety properties.**

## Recommended remediation order

1. **P0a — wire pool + SAC summaries** (production change, ~14 wrappers). Needs your decision.
2. **P0b — fix the two broken summary contracts** (HF central case, calculate_linear_bonus). Pure spec changes.
3. **P0c — add 5 missing pool view summaries** (reserves / supplied_amount / borrowed_amount / protocol_revenue / capital_utilisation).
4. **P0d — author `summaries/reflector.rs`**. Optional production wrappers (~20 lines) to wire Reflector summarisation.
5. **P1 — per-domain efficiency cleanups** as enumerated above. Pure spec changes; can be done in parallel by domain agents.
6. **P2 — replacement rule-set** per entry point. Larger spec rewrite; do after P0/P1 stabilise.

Each step is independently verifiable via `cargo check --features certora` (compilation) and the engagement team's prover dispatch (semantic).
