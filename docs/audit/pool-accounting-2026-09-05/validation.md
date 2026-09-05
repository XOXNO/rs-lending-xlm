# Validation record

Revision: `99613335b410f70ff42dd99d13ff530f6adaee67`. Commands run locally; no network or deployment tests. Host tests mock authorization where appropriate and therefore do not establish ordinary-user controller reachability by themselves.

## Counts

| Check | Result | Evidence |
|---|---|---|
| Pool unit suite | 168 passed | Independent cash review execution record |
| Common math suite | 149 passed | `evidence/math/common-math-tests.log`; isolated repeat in `isolated-common-math-tests.log` |
| Common rates suite | 76 passed | Independent rates review execution record |
| Retained composed lifecycle and fee tests | 3 passed | `evidence/money-flow.log` |
| Existing large-value controller regression | 1 passed | `evidence/value-ceiling.log` |
| Independent actual-source arithmetic checks | 83,792 assertions passed | `evidence/math/probe.log`; retained-runner repeat also passed |
| Independent Decimal/rational equation replay | 105 conversion combinations; precision discrepancies quantified | `evidence/rates-reference.log` |

Total distinct Rust tests: **397**. Separately run pool-interest tests (22) and existing floor regression (1) are included in the 168. The standalone fee probe is the same case retained in the three audit tests; its original one-test result is not counted twice. Repeated math checks are likewise counted once. The existing common rates suite contains an additional 120,000-case randomized conservation sweep inside its 76 tests.

## Retained audit tests

Run from the repository root with an isolated target. An initial build needs dependencies already cached for offline mode.

```sh
CARGO_TARGET_DIR=/private/tmp/astra-pool-isolated-target RUSTC_WRAPPER= cargo test --offline --locked -p test-harness --test pool_money_flow_audit -- --nocapture

CARGO_TARGET_DIR=/private/tmp/astra-pool-isolated-target RUSTC_WRAPPER= cargo test --offline --locked -p test-harness --test controller a_whale_market_at_sustained_high_utilization_hits_the_ray_value_ceiling_before_the_index_cap -- --nocapture
```

The retained test file is [pool_money_flow_audit.rs](/Users/mihaieremia/GitHub/rs-lending-xlm/tests/test-harness/tests/pool_money_flow_audit.rs). It contains:

1. All successful pool money paths, shared-token cash across two hubs, unbooked donation, distinct payer/receiver refund checks, explicit net-settle and liquidation share deltas, and revenue custody reconciliation.
2. Zero-seeded-liquidity origination, cash withdrawal, complete debt seizure, index floor, partial recap, excess refund, and final payout. Maximum utilization is deliberately RAY. This is a valid pool-layer construction, not proof of controller liquidation eligibility.
3. The independent reviewer's real 7-decimal SAC fee-headroom case, using public pool methods without storage injection.

The controller regression uses raised caps, disabled utilization restriction, one billion tokens and initial 98% utilization. It checks the fourth-year overflow, the index being below its cap, and subsequent repayment/withdrawal failure. It demonstrates a documented local liveness limit; it does not establish current mainnet exposure.

## Arithmetic runner

The runner imports actual production math/constants/errors by path, generates 20,840 Python bigint reference vectors, and compiles the Rust probe with overflow checks enabled and debug assertions disabled. To avoid leaving a generated TSV and binary in the repository, run a scratch copy:

```sh
mkdir -p /private/tmp/astra-fixed-point-retained-check
cp docs/audit/pool-accounting-2026-09-05/evidence/math/{run.py,generate.py,probe.rs} /private/tmp/astra-fixed-point-retained-check/
CARGO_TARGET_DIR=/private/tmp/astra-pool-isolated-target python3 /private/tmp/astra-fixed-point-retained-check/run.py /Users/mihaieremia/GitHub/rs-lending-xlm

python3 docs/audit/pool-accounting-2026-09-05/evidence/rates-reference.py
```

The retained runner was updated only to honor `CARGO_TARGET_DIR` for SDK artifact discovery and use `--locked`; that exact retained version was rerun successfully. Original commands/logs remain alongside `retained-runner-check.log`. The numeric probe and generator are the independent reviewer's originals; no generated 4 MB TSV or compiled binary is retained.

The probe verifies 72,378 multiply/divide results; 684 conversions over decimals 0–18; all 10,001 Bps ratios from 0 to 10,000; and 729 rescale cases. Two signed extreme underflows and the separately documented positive overflow are reproduced as expected panics. The separate rates Python script replays equations; it is not another execution of contract source.

## Existing suite commands

```sh
CARGO_TARGET_DIR=/private/tmp/astra-pool-isolated-target RUSTC_WRAPPER= cargo test --offline --locked -p pool --lib
CARGO_TARGET_DIR=/private/tmp/astra-pool-isolated-target RUSTC_WRAPPER= cargo test --offline --locked -p common math::
CARGO_TARGET_DIR=/private/tmp/astra-pool-isolated-target RUSTC_WRAPPER= cargo test --offline --locked -p common rates::
```

The initial reviewer runs used the default workspace target with `RUSTC_WRAPPER=`. The commands above recommend the isolated target that resolved later integration build failures. Detailed original commands and timing are preserved in the reviewer reports. Do not interpret a recommended command as an additional completed run.

## Build and fixture corrections

The initial workspace integration build failed with E0277 `TryFromVal` / `FromVal` errors and conflicting Soroban crate identities. This followed a standalone scratch-project build into the workspace target; artifact interference is the suspected cause, not a proven compiler diagnosis. Zero target integration tests executed in those failed attempts. `evidence/build-failure-excerpt.log` preserves the diagnostic; the full temporary logs were about 775 KB each. No source or dependency repair was made. A separate target directory compiled and ran the same workspace successfully.

The first isolated lifecycle run then failed because the harness preset defaults to one million tokens of injected initial cash. The audit fixture now explicitly sets initial liquidity to zero. The final passing runs reconcile only cash introduced through actual test token transfers; initial market economic state is not fabricated.

## Evidence provenance

`reviews/cash.md`, `reviews/rates.md`, and `reviews/math.md` preserve the independent reports. Their temporary paths and initially blocked integration notes describe the original work, not outstanding coordinator blockers. The pool fee reproduction was subsequently consolidated into the retained integration test; prefer that test command over the original standalone scratch manifest command.

`evidence/fee-headroom-original.log` records the original fee test. `pool-interest.log` and `floor-existing-test.log` record overlapping targeted tests, not extra unique coverage. Successful source checks, report reconciliation and final scope hashes belong to this pool-only audit; earlier broad-audit results are not reused in the 397-test count.

Validation limitations: native Soroban host execution, not compiled-WASM resource-limit testing or live mainnet transactions. No exhaustive branch coverage, formal proof, arbitrary hostile-token listing proof, or full controller re-audit is claimed. Production code, dependencies, prior audit artifacts, and deployment state were left unchanged.
