#!/usr/bin/env bash
# Fast fuzz coverage: replays existing corpus through instrumented binary and
# emits llvm-cov HTML + text summary. No active fuzzing is performed.
#
# Usage:
#   fuzz/coverage.sh                         # default: fast targets (fp_math, rates_and_index)
#   fuzz/coverage.sh fp_math flow_e2e        # explicit target list
#   FUZZ_COV_TIME=30 fuzz/coverage.sh ...    # optional short pre-fuzz to expand corpus
#   SANITIZER=thread fuzz/coverage.sh flow_e2e
#
# Env:
#   FUZZ_COV_TIME   If >0, run `cargo fuzz run` for N seconds to grow the corpus
#                   before collecting coverage. Default: 0 (corpus replay only).
#   SANITIZER       Passed through as `--sanitizer=$SANITIZER`. On macOS contract-level
#                   targets require `thread`. Default: "" (no sanitizer) — function-level.
#   BUILD_STD       If set, adds `-Zbuild-std` (needed with --sanitizer=thread).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
COV_OUT="$REPO_ROOT/target/coverage/fuzz"

DEFAULT_TARGETS=("fp_math" "rates_and_index")
TARGETS=("$@")
if [ ${#TARGETS[@]} -eq 0 ]; then
    TARGETS=("${DEFAULT_TARGETS[@]}")
fi

FUZZ_COV_TIME="${FUZZ_COV_TIME:-0}"
SANITIZER="${SANITIZER:-}"
BUILD_STD="${BUILD_STD:-}"

EXTRA_FLAGS=()
if [ -n "$SANITIZER" ]; then
    EXTRA_FLAGS+=("--sanitizer=$SANITIZER")
fi
if [ -n "$BUILD_STD" ]; then
    EXTRA_FLAGS+=("-Zbuild-std")
fi

# Locate llvm-cov. Prefer the nightly sysroot (llvm-tools-preview) because it
# matches the rustc that produced the profile data; fall back to system llvm-cov
# (homebrew `llvm` package on macOS) if the component isn't installed.
SYSROOT="$(rustc +nightly --print sysroot)"
HOST_TRIPLE="$(rustc +nightly -vV | sed -n 's|host: ||p')"
SYSROOT_LLVM_COV="$SYSROOT/lib/rustlib/$HOST_TRIPLE/bin/llvm-cov"

if [ -x "$SYSROOT_LLVM_COV" ]; then
    LLVM_COV="$SYSROOT_LLVM_COV"
elif command -v llvm-cov >/dev/null 2>&1; then
    LLVM_COV="$(command -v llvm-cov)"
    echo "note: using system llvm-cov ($LLVM_COV)"
    echo "      for best results: rustup component add llvm-tools-preview --toolchain nightly"
else
    echo "error: llvm-cov not found" >&2
    echo "       run: rustup component add llvm-tools-preview --toolchain nightly" >&2
    echo "       or:  brew install llvm  (macOS)" >&2
    exit 1
fi

# Optional demangler — prettier function names in HTML. Skip if missing.
DEMANGLER_ARGS=()
if command -v rustfilt >/dev/null 2>&1; then
    DEMANGLER_ARGS=(-Xdemangler=rustfilt)
fi

# Source files to exclude. Keeps only project surface: common/, controller/,
# pool/, test-harness/src/. Filters std (toolchain), deps (cargo registry),
# the fuzz harness itself, and the test-harness tests.
IGNORE_REGEX='(\.rustup/|/\.cargo/|/rustc/|rs-lending-xlm/fuzz/|test-harness/tests/)'

mkdir -p "$COV_OUT"

if [ ${#EXTRA_FLAGS[@]} -eq 0 ]; then
    FLAGS_DISPLAY="<none>"
else
    FLAGS_DISPLAY="${EXTRA_FLAGS[*]}"
fi

echo
echo "=============================================="
echo "  Fuzz coverage"
echo "  targets:   ${TARGETS[*]}"
echo "  pre-fuzz:  ${FUZZ_COV_TIME}s"
echo "  flags:     ${FLAGS_DISPLAY}"
echo "  output:    $COV_OUT"
echo "=============================================="

cd "$SCRIPT_DIR"

for target in "${TARGETS[@]}"; do
    echo
    echo "--- $target ---"

    if [ "$FUZZ_COV_TIME" -gt 0 ]; then
        echo "  [1/3] short fuzz run (${FUZZ_COV_TIME}s)"
        cargo +nightly fuzz run "$target" ${EXTRA_FLAGS[@]+"${EXTRA_FLAGS[@]}"} -- \
            -max_total_time="$FUZZ_COV_TIME" >/dev/null 2>&1 || true
    fi

    echo "  [2/3] coverage build + corpus replay"
    # macOS + TSAN + coverage: `-Cinstrument-coverage` pulls in `profiler_builtins`,
    # which cargo-fuzz rebuilds under `-Zbuild-std` WITHOUT the sanitizer flag
    # (its internal RUSTFLAGS list is fixed). rustc then refuses the mix. This
    # env var tells rustc the mismatch is safe (profiler_builtins has no TSAN
    # runtime dependency). cargo-fuzz appends its own flags to user RUSTFLAGS,
    # so ours lands first and applies to sysroot crates.
    RUSTFLAGS="-Cunsafe-allow-abi-mismatch=sanitizer${RUSTFLAGS:+ }${RUSTFLAGS:-}" \
        cargo +nightly fuzz coverage "$target" ${EXTRA_FLAGS[@]+"${EXTRA_FLAGS[@]}"}

    profdata="coverage/$target/coverage.profdata"
    binary="target/$HOST_TRIPLE/coverage/$HOST_TRIPLE/release/$target"

    if [ ! -f "$profdata" ]; then
        echo "  error: $profdata not produced (corpus empty?)" >&2
        continue
    fi
    if [ ! -x "$binary" ]; then
        echo "  error: instrumented binary not found at $binary" >&2
        continue
    fi

    html_dir="$COV_OUT/$target"
    rm -rf "$html_dir"
    mkdir -p "$html_dir"

    echo "  [3/3] html + summary"
    "$LLVM_COV" show "$binary" \
        ${DEMANGLER_ARGS[@]+"${DEMANGLER_ARGS[@]}"} \
        -instr-profile="$profdata" \
        -ignore-filename-regex="$IGNORE_REGEX" \
        -show-line-counts-or-regions \
        -show-instantiations \
        -format=html \
        -output-dir="$html_dir" >/dev/null

    "$LLVM_COV" report "$binary" \
        ${DEMANGLER_ARGS[@]+"${DEMANGLER_ARGS[@]}"} \
        -instr-profile="$profdata" \
        -ignore-filename-regex="$IGNORE_REGEX" \
        | tee "$html_dir/summary.txt" | tail -30
done

echo
echo "=============================================="
echo "  Open: $COV_OUT/<target>/index.html"
echo "=============================================="
