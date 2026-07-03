#!/usr/bin/env bash
# Submit Certora Soroban verification jobs (Aave-style entry point).
#
# Usage (from repo root):
#   ./certora/scripts/run-all.sh [profile] [-- extra prover args...]
#
# Profiles: sanity | fast | core | critical | heavy | manual | all
# Default profile: fast (CI-stable subset).
#
# Requires CERTORAKEY for hosted runs. Use --dry-run to print commands only.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PROFILE="${1:-fast}"
shift || true

if [[ "${1:-}" == "--" ]]; then
  shift
fi

exec "$ROOT/certora/scripts/run_profile.py" "$PROFILE" "$@"
