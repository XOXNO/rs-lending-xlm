#!/usr/bin/env bash
# Compile every Certora feature path and verify conf/profile rule coverage.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

cargo check -p common --features certora
cargo check -p pool --features certora --no-default-features
cargo check -p controller --features certora --no-default-features
python3 verification/certora/check_orphans.py
python3 verification/certora/check_invariant_coverage.py
python3 verification/certora/scripts/sync_wasm_conf.py

if [[ "${1:-}" == "--wasm" ]]; then
  if ! command -v stellar >/dev/null 2>&1; then
    echo "stellar CLI required for --wasm (make certora-wasm)" >&2
    exit 1
  fi
  make certora-wasm
  python3 verification/certora/scripts/check_wasm_artifacts.py
fi