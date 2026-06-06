# Fuzzing

A fuzzer hammers the code with huge numbers of random and adversarial inputs,
looking for any that make it crash, panic, or break an accounting rule — the
edge cases a human writing example tests would never think to try.

This directory contains the protocol's `cargo-fuzz` package. It is kept under
`verification/` because it is audit and assurance infrastructure, not
production contract code.

Fuzzing is used to exercise inputs that unit tests do not enumerate: fixed
point rounding, index accrual, liquidation edge cases, multi-step account
flows, strategy routing, authorization boundaries, TTL behavior, and accounting
conservation.

## What To Run

Use the Makefile from the repository root for normal workflows:

```bash
make fuzz-build                         # compile libFuzzer targets
make fuzz FUZZ_TIME=60                  # fast math/native fuzz targets
make fuzz-contract FUZZ_TIME=60         # full protocol flow fuzz targets
make proptest PROPTEST_CASES=256        # contract-level property tests
make miri-common                        # UB checks for pure fixed-point math
```

For release or audit preparation:

```bash
make fuzz-seed-corpus
make fuzz FUZZ_TIME=3600
make fuzz-contract FUZZ_TIME=3600
make proptest PROPTEST_CASES=10000
make fuzz-coverage-all
```

On macOS, the Makefile automatically passes the sanitizer flags needed by
`cargo-fuzz`. Direct manual runs may require:

```bash
rustup install nightly
rustup component add rust-src --toolchain nightly
cargo install cargo-fuzz
cd verification/fuzz
cargo +nightly fuzz run flow_e2e --sanitizer=thread -Zbuild-std -- -max_total_time=60
```

## Test Layers

| Layer | Location | Purpose |
|---|---|---|
| Function fuzzing | `verification/fuzz/fuzz_targets/fp_math.rs`, `rates_and_index.rs`, `fp_ops.rs` | Pure math, rounding, overflow, rates, and index transitions. Fast and cheap. |
| Native pool fuzzing | `verification/fuzz/fuzz_targets/pool_native.rs` | Pool constructor, index update, rewards, views, and reserve invariants without token transfer setup. |
| Protocol flow fuzzing | `verification/fuzz/fuzz_targets/flow_e2e.rs`, `flow_strategy.rs` | Multi-asset user flows, liquidations, flash-loan failure paths, strategy routes, router allowance cleanup, and rollback behavior. |
| Property tests | `verification/test-harness/tests/fuzz/` | Deterministic proptest suites for accounting conservation, auth, strategy invariants, budget metering, and liquidation differentials. |
| Miri | `common/src/math/fp_core.rs` tests | Undefined-behavior checks for pure i128 fixed-point helpers. |

## Targets

`make fuzz` runs:

| Target | Scope |
|---|---|
| `fp_math` | `mul_div_half_up`, `div_by_int_half_up`, `rescale_half_up`. |
| `rates_and_index` | Borrow rate, compound interest, supplier rewards, protocol fee split. |
| `fp_ops` | Fixed-point wrapper operations and boundary behavior. |

`make fuzz-contract` runs:

| Target | Scope |
|---|---|
| `flow_e2e` | Supply, borrow, withdraw, repay, liquidation, flash-loan failure/success paths, oracle jitter, index sync, revenue claim, bad-debt cleanup. |
| `flow_strategy` | `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, and router allowance cleanup. |
| `pool_native` | Native pool state transitions and view invariants. |

`make proptest` runs:

| Property (`--test fuzz`) | Scope |
|---|---|
| `prop_accounting_conservation` | Pool accounting laws, non-negative reserves, index monotonicity. |
| `prop_owner_only_endpoints_reject_unauthed` / `prop_wrong_role_rejected` | Privileged endpoint auth gates. |
| `prop_strategy_under_budget` | `multiply` under Soroban default budget limits. |
| `prop_multiply_leverage_hf_safe` / `prop_strategy_swap_collateral_balance_delta` | Strategy HF, allowance, payload guards. |
| `prop_short_aggregator_rejected` | Zero-output aggregator rejection. |
| `prop_liquidation_matches_bigrational_reference` | Liquidation vs `BigRational` reference. |

## Corpus And Regressions

The libFuzzer corpus is local and ignored by git:

```text
verification/fuzz/corpus/
verification/fuzz/artifacts/
verification/fuzz/coverage/
```

Seed the corpus from generated ledger snapshots before long campaigns:

```bash
make fuzz-seed-corpus
```

When libFuzzer finds a crash, minimize it before keeping it as evidence:

```bash
cd verification/fuzz
cargo +nightly fuzz tmin <target> artifacts/<target>/crash-<hash>
```

Proptest regressions are different: files under
`verification/test-harness/tests/fuzz/*.proptest-regressions` are committed so
minimized failing cases replay automatically.

## Coverage

Fuzz coverage replays the existing corpus through instrumented targets. It does
not perform active fuzzing unless `FUZZ_COV_TIME` is set.

```bash
make fuzz-coverage
make fuzz-coverage FUZZ_COV_TIME=30
make fuzz-coverage-all
make fuzz-coverage-one TARGET=flow_e2e FUZZ_COV_TIME=60
```

Reports are written to:

```text
target/coverage/fuzz/<target>/index.html
```

Coverage filters out harness code, dependencies, and standard library files so
the report focuses on `common/`, `controller/`, `pool/`, and shared harness
helpers.

## CI

`.github/workflows/fuzz.yml` runs:

- PR smoke: short function fuzzing plus property tests.
- Nightly/manual: longer function fuzzing, protocol flow fuzzing, and property
  tests.
- Miri on the pure fixed-point math subset.

The workflow uploads `verification/fuzz/artifacts/` and proptest regression
files on failure.
