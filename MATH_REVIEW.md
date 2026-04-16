# Math & Verification Review

A review of the math components in `controller/src/` and `pool/src/` against the
Certora-style rules and summaries in `controller/certora/spec/`, and against the
claims in `INVARIANTS.md`.

Goals:

- catch stale, tautological, weak, or vacuous rules
- catch invariants stated in docs but not enforced by any rule
- catch implementation drift (what the code does vs. what docs say it does)
- propose a tight remediation plan

Citations are `path:line` into the current tree.

---

## 0. Remediation Progress

This review was followed by a first remediation pass. See
[`controller/certora/SPIKES.md`](./controller/certora/SPIKES.md) for the
ground-truth investigation into the Certora harness that informed
several corrections in this document.

**Toolchain note:** the `cvlr-spec` compile blocker SPIKES.md Spike A
flagged is **resolved** in the current tree by vendoring CVLR under
`vendor/cvlr/` with `#![no_std]` patched. `cargo check --features
certora` passes. End-to-end prover verification (running
`certoraSorobanProver` against a conf) was not exercised in this
review; that one empirical run remains as action item §1. See §3.1.2.

Completed so far:

| Item | Status | Location |
|---|---|---|
| Fix fragile `CompatAssetConfig.debt_ceiling_usd_wad` rename | Done | `controller/src/storage/mod.rs:494,517`, `controller/certora/spec/isolation_rules.rs:141` |
| Add §12 — claim revenue ≤ reserves | Done | `solvency_rules::claim_revenue_bounded_by_reserves` |
| Add §8 — utilization zero when supplied_ray zero | Done | `solvency_rules::utilization_zero_when_supplied_zero` |
| Add §11 — isolation debt non-negative after repay | Done | `solvency_rules::isolation_debt_never_negative_after_repay` |
| Add §13 — borrow respects reserves | Done | `solvency_rules::borrow_respects_reserves` |
| Add §10 — LTV borrow bound enforced | Done | `solvency_rules::ltv_borrow_bound_enforced` |
| Add §7 — supply_index stays above floor across supply | Done | `solvency_rules::supply_index_above_floor_after_supply` |
| Add §7 — supply_index monotone across borrow | Done | `solvency_rules::supply_index_monotonic_across_borrow` |
| Register new rules in `confs/solvency.conf` | Done | 190 source rules, 0 orphans |
| Fix doc drift (Taylor/sentinel/floor) in INVARIANTS.md | Done | §1.4.1 |
| Resolve `cvlr-spec` compile blocker (SPIKES.md Spike A) | Done | `vendor/cvlr/`, workspace `[patch]` block |
| Empirical `certoraSorobanProver <conf>` run with vendored stack | Pending | — |
| Delete or repurpose misleading `summaries/mod.rs` | Pending | `controller/certora/spec/summaries/` |
| Add `apply_summary!` wrappers at pool/oracle/SAC call sites | Pending | see §3.1.1 |
| Delete dead `model.rs` ghost vars | Pending | `controller/certora/spec/model.rs` |
| Rewrite 13 tautological rules to call prod | Pending | see §3.2 |

Drift risk introduced: `supply_index_above_floor_after_supply`
hard-codes `SUPPLY_INDEX_FLOOR_RAW = 10^18`. If `pool/src/interest.rs:14`
ever changes, this rule silently diverges. Consider moving the constant
to `common::constants` and re-exporting.

## 1. Executive Summary

| Category | Count | Severity |
|---|---|---|
| Stale / fragile rules | 1 fragile, 0 broken | Medium |
| Tautological rules (local reimplementation) | 13 | High |
| Weak rules (loose bound or positivity-only) | 6 | Medium |
| Vacuous / ambiguous rules | 5 | Medium |
| Documented invariants with NO rule coverage | 8 | High |
| Documentation drift vs. implementation | 5 | High |
| Latent math concerns with no PoC | 7 | Medium |

Top five action items:

1. **Verify the Certora prover runs end-to-end** (see §3.1.2). The
   vendor-patched `cvlr-spec` removes the specific compile blocker
   SPIKES.md flagged; compile-step checks pass. What remains is one
   empirical `certoraSorobanProver controller/confs/math.conf` run to
   confirm the prover now produces verdicts.
2. Add `apply_summary!` wrappers at controller-side call sites for
   pool, Reflector, and SAC methods (see §3.1.1). The empty
   `controller/certora/spec/summaries/mod.rs` is a misleading leftover;
   summaries live at source sites, not in the spec tree.
3. Replace 13 tautological rules that reimplement logic locally and assert
   properties of the local reimplementation. They pass today and prove
   nothing.
4. Correct INVARIANTS.md on Taylor order, withdraw-all sentinel, and the
   supply-index floor magnitude (see §4.1). **Done — §0.**
5. Backfill uncovered invariants §8 / §10 / §11 / §12 / §13 / §7 floor.
   **Done — §0 (7 new rules, all compile, all registered in
   `confs/solvency.conf`).**

---

## 2. Scope And Method

### In scope

- `controller/src/` — all files
- `pool/src/` — `cache.rs`, `interest.rs`, `views.rs`, `lib.rs`
- `common/src/` — `fp.rs`, `fp_core.rs`, `rates.rs`, `constants.rs`,
  `types.rs`
- `controller/certora/spec/` — 16 spec files + `summaries/`
- `INVARIANTS.md`, `ARCHITECTURE.md`, `README.md`, `DEPLOYMENT.md`

### Method

- Enumerated every math-bearing function in controller and pool.
- Classified every `#[rule]` declaration under one of: live / stale /
  vacuous / tautological / weak / strong / ambiguous.
- Built a coverage matrix of INVARIANTS.md sections ↔ rule IDs.
- Flagged any prod-path raw arithmetic that skips half-up rounding or skips
  overflow checks.
- Traced every division in the prod path to a div-by-zero guard.

No PoCs were run. No Certora runs were executed.

---

## 3. Certora Rules Audit

### 3.1 Structural concerns

#### 3.1.1 Misnamed `summaries/mod.rs` module

File: `controller/certora/spec/summaries/mod.rs`.

**Earlier ground-truth investigation in
[`controller/certora/SPIKES.md`](./controller/certora/SPIKES.md) (Spike B)
established that this module CANNOT hold summaries.** The real CVLR
summary mechanism is the `cvlr_soroban_macros::apply_summary!` macro,
which wraps a function definition in-place at its source site. Example:

```rust
// at the function's own source location
cvlr_soroban_macros::apply_summary!(path::to::spec_fn,
    pub fn original_fn(env: Env, arg: T) -> R {
        // real implementation
    }
);
```

Under `--cfg feature="certora"` the body becomes `spec_fn`; otherwise the
real body runs. To summarize a cross-contract call you cannot own
(`LiquidityPoolClient::reserves`, `ReflectorClient::prices`), you write a
**local wrapper function** in the controller and apply `apply_summary!`
around that wrapper.

Consequences:

- Every rule that directly calls `LiquidityPoolClient::method()` currently
  runs with whatever behavior the `#[contractclient]`-generated client
  produces under certora. What that is, and whether those calls are
  symbolically modelled or havoced, depends on the Certora toolchain and
  is not documented in this repo.
- The 7 rules added in §0 inherit the same uncertainty — they match the
  style of the existing pool-reads rules.
- `summaries/mod.rs` has been empty since creation and will stay empty
  because the mechanism lives elsewhere.

**Remediation:** delete `summaries/mod.rs` (and the `summaries` directory)
as a misleading leftover, OR convert it into a documentation file
explaining where real summaries live. Add `apply_summary!` wrappers at
the controller-side call sites for:

- pool reads: `reserves`, `supplied_amount`, `borrowed_amount`,
  `protocol_revenue`, `get_sync_data`, `capital_utilisation`.
- pool mutations: `supply`, `borrow`, `withdraw`, `repay`,
  `update_indexes`, `seize_position`, `claim_revenue`,
  `flash_loan_begin`/`end`, `create_strategy`.
- Reflector reads: `fetch_price`, `fetch_twap`.
- SAC reads/transfers: `balance`, `transfer`.

Each wrapper's spec function should be a minimal havoc that preserves the
invariants in [`INVARIANTS.md`](./INVARIANTS.md).

#### 3.1.2 Toolchain status

The compile blocker SPIKES.md Spike A flagged — `error[E0463]: can't
find crate for core` inside `cvlr-spec/src/spec.rs` — is **resolved in
the current tree** by vendoring CVLR under `vendor/cvlr/` and
patch-applying `#![no_std]` to `cvlr-spec/src/lib.rs`. The workspace
`Cargo.toml` (see "Vendored cvlr with `#![no_std]` patched" block)
redirects every `cvlr-*` crate to the vendored copy.

Evidence of compile unblock:

- `cargo check -p controller --features certora --no-default-features`
  passes cleanly (only two pre-existing unused-import warnings in
  `oracle_rules.rs`). Same command was failing for SPIKES.md's author
  at `cvlr-spec` build.
- `vendor/cvlr/cvlr-spec/src/lib.rs:1` is `#![no_std]` — the exact
  missing attribute SPIKES.md identified.

What this does NOT establish:

- Whether `certoraSorobanProver <conf>` runs end-to-end against the
  vendored stack and produces verdicts. The prover binary is not
  present at the repo-local `.certora-venv/bin/` path and was not
  exercised during this review.
- Whether rules that call `LiquidityPoolClient::*` methods hit any
  additional modelling gap once the prover runs.

**Recommended verification gate:** run
`certoraSorobanProver controller/confs/math.conf` (the simplest conf
in the tree) with the vendored stack. If that returns a verdict, the
remaining remediation items in §6 become actionable. If it still
fails, SPIKES.md's original guidance (escalate to Certora / self-host
known-good container) applies but with a smaller surface — the
`cvlr-spec` fix is already done.

#### 3.1.2 Dead ghost scaffolding in `model.rs`

File: `controller/certora/spec/model.rs:11-95`.

Declares `GHOST_HEALTH_CHECKED`, `GHOST_FLASH_LOAN_GUARD_SET`,
`GHOST_SUPPLY_INDEX_BEFORE`, `GHOST_BORROW_INDEX_BEFORE`, plus two skolem
vars. `rg spec::model::` across the spec tree finds zero consumers. Dead
scaffolding today. Either wire it into rules that need before/after
snapshots, or delete.

#### 3.1.3 Compat shim field rename

File: `controller/src/storage/mod.rs:482-517`.

`CompatAssetConfig` exposes the real struct's
`isolation_debt_ceiling_usd_wad` as `debt_ceiling_usd_wad` (line 517:
`debt_ceiling_usd_wad: cfg.isolation_debt_ceiling_usd_wad`). This exists
only to keep `isolation_rules::isolation_debt_ceiling_respected:141`
compiling.

Risk: if the production field is ever renamed or removed, the shim still
compiles and the rule silently asserts against a possibly stale value.

**Remediation:** rename the shim field to match the prod struct, or remove
the shim entirely and let the rule read `AssetConfig` directly.

### 3.2 Tautological rules

These rules reimplement the property locally and then assert against the
reimplementation. They prove nothing about the prod code path.

| ID | File:line | What it does |
|---|---|---|
| `oracle_rules::first_tolerance_uses_safe_price` | `oracle_rules.rs:56-86` | Rewrites `if within_first → safe`, asserts the rewrite |
| `oracle_rules::second_tolerance_uses_average` | `oracle_rules.rs:95-151` | Rewrites `(agg+safe)/2` locally, asserts properties of the local value |
| `oracle_rules::beyond_tolerance_blocks_risk_ops` | `oracle_rules.rs:161-197` | Local rewrite; `!within_second && !allow_unsafe → assert false` is vacuous without a forcing summary |
| `oracle_rules::tolerance_bounds_valid` | `oracle_rules.rs:230-270` | Asserts restatements of its own `assume!`s |
| `boundary_rules::liquidation_at_hf_exactly_one` | `boundary_rules.rs:210-219` | Assumes `hf == WAD`, asserts `hf >= WAD` |
| `boundary_rules::liquidation_at_hf_just_below_one` | `boundary_rules.rs:232-241` | Same pattern |
| `boundary_rules::bad_debt_at_exactly_5_usd` | `boundary_rules.rs:287-302` | Local `qualifies` predicate, not the prod guard in `liquidation.rs:413` |
| `boundary_rules::bad_debt_at_6_usd` | `boundary_rules.rs:316-330` | Same pattern |
| `boundary_rules::tolerance_at_exact_first_bound` | `boundary_rules.rs:444-462` | Restates assumes |
| `boundary_rules::tolerance_at_exact_second_bound` | `boundary_rules.rs:477-493` | Restates assumes |
| `boundary_rules::tolerance_just_beyond_second` | `boundary_rules.rs:509-527` | Restates assumes |
| `boundary_rules::borrow_exact_reserves` | `boundary_rules.rs:572-584` | Pure local arithmetic, no pool call |
| `boundary_rules::withdraw_more_than_position` | `boundary_rules.rs:600-612` | Pure `i128::min` tautology |
| `liquidation_rules::seizure_proportional` | `liquidation_rules.rs:132-164` | Re-derives proportional split locally |
| `liquidation_rules::protocol_fee_on_bonus_only` | `liquidation_rules.rs:174-205` | Same |
| `liquidation_rules::bad_debt_threshold` | `liquidation_rules.rs:214-237` | Restates assumes |

**Remediation pattern:** each rule must call the actual prod function
(`oracle::calculate_final_price`, `liquidation::calculate_seized_collateral`,
`liquidation::check_bad_debt_after_liquidation`, etc.) and assert a
property about its result.

### 3.3 Weak rules

Rules that look like coverage but assert almost nothing.

| ID | File:line | Why weak |
|---|---|---|
| `solvency_rules::supply_scaled_conservation` | `solvency_rules.rs:269-311` | Only asserts `scaled_delta > 0`; docstring claims "±1 of `calculate_scaled_supply`" but that equation is never computed |
| `solvency_rules::borrow_scaled_conservation` | `solvency_rules.rs:320-359` | Same pattern — positivity only |
| `solvency_rules::compound_interest_bounded_output` | `solvency_rules.rs:741-758` | Asserts `<100_000 * RAY`; docstring says "<100*RAY". The looser bound passes trivially |
| `strategy_rules::claim_revenue_transfers_to_accumulator` | `strategy_rules.rs:474-484` | Only `amount >= 0`; no delta check on the accumulator, the pool reserve, or the supply |
| `liquidation_rules::ideal_repayment_targets_102` | `liquidation_rules.rs:283-330` | Asserts bounds, not the HF→1.02 property the docstring promises |
| `interest_rules::compound_interest_ge_simple` | `interest_rules.rs:311-336` | Tolerance of `-2` allows simple − 2; docstring claims `compound ≥ simple` |

### 3.4 Vacuous / ambiguous rules

| ID | File:line | Issue |
|---|---|---|
| `emode_rules::emode_add_asset_to_deprecated_category` | `emode_rules.rs:389-395` | Never asserts that the category is deprecated. Rule passes for any category. |
| `health_rules::supply_cannot_decrease_hf` | `health_rules.rs:93-111` | Creates two separate `ControllerCache` snapshots which may observe different synced indexes. Post-delta comparison can spuriously fail or pass. |
| `position_rules::full_repay_clears_debt` | `position_rules.rs:70-80` | Uses `amount = i128::MAX`. Whether the prod path accepts that literal depends on caps and overflow guards; if the call reverts, the rule passes vacuously. |
| `liquidation_rules::bad_debt_supply_index_decreases` | `liquidation_rules.rs:247-266` | Uses `debt_asset = env.current_contract_address()` (the controller's own address). Reads an uninitialized market index. The `assume` before the call may be unsatisfiable. |
| `solvency_rules::mode_transition_blocked_with_positions` | `solvency_rules.rs:684-722` | The five assumes (`e_mode > 0 && !is_isolated && borrow_list > 0 && is_isolated_asset`) form a tight conjunction. Confirm that a satisfying pre-state exists or the rule is vacuous. |

### 3.5 Deleted rules kept as comments

Acknowledged in source; confirm they are not needed:

- `flash_loan_rules.rs:11-16`
- `strategy_rules.rs:487-493`
- `liquidation_rules.rs:66-71, 93-98, 269-273`

### 3.6 Strong rules (keep)

High-value rules that actually prove a property of the real code. Do not
lose these in a refactor:

- `solvency_rules`: `revenue_subset_of_supplied`, `borrowed_lte_supplied`,
  `supply_withdraw_roundtrip_no_profit`, `borrow_repay_roundtrip_no_profit`,
  `borrow_index_gte_supply_index`, `supply_index_grows_slower`,
  `index_cache_single_snapshot`, `price_cache_invalidation_after_swap`,
  the zero-amount rejection rules, both position-limit rules.
- `math_rules`: all 12 rules (half-up convention coverage).
- `index_rules`: all 5 rules (monotonicity).
- `interest_rules`: borrow-rate piecewise coverage, monotonicity,
  supplier-rewards conservation, index monotonicity.
- `emode_rules`: 1, 2, 3a/b, 4, 5, 6, 8, 10, and mutual-exclusion invariant.
- `strategy_rules` multiply/swap/repay strong-side coverage and the flash
  loan blocker set.
- `flash_loan_rules`: 2, 3, 4.

### 3.7 INVARIANTS ↔ rules coverage matrix

Columns: invariant §, covered-by, strength.

| INVARIANTS.md § | Topic | Covered by | Strength |
|---|---|---|---|
| 1 | Fixed-point domains | `math_rules::rescale_*`, `boundary_rules::rescale_*` | Strong |
| 2 | Half-up rounding | `math_rules` 1-12 | Strong |
| 3 | Scaled balance | `position_rules` 1-5 | **Weak — positivity only** |
| 4 | Pool state identity `revenue_ray ≤ supplied_ray` | `solvency_rules::revenue_subset_of_supplied` | Strong |
| 5 | Interest split | `interest_rules::supplier_rewards_conservation` | Strong |
| 6 | Borrow index monotonicity | `index_rules::borrow_index_monotonic_after_accrual`, `interest_rules::update_borrow_index_monotonic` | Strong |
| 7 | Supply index monotonicity + bad-debt exception + floor | `index_rules::supply_index_monotonic_after_accrual` | **Partial — bad-debt exception and floor not modelled** |
| 8 | Utilization div-by-zero convention | — | **Not covered** |
| 9 | Health factor | `health_rules` 1-3, `boundary_rules` 6, 7 (tautological) | Partial |
| 10 | LTV borrow bound (≠ liquidation threshold) | — | **Not covered** |
| 11 | Isolation debt (never negative, ceiling, dust) | `isolation_rules::isolation_debt_ceiling_respected` | **Partial — dust rule and non-negativity not covered** |
| 12 | Claim revenue ≤ reserves | — | **Not covered** (the one candidate rule is weak) |
| 13 | Reserve availability on withdraw/borrow/flashloan | — | **Not covered** |
| 14 | Market oracle invariants (decimals discovery) | — | **Not covered** |
| 15 | Controller/pool separation | — | **Not covered (architectural)** |
| 16 | Account storage invariant (meta ↔ positions consistency) | — | **Not covered** |

---

## 4. Implementation Drift

### 4.1 INVARIANTS.md vs. code

| Claim in INVARIANTS.md | Section | Reality in code |
|---|---|---|
| "5-term Taylor approximation of `e^(rate*time)`" | §6 | **8-term Taylor.** `common/src/rates.rs:67-105` includes `x^7/5040` and `x^8/40320`. Per inline comment this was tightened (M-08). |
| "caps full withdraws when `amount = 0`" | §A flow; mirrored in ARCHITECTURE.md §Withdraw | **Both sentinels exist.** Controller maps `amount == 0 → i128::MAX` at `controller/src/positions/withdraw.rs:84` (comment `// 0 = withdraw all`); pool then takes the full-withdraw branch via `amount ≥ current_supply_actual` at `pool/src/lib.rs:181-183`. ARCHITECTURE.md note at line 246 corrected. |
| "During bad debt application, the new supply index floors at `1`. So `supply_index_ray ≥ 1` always holds." | §7 Safety floor | **Floor is `SUPPLY_INDEX_FLOOR_RAW = 10^18`** (`pool/src/interest.rs:14, 131-135`). In raw Ray that's 10^-9 of nominal, not 1. The "always holds" claim is correct only if you read "1" as "10^18 raw". Ambiguous today. |
| "`add_protocol_revenue` preserves the invariant by incrementing both `revenue_ray` and `supplied_ray`." | §4 | True for the `_ray` variant, with a silent-drop branch: if `supply_index < 10^18 raw`, the fee is skipped, not queued (`pool/src/interest.rs:63-75`). Should be documented. |
| "revenue claims burn scaled revenue from both in the same proportion." | §4, §12 | True in the full-claim branch. The partial-claim branch (`pool/src/lib.rs:478-496`) builds the ratio by wrapping asset-decimal values in `Ray::from_raw`, which mixes dimensions. The ratio cancels so the result is numerically correct, but the expression is domain-unsafe and worth a dedicated rule. |

Fix the first three; document the last two.

### 4.2 ARCHITECTURE.md vs. code

- §Withdraw Flow: same `amount = 0` mistake as INVARIANTS.md §A.
- §Controller To Pool Communication: wording suggests Soroban token transfers are executed before `pool.supply`. That is correct for supply; for repay, the controller transfers from caller to pool then calls `pool.repay`, which can also refund overpayment back to caller. The text understates the overpayment refund path.

### 4.3 DEPLOYMENT.md vs. code

Largely accurate. One minor point: step 1 of `setupAllMarkets` is
`createMarket`, which calls `create_liquidity_pool`. After this call the
market is `PendingOracle` (pool created, asset config pending). This
matches both the code and the doc.

---

## 5. Math Surface Latent Concerns

These are not bugs. They are sharp edges that the Certora suite should
have rules for.

### 5.1 Asymmetric isolation-debt dust rule

`controller/src/utils.rs:61-92` (decrement) applies a sub-$1-USD → 0
erasure. `controller/src/positions/borrow.rs:204-242` (increment) applies
no symmetric rule. Borrow-and-full-repay cycles can therefore shift the
tracker downward by the sub-$1 residual per cycle.

Property a rule should prove:

> For any isolation-enabled account, if `adjust_isolated_debt_usd` is called
> with `amount_wad >= current`, the tracker becomes 0; otherwise it becomes
> `current - amount_wad`, with the additional `< WAD → 0` clamp. The
> clamp never produces a negative tracker.

### 5.2 Directional drift of `Bps::apply_to_wad`

`common/src/fp.rs:194` rounds twice (BPS→WAD, then WAD×WAD). Used inside
LTV and HF summing loops in `controller/src/helpers/mod.rs:45-180`. Over N
positions each half-up can accumulate by up to `N/2` raw units.

A rule should bound the sum error. Easy to state:

> For a weighted sum over N positions where each term is
> `apply_to_wad(value_i, bps_i)`, the total error is at most
> `N / 2` raw units below the exact rational sum.

Given MAX positions = 10, the drift is negligible, but the bound should be
formal.

### 5.3 Truncating integer divisions in oracle and reporting

- `oracle/mod.rs:132`: `(agg + safe) / 2` truncates.
- `oracle/mod.rs:252, 306`: TWAP `sum / count` truncates.
- `pool/src/interest.rs:117`: bad-debt-ratio-bps event computation
  truncates.

All three are "display or anchor" values, not accounting. Document that
they are intentionally truncating-toward-zero.

### 5.4 `Ray::from_raw(asset_decimal_value)` in claim-revenue partial path

`pool/src/lib.rs:478-496`. `amount_to_transfer` and `treasury_actual` are
asset-decimal i128s; wrapping them in `Ray::from_raw` and dividing
produces a Ray whose magnitude is dimensionally correct (the decimals
cancel) but whose raw representation can be much smaller than a typical
Ray. For tiny revenues (sub-1-unit asset) the ratio loses precision.

Property a rule should prove:

> For any call to `claim_revenue` with `revenue_scaled > 0`, either
> `amount_to_transfer = 0` (no burn) or
> `actual_revenue_burn * supply_index ≈ amount_to_transfer * RAY`
> within 1 raw unit of rounding tolerance.

### 5.5 Missing floor guard on `add_protocol_revenue` (asset path)

`pool/src/interest.rs:49-59`. Unlike `add_protocol_revenue_ray` (lines
63-75), the asset-decimal path does not short-circuit when
`supply_index < SUPPLY_INDEX_FLOOR_RAW`. Reachable only after severe bad
debt socialization, but the asymmetry is a latent footgun.

**Remediation:** mirror the `supply_index.raw() < SUPPLY_INDEX_FLOOR_RAW`
guard into the asset-decimal path.

### 5.6 `Ray::Add` / `Ray::Sub` lack overflow checks

`common/src/fp.rs` uses raw `i128 + i128` on `Ray`, `Wad`, `Bps`. Safe in
practice because every caller narrows via `mul_div_half_up`
(I256 → i128 checked) before adding. Any new caller that sums two bounded
`Ray` values directly risks silent wrap.

A design decision either way; if we want belt-and-suspenders, wrap `Add`
and `Sub` in `checked_add` / `checked_sub` with `MathOverflow` panics.

### 5.7 `calculate_linear_bonus_with_target` domain label

`controller/src/helpers/mod.rs:186-207`. Wraps a BPS range inside
`Wad::from_raw` and multiplies by a WAD scale. Numerically safe given
`scale ≤ 1.0 WAD`, but the domain label is misleading. Either rename the
local to indicate it is raw-weighted BPS, or rescale explicitly.

### 5.8 Division-by-zero audit (all guarded today)

| Division | File:line | Guard |
|---|---|---|
| `mul_div_half_up(..., 0)` | `fp_core.rs:13-20` | Callers always pass RAY/WAD/BPS/validated denominators |
| `utilization → div(supplied)` | `rates.rs:146-155` | Zero-check on `supplied` at 147 |
| `apply_bad_debt → div(total_value)` | `interest.rs:88-136` | Early return on `total_value == 0` at 91 |
| `update_supply_index → div(total_supplied_value)` | `rates.rs:111-125` | Early return on `supplied == 0` at 117 |
| `calculate_borrow_rate` regions | `rates.rs:7-42` | `mid > 0`, `optimal > mid`, `optimal < RAY` enforced in `pool::update_params:529-554` |
| `calculate_health_factor → div(total_borrow)` | `helpers/mod.rs:71-118` | `total_borrow == WAD::ZERO → i128::MAX` at 113 |
| `get_account_bonus_params → div(total_collateral)` | `helpers/mod.rs:363-399` | Empty-supply fallback |
| `is_within_anchor → div(aggregator)` | `oracle/mod.rs:337-352` | Zero-aggregator returns `false` |
| `claim_revenue → div(treasury_actual)` | `pool/src/lib.rs:478-496` | Outer guard `if amount_to_transfer > 0` at 469 |

No unguarded division found. Add a regression rule that enumerates these
sites and asserts the guard fires first.

---

## 6. Remediation Plan

### 6.1 Immediate (before next audit)

1. **Run `certoraSorobanProver controller/confs/math.conf`** with the
   vendored stack. The compile blocker SPIKES.md identified is fixed
   (§3.1.2); one empirical run determines whether remaining prover-side
   issues exist. All verification value below is gated on this.
2. Replace `CompatAssetConfig.debt_ceiling_usd_wad` with
   `isolation_debt_ceiling_usd_wad` and update the referencing rule
   (`isolation_rules.rs:141`). **Done — §0.**
3. Delete `controller/certora/spec/summaries/` (or convert to a README
   pointing at `apply_summary!`). Add `apply_summary!` wrappers at
   call sites for pool, Reflector, and SAC methods per §3.1.1.
4. Delete dead `model.rs` ghost vars or wire them into
   `supply_cannot_decrease_hf`-style before/after snapshots.
5. Fix documentation drift in INVARIANTS.md §6 (8 Taylor terms), §A
   flow (`amount ≥ current_supply`), §7 floor magnitude (`10^18` raw).
   **Done — §0.**

### 6.2 Strengthen existing rules

For each tautological rule in §3.2, rewrite to call the actual prod
function and assert a property of its output. Priority order:

1. `liquidation_rules::seizure_proportional` and
   `protocol_fee_on_bonus_only` — they touch money.
2. `oracle_rules::first_tolerance_uses_safe_price`,
   `second_tolerance_uses_average`, `beyond_tolerance_blocks_risk_ops`,
   `tolerance_bounds_valid` — they touch price under attack.
3. `boundary_rules::liquidation_at_hf_exactly_one` and
   `just_below_one` — HF boundary behavior.
4. `boundary_rules::bad_debt_at_*`, `borrow_exact_reserves`,
   `withdraw_more_than_position`.

### 6.3 Backfill missing invariant coverage

New rules needed for each row marked "not covered" in §3.7:

- `utilization_zero_when_supplied_zero` — §8.
- `ltv_borrow_bound_enforced` — §10.
- `isolation_debt_never_negative`, `isolation_debt_dust_cleared` — §11.
- `claim_revenue_bounded_by_reserves` — §12.
- `reserve_availability_{withdraw,borrow,flash_loan}` — §13.
- `oracle_decimals_discovered_on_configure` — §14.
- `account_meta_consistent_with_positions` — §16.

Add floor guard and bad-debt exception to §7 coverage:

- `supply_index_floor_after_bad_debt`.
- `supply_index_only_decreases_via_bad_debt`.

### 6.4 Math hardening (lower priority)

- Fix the asymmetric isolation dust rule (§5.1) or document the intent.
- Mirror the `SUPPLY_INDEX_FLOOR_RAW` guard into
  `add_protocol_revenue` (§5.5).
- Rewrite `calculate_linear_bonus_with_target` without the
  `Wad::from_raw(bps)` shortcut (§5.7).
- Consider `checked_add`/`checked_sub` on `Ray`/`Wad`/`Bps` (§5.6).

---

## 7. Appendix A — Rule Classification Summary

Totals below include sanity satisfies.

| File | Strong | Weak | Tautological | Vacuous | Sanity | Deleted |
|---|---|---|---|---|---|---|
| `solvency_rules.rs` | 14 | 3 | 0 | 0 | 6 | 0 |
| `oracle_rules.rs` | 2 | 1 | 4 | 0 | 2 | 0 |
| `boundary_rules.rs` | 10 | 0 | 9 | 0 | 2 | 0 |
| `strategy_rules.rs` | 14 | 1 | 0 | 0 | 4 | 1 |
| `position_rules.rs` | 4 | 0 | 0 | 1 | 1 | 0 |
| `health_rules.rs` | 3 | 0 | 0 | 1 | 2 | 0 |
| `isolation_rules.rs` | 6 | 1 | 0 | 0 | 2 | 0 |
| `math_rules.rs` | 12 | 0 | 0 | 0 | 3 | 0 |
| `index_rules.rs` | 5 | 0 | 0 | 0 | 1 | 0 |
| `flash_loan_rules.rs` | 3 | 0 | 0 | 0 | 1 | 1 |
| `interest_rules.rs` | 13 | 1 | 0 | 0 | 1 | 0 |
| `liquidation_rules.rs` | 4 | 2 | 3 | 1 | 2 | 3 |
| `emode_rules.rs` | 12 | 0 | 0 | 1 | 2 | 0 |
| **Total** | **102** | **9** | **16** | **4** | **29** | **5** |

---

## 8. Appendix B — Key Constants

| Name | Value | Location | Meaning |
|---|---|---|---|
| `BPS` | `10_000` | `common/src/constants.rs` | Basis-points base |
| `WAD` | `10^18` | `common/src/constants.rs` | USD fixed-point base |
| `RAY` | `10^27` | `common/src/constants.rs` | Index fixed-point base |
| `MILLISECONDS_PER_YEAR` | `31_556_926_000` | `common/src/constants.rs` | Per-ms rate conversion |
| `SUPPLY_INDEX_FLOOR_RAW` | `10^18` | `pool/src/interest.rs:14` | Floor for supply index post-bad-debt |
| `MAX_LIQUIDATION_BONUS` | `1_500` bps (15%) | `common/src/constants.rs` | Cap on linear bonus |
| `THRESHOLD_UPDATE_MIN_HF` | `1.05 WAD` | `common/src/constants.rs` | Min HF after risky config edits |
| Liquidation target primary | `1.02 WAD` | `controller/src/helpers/mod.rs:220-303` | Preferred post-liq HF |
| Liquidation target fallback | `1.01 WAD` | same | Fallback target |
| Bad debt collateral ceiling | `5 * WAD` | `controller/src/positions/liquidation.rs:413` | Bad debt trigger gate |
| Isolation dust floor | `1 * WAD` | `controller/src/utils.rs:86-88` | Debt-tracker erasure threshold |

---

## 9. Related Documents

- [INVARIANTS.md](./INVARIANTS.md) — algebraic invariants and worked examples.
- [ARCHITECTURE.md](./ARCHITECTURE.md) — component boundaries and flows.
- [DEPLOYMENT.md](./DEPLOYMENT.md) — operator runbook.
- [README.md](./README.md) — top-level map.
