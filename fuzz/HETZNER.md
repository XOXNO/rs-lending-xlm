# Running the fuzz campaign on Hetzner

One-command setup for a fresh Hetzner box. Tested on AX41 and CX32 with Ubuntu 22.04+.

## Bootstrap (one-time)

```bash
# 1. System deps
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev git curl clang llvm

# 2. Rust stable + nightly (nightly needed for cargo-fuzz)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.93
source $HOME/.cargo/env
rustup install nightly
rustup target add wasm32-unknown-unknown wasm32v1-none

# 3. Soroban CLI (for pool WASM build)
cargo install --locked stellar-cli

# 4. cargo-fuzz
cargo install --locked cargo-fuzz

# 5. Clone + build baseline
git clone <your repo> rs-lending
cd rs-lending/stellar
make build          # produces target/wasm32v1-none/release/pool.wasm
make test           # sanity check — all 851 tests should pass
```

## Running a campaign

```bash
cd rs-lending/stellar

# --- Pre-campaign: seed corpus from existing test_snapshots (~30s). ---
# Without seeding, libFuzzer burns the first ~10k iterations on trivial
# inputs. Seeding gives it realistic RAY indexes, rate params, and position
# amounts to mutate from iteration 0.
make fuzz-seed-corpus

# --- Recommended full campaign (~5 hours wall time on 6-core) ---
# Function-level: 5 targets × 1 hour each
make fuzz FUZZ_TIME=3600 2>&1 | tee fuzz-funclvl.log &

# Contract-level: 5 tests × 10k cases each (~30-60 min per test)
make proptest PROPTEST_CASES=10000 2>&1 | tee fuzz-contract.log &

wait
echo "Campaign complete. Check logs for failures."
```

## Parallelism tuning

libFuzzer runs single-threaded per target; parallelism comes from running
multiple targets concurrently. Proptest requires `--test-threads=1` because
the `LendingTest` harness mutates global Env state.

To pin targets to specific cores on a multi-core box:

```bash
# Function-level fuzz in parallel (one core per target)
for i in 0 1 2 3 4; do
    target=$(echo "fp_mul_div fp_rescale fp_div_by_int rates_borrow compound_monotonic" | cut -d' ' -f$((i+1)))
    taskset -c $i make fuzz-one TARGET=$target FUZZ_TIME=3600 2>&1 > fuzz-$target.log &
done
wait
```

## Monitoring

```bash
# Watch exec rate for a running libFuzzer target
tail -f fuzz-funclvl.log | grep -E 'exec/s|pulse|DONE'

# Check for crashes mid-run
ls fuzz/artifacts/*/crash-* 2>/dev/null

# Check for proptest regressions
ls test-harness/tests/*.proptest-regressions 2>/dev/null
```

## Triaging findings

### Function-level crash

```bash
cd stellar/fuzz

# 1. Minimize the crashing input
cargo +nightly fuzz tmin fp_mul_div artifacts/fp_mul_div/crash-<hash>

# 2. Pretty-print the structured input
cargo +nightly fuzz fmt fp_mul_div artifacts/fp_mul_div/crash-<hash>

# 3. Reproduce with full backtrace
RUST_BACKTRACE=1 cargo +nightly fuzz run fp_mul_div artifacts/fp_mul_div/crash-<hash>

# 4. Once fixed: commit the crash file under corpus/<target>/ as regression
cp artifacts/fp_mul_div/crash-<hash> corpus/fp_mul_div/regression-<issue-number>
```

### Contract-level regression

Proptest saves a single seed per failure in
`test-harness/tests/<test>.proptest-regressions`. These files are
auto-regression tests — commit them and proptest replays them on every future
run before exploring new inputs.

```bash
# Replay the saved regression
cargo test --release -p test-harness --test fuzz_supply_borrow_liquidate

# Increase verbosity for debugging
PROPTEST_VERBOSE=2 cargo test --release -p test-harness --test <name> -- --nocapture
```

## Expected costs (AX41 ~ €40/mo on-demand)

| Campaign | Wall time | Iterations | Cost at on-demand rate |
|---|---|---|---|
| PR smoke | ~5 min | ~10M function-level + 256 contract | negligible |
| Release-gate | ~5 h | ~180M + 50k contract | ~€0.40 |
| Weekly soak | ~24 h | ~900M + 500k contract | ~€2 |

The campaign is embarrassingly parallel: multiple Hetzner boxes can run
different targets simultaneously. Save artifacts to S3 or a persistent
volume between runs.

## Reporting

Write findings into `bugs.md` under a new **## FUZZ** section with the
reproducing input attached. For each confirmed bug:

1. Add the minimized crash input to `corpus/<target>/regression-<id>`
   (function-level) or commit the `.proptest-regressions` file
   (contract-level).
2. Open a GitHub issue tagged `fuzz` with the finding summary and affected
   LOC.
3. After the fix lands, leave the regression corpus in place — it becomes a
   permanent guard against the same class of bug recurring.
