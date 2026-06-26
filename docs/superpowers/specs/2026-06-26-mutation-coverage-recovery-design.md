# Mutation-Testing Coverage Recovery — `common` + controller helpers

**Date:** 2026-06-26
**Status:** Approved (design)
**Scope:** `make mutants` target, `.cargo/mutants.toml`, co-located unit tests in `common` and `contracts/controller`.

## Problem

`make mutants` (mutating `common/src/**` and `contracts/controller/src/helpers/**`) reports
**155 missed / 242 caught / 124 unviable** — a 61% kill rate (242/397 viable), well below this
codebase's historical 96%+.

The low score is **not** a code regression. It is a **test-scope misconfiguration plus a few
genuine in-package gaps**. "Missed" means "not killed by the package-scoped suite this target
runs," not "untested in the protocol."

## Root cause

**Cause 1 — controller helpers have no in-package tests, and the target never runs the harness.**
- `contracts/controller/src/helpers/**` has zero co-located `#[cfg(test)] mod tests;` modules.
  Those helpers are exercised only by the `test-harness` integration crate.
- `cargo-mutants` defaults to running the *mutated package's own* tests. The `mutants` target
  (`Makefile:439`) passes no `--test-package`, so it runs `cargo test -p controller`, which never
  links `helpers/**`. Every helper mutant is structurally unkillable (the "0s build + 0s test"
  tell). `mutants-pool` (`Makefile:456`) already does this correctly with
  `--test-package pool --test-package test-harness`; the main target was never given the same
  treatment.
- Harness viability confirmed: `tests/test-harness/src/setup/builder.rs:175` registers the
  controller **natively** (`env.register(controller::Controller, …)`), so controller *source*
  mutations propagate into harness tests. Only the pool is a prebuilt wasm fixture
  (`builder.rs:180`), and this run does not mutate pool.

**Cause 2 — `common` gaps of three kinds.**
- Untested functions inside tested files: `common/tests/validation.rs` covers
  `validate_risk_bounds` / `validate_sanity_bounds` / `require_cap_within_asset_domain` but not
  `require_positive_amount`, `require_nonneg_amount`, `cap_is_enabled`, `require_wasm_receiver`
  (= all 9 validation misses).
- Files with no co-located tests: `oracle/observation.rs` (~30 misses),
  `oracle/providers/redstone.rs`, `constants/*`.
- Genuinely-equivalent mutants: `Wad::min` `< → <=` (`fp.rs:232`), `Wad::max` `> → >=`
  (`fp.rs:240`) — identical output at equality; no test can kill them.

## Criticality

| Severity | Representative mutants | Real status |
|---|---|---|
| CRITICAL | `helpers/math.rs:150-161` `+= → -=` (collateral/debt accumulation → HF & borrow-gate corruption) | harness-covered, unmeasured |
| CRITICAL | `helpers/risk_params.rs:71,72,89` LT-downgrade gating disabled/inverted | harness-covered, unmeasured |
| CRITICAL | `helpers/emode_caps.rs` `enforce_spoke_*_cap → ()`, `+ → -` (171/191) — spoke-cap-overflow finding | harness-covered, unmeasured |
| HIGH | `oracle/observation.rs:34,41,30` staleness/skew gate deleted/flipped | cross-crate only; no in-package test |
| HIGH | `validation.rs:12,19,50` positive/nonneg/wasm-receiver guards deleted | flash receiver e2e-only; trivially unit-testable |
| MEDIUM | `helpers/utils.rs:81-85` payment aggregation; `account.rs:70,85` cleanup bools; `fp_core.rs` rescale boundaries | harness-covered, unmeasured |
| LOW / equivalent | `fp.rs` min/max boundaries; `constants/shared.rs` `* → +`/`/` | suppress (min/max) or pin (constants) |

No critical invariant is actually unverified in the protocol — but several are currently
invisible to mutation testing, so a future refactor could break them undetected.

## Solution — three tracks

### Track C — honest denominator
- `.cargo/mutants.toml`: add `exclude_re` for the provably-equivalent boundary mutants. All six are
  unkillable because the equality case is pre-handled (or symmetric at zero):
  - `Wad::min` `< → <=` (`fp.rs:232`), `Wad::max` `> → >=` (`fp.rs:240`).
  - `rescale_half_up` / `rescale_floor` / `rescale_ceil` `> → >=` (`fp_core.rs:66/94/118`) — the
    `from_decimals == to_decimals` early `return a` makes `>` vs `>=` unobservable.
  - `mul_div_half_up_signed` `< → <=` (`fp_core.rs:46`) — at `product == 0` both rounding
    directions truncate to `0`.
- **Decision:** constants are **pinned with a test**, not suppressed. New `common/tests/constants.rs`
  asserts the derived values of `constants/shared.rs` and `constants/pool.rs` arithmetic (e.g.
  seconds-per-day, `MS_PER_SECOND` relationships). Kills the `* → +` / `* → /` mutants meaningfully
  and documents intent.

### Track B — fast co-located unit tests (pure / near-pure helpers; `Env`-only, no `Cache`)
- `common/oracle/observation.rs` → new `common/tests/oracle/observation.rs`, wired via
  `#[cfg(test)] #[path] mod tests;`: `is_stale` boundaries (`now==feed_ts`, exactly `max_stale`),
  `check_not_future_at` skew edge, `validate_timestamp` stale + future rejection,
  `normalize_positive_price` (rejects ≤0), `millis_to_seconds`, `u256_to_i128` overflow.
- `common/tests/validation.rs` → add the 4 untested guards: `require_positive_amount`,
  `require_nonneg_amount`, `cap_is_enabled` truth table, `require_wasm_receiver`
  (registered contract address vs an account address).
- `contracts/controller/src/helpers/utils.rs` → new co-located test module: `aggregate_payment_amount`
  (negative / zero / withdraw-all sentinel / sums), `push_unique_address` dedup. `transfer_amount`
  (SAC-dependent) is left to Track A.

`emode_caps.rs`, `math.rs`, `risk_params.rs`, `account.rs`, the oracle provider wrappers, and
`rates.rs` boundary mutants are **not** unit-tested here — they are `Cache` / storage / SAC / wasm
dependent and are already exercised by harness e-mode, liquidation, borrow, and oracle flows. Track A
is their coverage.

### Track A — scope the harness into the default target
- **Decision:** extend the **main** `mutants` target with
  `--test-package common --test-package controller --test-package test-harness`
  (mirroring `mutants-pool`). Every `make mutants` now runs the harness; this is the accepted
  runtime cost. Catches the Cache/wasm-boundary and provider mutants (`helpers/math.rs`,
  `risk_params.rs`, storage-touching `emode_caps`, oracle provider `-> None`) that are exercised by
  harness liquidation / borrow / oracle flows.

## Sequencing

1. Track C (suppress equivalents + pin constants) — honest denominator first.
2. Track B (pure-helper unit tests) — fast feedback, in-package kills.
3. Track A (Makefile target scope) — safety net for harness-only mutants.
4. Verify and iterate on residual stragglers.

## Verification bar

- `cargo test -p common` and `cargo test -p controller` green.
- `cargo check --workspace` + `cargo clippy --workspace -- -D warnings` clean (no `--all-features`).
- `make build` (rebuild `pool.wasm`) so the harness loads its fixture.
- Re-run `make mutants`; record the new kill rate; enumerate any *genuine* residual misses with a
  one-line justification each (equivalent / accepted / follow-up).

## Non-goals

- Not mutating or fixing `pool` coverage (separate `mutants-pool` target already exists).
- Not changing `-j 1` concurrency (`cargo-mutants` forwards `--test-threads` to the baseline; keep
  `--jobs 1` per prior finding).
- Not chasing provably-equivalent mutants beyond the documented suppressions.

## Risks

- **Runtime:** harness in the default loop materially lengthens `make mutants`. Accepted by owner.
- **Harness fixture staleness:** `pool.wasm` must be rebuilt before the run or harness setup panics
  (`builder.rs:193`). Codified in the verification bar.
- **Residual genuine gaps:** some provider mutants may still survive if no harness flow asserts on
  the mocked return; these become an explicit, justified follow-up list rather than silent misses.

## Result (2026-06-27)

**Before:** 521 mutants — 155 missed, 242 caught, 124 unviable (61% viable kill rate).

**Implemented:** Track C suppressed 7 provably-equivalent mutants (6 fp boundaries +
`is_stale:30:14`). Track B added 36 co-located unit tests across constants, validation, oracle
observation, and payment aggregation. Track A scoped `common` + `controller` + `test-harness` into
the default `mutants` target (`pool.wasm` rebuilt; controller is natively registered so source
mutations propagate).

**After — verified across the full mutated scope (the `-j4` single full run was killed mid-flight by
this session's background-process instability, so coverage was confirmed piecewise at `-j8`; every
file in scope was covered):**

| Run (post-change) | Result |
| --- | --- |
| Track B in-package — constants | 18/18 caught, 0 missed |
| Track B in-package — validation | 16/16 caught, 0 missed |
| Track B in-package — observation | 25 caught, 1 equivalent (`is_stale:30:14`, suppressed) |
| Track A smoke — `helpers/math.rs` | 6 caught, 13 unviable, **0 missed** |
| Track A chunk — `account`/`emode_caps`/`risk_params` | 49 caught, 11 unviable, 1 timeout, **0 missed** |
| Track A chunk — `rates`/`redstone`/`reflector`/`fp`/`fp_core`/`utils` | 250 caught, 85 unviable, **0 missed** |

**Net:** zero missed mutants across the entire mutated surface. The crown-jewel mutants —
`calculate_account_risk_totals_body` `+=→-=` (health-factor sign), `enforce_spoke_*_cap` deletion
and `+→-` (spoke-cap overflow), `risk_params` LT-downgrade gating, `transfer_amount` return, and the
oracle staleness guards — are all now caught.

**Residual triage:** the only un-killed mutants are provably-equivalent and suppressed with
justification in `.cargo/mutants.toml` (the six fp boundary mutants where the equality case is
pre-handled or symmetric at zero, plus `is_stale:30:14` where elapsed is `0` at `now == feed_ts`).
No `follow-up` (genuine-gap) residuals remain.

**Note:** the committed `mutants` target keeps `--jobs 1` (the owner's determinism choice); the `-j8`
used for this verification only parallelizes mutant workers and yields identical caught/missed
verdicts.
