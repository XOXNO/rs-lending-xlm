#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

cargo check -p common --features certora
cargo check -p pool --features certora --no-default-features
cargo check -p controller --features certora --no-default-features
python3 verification/certora/check_orphans.py
