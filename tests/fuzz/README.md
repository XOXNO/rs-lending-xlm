# Fuzzing

`cargo-fuzz` targets for this protocol. Lives under `tests/` as assurance
infrastructure, not production contract code.

Exercise inputs unit tests miss: fixed-point rounding, index accrual,
liquidation edges, multi-step flows, strategy routing, auth boundaries, TTL,
and accounting conservation.

## What to run

Use the Makefile from the repository root for normal workflows:

```bash
make fuzz-build                         # compile libFuzzer targets
make fuzz FUZZ_TIME=60                  # fast math/native fuzz targets
make fuzz-contract FUZZ_TIME=60         # full protocol flow fuzz targets
make proptest                           # tuned per-property defaults
make proptest PROPTEST_CASES=256        # uniform deeper property run
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
cd tests/fuzz
cargo +nightly fuzz run flow_e2e --sanitizer=thread -Zbuild-std -- -max_total_time=60
```

## Test Layers

| Layer | Location | Purpose |
|---|---|---|
| Function fuzzing | `tests/fuzz/fuzz_targets/fp_math.rs`, `rates_and_index.rs`, `fp_ops.rs` | Pure math, rounding, overflow, rates, and index transitions (scaled balances, supply-index floor on bad debt per INVARIANTS/ADR 0007). Fast and cheap. |
| Native pool fuzzing | `tests/fuzz/fuzz_targets/pool_native.rs` | Pool constructor, index update, rewards, views, and reserve invariants (cash excludes donations, revenue ≤ supplied). |
| Protocol flow fuzzing | `tests/fuzz/fuzz_targets/flow_e2e.rs`, `flow_strategy.rs` | Fixed-width byte op streams for multi-asset user flows, liquidations (per-spoke curves, tainted debt), flash-loan failure paths, strategy routes (balance-delta per ADR 0005), router allowance cleanup, and rollback behavior. |
| Property tests | `tests/test-harness/tests/fuzz/` | Deterministic proptest suites for accounting conservation, auth, strategy invariants, budget metering, and liquidation differentials vs reference. |
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
| `owner_only_endpoints_reject_unauthed_before_validation` / `governance_endpoints_reject_unauthed_before_validation` | Deterministic privileged endpoint auth matrices. |
| `prop_valid_multiply_fits_default_budget` | Valid `multiply` calls under Soroban default budget limits. |
| `prop_multiply_succeeds_with_safe_hf_and_clean_router` / `prop_swap_collateral_conserves_position_delta` | Strategy success, exact deltas, HF, allowance, and flash-guard cleanup. |
| `prop_liquidation_matches_bigrational_reference` | Liquidation vs `BigRational` reference. |

## Corpus And Regressions

Each target starts from one small committed, structurally accepted input under
`tests/fuzz/seeds/`. This prevents short CI campaigns from spending their
budget merely growing inputs to the target's minimum useful length. New
coverage inputs are written to the local, ignored corpus:

```text
tests/fuzz/corpus/
tests/fuzz/artifacts/
tests/fuzz/coverage/
```

For long campaigns, enrich the writable corpus from generated ledger
snapshots. Flow seeds are valid 5-byte operation streams, not
`Arbitrary<Vec<_>>` prefixes. Numeric seed generation is intentionally bounded
so corpus replay remains fast, and `rates_and_index` has a production-preset
fallback when snapshots contain no decodable market parameters.

```bash
make fuzz-seed-corpus
```

When libFuzzer finds a crash, minimize it before keeping it as evidence:

```bash
cd tests/fuzz
cargo +nightly fuzz tmin <target> artifacts/<target>/crash-<hash>
```

Proptest regressions are different: files under
`tests/test-harness/tests/fuzz/*.proptest-regressions` are committed so
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
the report focuses on `common/`, `controller/`, and native `pool/` code. Flow
targets execute the pool through uploaded WASM, so native pool source coverage
comes from `pool_native`, not `flow_e2e` or `flow_strategy`.

## CI

`.github/workflows/fuzz.yml` runs:

- PR smoke: short function fuzzing plus property tests.
- Nightly/manual: longer function fuzzing, protocol flow fuzzing, and property
  tests.
- Miri on the pure fixed-point math subset.

The workflow uploads `tests/fuzz/artifacts/` and proptest regression
files on failure.
