# Fuzzing

`cargo-fuzz` targets in their own Cargo workspace (excluded from the root). They reach inputs unit tests miss: fixed-point rounding, index accrual, liquidation edges, multi-step flows, strategy routing, accounting conservation. Proptest properties live in the [harness](../test-harness/tests/fuzz/README.md).

## What to run

From the repository root:

```bash
make fuzz-build                         # compile libFuzzer targets
make fuzz FUZZ_TIME=60                  # function-level math targets, 60 s each
make fuzz-contract FUZZ_TIME=60         # contract-level targets, 60 s each
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

On Linux, the Makefile builds without a sanitizer (`FUZZ_SANITIZER=none`). The fuzzed crates have no `unsafe` code, and panics, debug assertions and overflow checks fire without one. ASan made `flow_e2e` about 5× slower. Set `FUZZ_SANITIZER=address` to check the `unsafe` code in the dependencies; the weekly CI campaign does. The build also leaves out comparison tracing (`FUZZ_TRACE_COMPARES=no`) and instruments only the workspace crates for coverage (`FUZZ_COVERAGE_SCOPE=workspace`, through `scripts/fuzz_rustc_wrapper.sh`). In 90 s on the same seeds, these settings gave 1.7-3.3x and then another 1.1-4.5x more runs at equal coverage. Set `FUZZ_TRACE_COMPARES=yes FUZZ_COVERAGE_SCOPE=all` for a long, deep campaign. `make fuzz` and `make fuzz-contract` run `FUZZ_JOBS` targets at a time (default 1), each for its full `FUZZ_TIME`. `make fuzz-wasm` builds the three WASM modules that the targets and the proptest suite load.

On macOS, the Makefile passes `--sanitizer=thread -Zbuild-std` to `cargo-fuzz`. A direct run needs the same flags, plus `--fuzz-dir .` from `tests/fuzz`:

```bash
rustup install nightly
rustup component add rust-src --toolchain nightly
cargo install cargo-fuzz
cd tests/fuzz
cargo +nightly fuzz run --fuzz-dir . flow_e2e --sanitizer=thread -Zbuild-std -- -max_total_time=60
```

## Test Layers

libFuzzer targets live in `tests/fuzz/fuzz_targets/<target>.rs` ([Targets](#targets)). Property tests live in `tests/test-harness/tests/fuzz/` and run with `make proptest`: accounting conservation, strategy invariants, budget metering, `migrate_from_blend` reconciliation, liquidation differentials vs reference, deterministic auth matrices ([property list](../test-harness/tests/fuzz/README.md#properties)). Miri (`common/tests/math/fp_core.rs`) checks the pure i128 helpers `rescale_half_up`, `rescale_floor`, `rescale_ceil` and `div_by_int_half_up` for undefined behavior.

## Targets

`make fuzz` runs the fast, cheap pure-math targets:

| Target | Scope |
|---|---|
| `fp_math` | `mul_div_half_up`, `div_by_int_half_up`, `rescale_half_up`; rounding, overflow |
| `rates_and_index` | Borrow and deposit rates, compound interest, index accrual and transitions (scaled balances, supply-index floor on bad debt), supplier rewards, protocol fee split |
| `fp_ops` | `Ray`, `Wad`, `Bps` operations; directed rounding of the `mul_div_*` primitives; exact `BigRational` checks of `position_value*`, `resolve_repay`, `resolve_withdrawal`, `resolve_net_settle` |
| `oracle_math` | Constant-product and StableSwap LP fair pricing against independent bounds (leg-order symmetry, book value, D bounds, amp monotonicity); `isqrt_of_product` |

`make fuzz-contract` runs:

| Target | Scope |
|---|---|
| `flow_e2e` | Fixed-width byte op streams: multi-asset supply, borrow, withdraw, repay, liquidation, flash-loan failure/success, oracle jitter, index sync, revenue claim, bad-debt cleanup, rollback |
| `flow_strategy` | `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, index sync; controller holds no token balance after a strategy op |
| `pool_native` | Native pool supply, borrow, withdraw, repay, index and parameter updates, revenue claim, seizure, strategy creation, flash loan (needs `flash_loan_receiver.wasm` from `make build`), net settle, recapitalize and views, indexes optionally pinned near caps, against reserve invariants (tracked cash ≤ token balance, revenue ≤ supplied) |
| `aggregator` | Arbitrary prices, ages, tolerances and sanity bands on single and dual sources: per-feed staleness ceiling, tolerance disagreement, sanity band |
| `oracle_sources` | Every provider (Reflector spot and TWAP, RedStone, Xoxno, Chainlink) and source shape (feed, scaled, Aquarius constant-product and stable LP, dual blends) with injected faults, via `set_oracle` and `seed_oracle` |
| `xoxno_oracle` | XOXNO oracle vs a reference model of signers, config, submissions, aggregates and history |

## Corpus And Regressions

Each target starts from committed inputs in `tests/fuzz/seeds/<target>/` that already decode to useful lengths, so short CI campaigns skip growing inputs. New corpus, crash artifacts and coverage data go to git-ignored:

```text
tests/fuzz/corpus/
tests/fuzz/artifacts/
tests/fuzz/coverage/
```

For long campaigns, `make fuzz-seed-corpus` adds `tests/fuzz/corpus/` inputs for `fp_math`, `fp_ops`, `rates_and_index`, flow and `pool_native`. Non-flow inputs come from the `test_snapshots/` JSON the test suites write, count-capped for fast replay. The contract unit tests write that JSON by default. The `test-harness` suite writes it only with `HARNESS_TEST_SNAPSHOTS=1`, so set that variable on the test run before you seed from harness snapshots. `rates_and_index` falls back to a fixed parameter set when no snapshot has decodable market parameters. Flow seeds are fixed, valid 5-byte operation streams, not `Arbitrary<Vec<_>>` prefixes.

Minimize a libFuzzer crash before you keep it as evidence (on macOS, add `--sanitizer=thread -Zbuild-std`):

```bash
cd tests/fuzz
cargo +nightly fuzz tmin --fuzz-dir . <target> artifacts/<target>/crash-<hash>
```

Proptest regressions: see the [harness fuzz README](../test-harness/tests/fuzz/README.md).

## Coverage

Coverage replays corpus and seeds through instrumented targets, fuzzing first only when `FUZZ_COV_TIME` (seconds) > 0.

```bash
make fuzz-coverage
make fuzz-coverage FUZZ_COV_TIME=30
make fuzz-coverage-all
make fuzz-coverage-one TARGET=flow_e2e FUZZ_COV_TIME=60
```

Reports: `target/coverage/fuzz/<target>/index.html`, showing `common/` and contract code (fuzz/harness code, dependencies and std excluded). Flow targets run the pool as uploaded WASM, so native pool coverage comes only from `pool_native`.

## CI

`.github/workflows/fuzz.yml` runs:

- PR smoke: `make fuzz FUZZ_TIME=30 FUZZ_JOBS=4`,
  `make fuzz-contract FUZZ_TIME=60 FUZZ_JOBS=6`, `make proptest PROPTEST_THREADS=4`,
  `make miri-all`, and a `make mutants-diff` shard matrix.
- Daily and manual dispatch: `fuzz-long` (every libFuzzer target),
  `proptest-long` (per-property matrix), `mutants` (per-area mutation testing).

The `fuzz-build` job builds every target once, and each `fuzz-long` cell runs
that binary through `make fuzz-one FUZZ_BIN_DIR=<dir>`. The Sunday campaign
builds with ASan, comparison tracing and coverage of every crate; the other runs
use the fast defaults. Each cell restores
the corpus of the last run of its target and saves it again, so a campaign
continues the coverage of the last one.

On Monday to Saturday nights, each `mutants` cell resumes from the state of its
last run (`MUTANTS_RESUME`). It tests only the mutants that run did not catch
or find unviable. A changed line gets new mutant names and runs again. The
Sunday run and every dispatch test the whole scope.

On failure, `fuzz-long` uploads `tests/fuzz/artifacts/<target>/` and
`proptest-long` uploads the proptest regression files.
