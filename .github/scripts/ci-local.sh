#!/usr/bin/env bash
# Run the same checks as .github/workflows/ci.yml (and optional extras) locally.
#
# Usage:
#   bash .github/scripts/ci-local.sh              # ci.yml build-and-test job
#   bash .github/scripts/ci-local.sh --security   # + security-scan job (slow)
#   bash .github/scripts/ci-local.sh --fuzz       # + fuzz.yml pr-smoke subset
#   bash .github/scripts/ci-local.sh --install-tools
#
# Environment (optional):
#   STELLAR_VERSION=26.0.0   stellar-cli pin (matches CI)
#   RUST_TOOLCHAIN=1.95      Rust toolchain channel (matches CI)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

STELLAR_VERSION="${STELLAR_VERSION:-26.0.0}"
RUST_TOOLCHAIN="${RUST_TOOLCHAIN:-1.95}"

RUN_SECURITY=0
RUN_FUZZ=0
INSTALL_TOOLS=0

usage() {
  sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --security) RUN_SECURITY=1 ;;
    --fuzz) RUN_FUZZ=1 ;;
    --install-tools) INSTALL_TOOLS=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

if [[ -t 1 ]]; then
  GREEN='\033[0;32m'
  RED='\033[0;31m'
  BOLD='\033[1m'
  RESET='\033[0m'
else
  GREEN='' RED='' BOLD='' RESET=''
fi

STEP=0
FAILURES=0
FAILED_STEPS=()

run_step() {
  local label="$1"
  shift
  STEP=$((STEP + 1))
  echo ""
  echo -e "${BOLD}==> [$STEP] $label${RESET}"
  if "$@"; then
    echo -e "${GREEN}PASS${RESET}  $label"
  else
    echo -e "${RED}FAIL${RESET}  $label"
    FAILURES=$((FAILURES + 1))
    FAILED_STEPS+=("$label")
  fi
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

ensure_rust_toolchain() {
  if rustup toolchain list --installed 2>/dev/null | grep -qE "^${RUST_TOOLCHAIN}(-|$)"; then
    :
  elif [[ "$INSTALL_TOOLS" -eq 1 ]]; then
    rustup toolchain install "$RUST_TOOLCHAIN" \
      --target wasm32v1-none \
      --component clippy,rustfmt
  else
    echo "Rust ${RUST_TOOLCHAIN} not installed." >&2
    echo "Install with: rustup toolchain install ${RUST_TOOLCHAIN} --target wasm32v1-none --component clippy,rustfmt" >&2
    echo "Or re-run with --install-tools" >&2
    exit 1
  fi
  export RUSTUP_TOOLCHAIN="$RUST_TOOLCHAIN"
}

ensure_nightly() {
  if rustup toolchain list --installed 2>/dev/null | grep -q '^nightly'; then
    :
  elif [[ "$INSTALL_TOOLS" -eq 1 ]]; then
    rustup toolchain install nightly --target wasm32v1-none
  else
    echo "Rust nightly not installed (needed for --security / --fuzz)." >&2
    echo "Install with: rustup toolchain install nightly --target wasm32v1-none" >&2
    echo "Or re-run with --install-tools" >&2
    exit 1
  fi
}

ensure_stellar_cli() {
  # install-stellar-cli.sh updates PATH when not in GitHub Actions.
  STELLAR_VERSION="$STELLAR_VERSION" bash .github/scripts/install-stellar-cli.sh
  export PATH="${HOME}/.local/bin:${PATH}"
}

require_cmd rustup
require_cmd cargo
require_cmd make
require_cmd python3

echo "Repository: $ROOT"
echo "Rust toolchain: $RUST_TOOLCHAIN"
echo "stellar-cli: v${STELLAR_VERSION}"

ensure_rust_toolchain
run_step "Install stellar-cli" ensure_stellar_cli
run_step "WASM deploy artifacts + size budget (make wasm-size-check)" make wasm-size-check
run_step "Workspace tests (cargo test --workspace)" cargo test --workspace
run_step "Clippy workspace (-D warnings)" \
  cargo clippy --workspace --all-targets -- -D warnings
run_step "Clippy fuzz crate" make clippy-fuzz
run_step "Certora compile and coverage gates" ./certora/compile_all.sh

if [[ "$RUN_FUZZ" -eq 1 ]]; then
  ensure_nightly
  if ! command -v cargo-fuzz >/dev/null 2>&1; then
    if [[ "$INSTALL_TOOLS" -eq 1 ]]; then
      run_step "Install cargo-fuzz" cargo +nightly install cargo-fuzz --locked
    else
      echo "cargo-fuzz not found; install with: cargo +nightly install cargo-fuzz --locked" >&2
      echo "Or re-run with --install-tools" >&2
      exit 1
    fi
  fi
  run_step "Build contracts (stellar contract build)" stellar contract build
  run_step "Function-level fuzz smoke (30s)" make fuzz FUZZ_TIME=30
  run_step "Contract-level fuzz smoke (60s)" make fuzz-contract FUZZ_TIME=60
  run_step "Contract-level proptest (256 cases)" make proptest PROPTEST_CASES=256
  run_step "Miri (make miri-all)" make miri-all
fi

if [[ "$RUN_SECURITY" -eq 1 ]]; then
  ensure_nightly
  if ! command -v soroban-scanner >/dev/null 2>&1; then
    run_step "Install soroban-scanner (nightly)" \
      cargo +nightly install --git https://github.com/XOXNO/soroban-security-detectors-sdk \
        --rev 43231611f039c444c4a1db0ef2ef25c3257afff1 \
        soroban-security-detectors-runner
  fi
  run_step "Security scan (scoped)" .github/scripts/run_scanner.sh
  run_step "Security scan strict gate (no CRITICAL/HIGH)" bash -c '
    .github/scripts/run_scanner.sh | tee /tmp/rs-lending-scanner.txt
    if grep -q "CRITICAL\|HIGH" /tmp/rs-lending-scanner.txt; then
      echo "Security scan found CRITICAL or HIGH severity issues" >&2
      exit 1
    fi
  '
fi

echo ""
if [[ "$FAILURES" -eq 0 ]]; then
  echo -e "${GREEN}${BOLD}All ${STEP} step(s) passed.${RESET}"
  exit 0
fi

echo -e "${RED}${BOLD}${FAILURES} step(s) failed:${RESET}"
for label in "${FAILED_STEPS[@]}"; do
  echo "  - $label"
done
exit 1