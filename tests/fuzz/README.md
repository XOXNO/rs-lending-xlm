# Fuzzing

`cargo-fuzz` targets for the protocol. This is test code in its own Cargo
workspace; the root workspace excludes it.

The targets and property tests reach inputs that unit tests miss: fixed-point
rounding, index accrual, liquidation edges, multi-step flows, strategy
routing, auth boundaries, and accounting conservation.

## What to run

Use the Makefile from the repository root for normal workflows:

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

On macOS, the Makefile passes `--sanitizer=thread -Zbuild-std` to
`cargo-fuzz`. A direct run needs the same flags, and `--fuzz-dir .` when it
starts in `tests/fuzz`:

```bash
rustup install nightly
rustup component add rust-src --toolchain nightly
cargo install cargo-fuzz
cd tests/fuzz
cargo +nightly fuzz run --fuzz-dir . flow_e2e --sanitizer=thread -Zbuild-std -- -max_total_time=60
```

## Test Layers

| Layer | Location | Purpose |
|---|---|---|
| Function fuzzing | `tests/fuzz/fuzz_targets/fp_math.rs`, `rates_and_index.rs`, `fp_ops.rs` | Pure math, rounding, overflow, rates, and index transitions (scaled balances, supply-index floor on bad debt). Fast and cheap. |
| Native pool fuzzing | `tests/fuzz/fuzz_targets/pool_native.rs` | Pool supply, borrow, withdraw, repay, index and parameter updates, revenue claim, seizure, strategy creation, and views, against reserve invariants (tracked cash ≤ token balance, revenue ≤ supplied). |
| Oracle fuzzing | `tests/fuzz/fuzz_targets/aggregator.rs` | Price-aggregator resolution against arbitrary prices, ages, tolerances and sanity bands, on both single- and dual-source configurations. |
| Protocol flow fuzzing | `tests/fuzz/fuzz_targets/flow_e2e.rs`, `flow_strategy.rs` | Fixed-width byte op streams for multi-asset user flows, liquidations, bad-debt cleanup, flash-loan failure paths, strategy routes, zero controller residual balance, and rollback behavior. |
| Property tests | `tests/test-harness/tests/fuzz/` | Proptest suites for accounting conservation, strategy invariants, budget metering, `migrate_from_blend` reconciliation, and liquidation differentials vs reference, plus deterministic auth matrices. |
| Miri | `common/tests/math/fp_core.rs` | Undefined-behavior checks for the pure i128 helpers `rescale_half_up`, `rescale_floor`, `rescale_ceil`, and `div_by_int_half_up`. |

## Targets

`make fuzz` runs:

| Target | Scope |
|---|---|
| `fp_math` | `mul_div_half_up`, `div_by_int_half_up`, `rescale_half_up`. |
| `rates_and_index` | Borrow and deposit rates, compound interest, index accrual, supplier rewards, protocol fee split. |
| `fp_ops` | `Ray`, `Wad`, and `Bps` operations, directed rounding of the `mul_div_*` primitives, and exact `BigRational` checks of `position_value*`, `resolve_repay`, `resolve_withdrawal`, and `resolve_net_settle`. |

`make fuzz-contract` runs:

| Target | Scope |
|---|---|
| `flow_e2e` | Supply, borrow, withdraw, repay, liquidation, flash-loan failure/success paths, oracle jitter, index sync, revenue claim, bad-debt cleanup. |
| `flow_strategy` | `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, and index sync. The controller holds no token balance after a strategy op. |
| `pool_native` | Native pool state transitions and view invariants. |
| `aggregator` | Price-aggregator gates on single and dual sources: staleness against the per-feed ceiling, tolerance disagreement, and the sanity band. |

`make proptest` runs:

| Property (`--test fuzz`) | Scope |
|---|---|
| `prop_accounting_conservation` | Pool accounting laws, non-negative reserves, index monotonicity. |
| `prop_seed_adjusted_cash_conservation_and_token_custody` / `prop_seed_adjusted_cash_conservation_through_liquidation_and_bad_debt` | Seed-adjusted cash conservation and token custody, also through liquidation and bad debt. |
| `owner_only_endpoints_reject_unauthed_before_validation` / `governance_endpoints_reject_unauthed_before_validation` | Deterministic privileged endpoint auth matrices. |
| `prop_valid_multiply_fits_default_budget` | Valid `multiply` calls under Soroban default budget limits. |
| `prop_multiply_succeeds_with_safe_hf_and_clean_router` / `prop_swap_collateral_conserves_position_delta` | Strategy success, exact deltas, HF, allowance, and flash-guard cleanup. |
| `prop_migrate_blend_reconciles_same_asset` / `prop_migrate_blend_cap_too_low_reverts` | `migrate_from_blend` reconciliation, and the revert when the cap is below the Blend liability. |
| `prop_liquidation_matches_bigrational_reference` / `prop_below_base_liquidation_matches_reference_and_never_loses_the_liquidator_money` | Liquidation vs `BigRational` reference, including accounts whose HF-preserving cap is below the base bonus. |

## Corpus And Regressions

Each target starts from small committed inputs in `tests/fuzz/seeds/<target>/`:
two for `rates_and_index`, one for each other target. The seeds already decode
to inputs of useful length, so short CI campaigns do not spend their budget
growing inputs. New corpus inputs, crash artifacts, and coverage data go to
these local, git-ignored directories:

```text
tests/fuzz/corpus/
tests/fuzz/artifacts/
tests/fuzz/coverage/
```

For long campaigns, `make fuzz-seed-corpus` adds inputs to
`tests/fuzz/corpus/` for every target except `aggregator`. The `fp_math`,
`fp_ops`, `rates_and_index`, and `pool_native` inputs take their values from the
`test_snapshots/` JSON files that the test suites write. Flow seeds are fixed,
valid 5-byte operation streams, not `Arbitrary<Vec<_>>` prefixes. The generator
caps the number of numeric seeds so corpus replay stays fast. `rates_and_index`
uses a fixed fallback parameter set when no snapshot has decodable market
parameters.

```bash
make fuzz-seed-corpus
```

When libFuzzer finds a crash, minimize it before you keep it as evidence. On
macOS, add `--sanitizer=thread -Zbuild-std`:

```bash
cd tests/fuzz
cargo +nightly fuzz tmin --fuzz-dir . <target> artifacts/<target>/crash-<hash>
```

Proptest stores minimized failures in
`tests/test-harness/tests/fuzz/*.proptest-regressions`. The files are
committed, so each stored case replays on every run. The two seed-adjusted cash
properties do not persist failures.

## Coverage

Fuzz coverage replays the corpus and seeds through instrumented targets. It
fuzzes first only when `FUZZ_COV_TIME` (seconds) is above zero.

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

Coverage excludes fuzz and harness code, dependencies, and standard-library
files, so the report shows `common/` and contract code. Flow targets execute
the pool through uploaded WASM, so native pool source coverage comes from
`pool_native`, not `flow_e2e` or `flow_strategy`.

## CI

`.github/workflows/fuzz.yml` runs:

- PR smoke: `make fuzz FUZZ_TIME=30`, `make fuzz-contract FUZZ_TIME=60`,
  `make proptest`, `make miri-all`, and a `make mutants-diff` shard matrix.
- Daily schedule and manual dispatch: `fuzz-long` runs every libFuzzer target,
  `proptest-long` runs a per-property matrix, and `mutants` runs per-area
  mutation testing.

On failure, `fuzz-long` uploads `tests/fuzz/artifacts/<target>/` and
`proptest-long` uploads the proptest regression files.
