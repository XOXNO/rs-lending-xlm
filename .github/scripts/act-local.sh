#!/usr/bin/env bash
# Run GitHub Actions workflows locally with nektos/act.
#
#   https://github.com/nektos/act
#
# Prerequisites: Docker running, `act` on PATH (brew install act).
# Runner images + platform are configured in the repo-root .actrc file.
#
# Usage:
#   .github/scripts/act-local.sh list
#   .github/scripts/act-local.sh ci                 # ci.yml → build-and-test
#   .github/scripts/act-local.sh ci --full          # ci.yml → all jobs
#   .github/scripts/act-local.sh certora-compile    # certora compile-check only
#   .github/scripts/act-local.sh fuzz-smoke         # fuzz.yml pr-smoke (slow)
#   .github/scripts/act-local.sh -n ci              # dry-run
#
# Certora hosted jobs need secrets:
#   cp .github/act/.secrets.example .github/act/.secrets
#   # edit CERTORAKEY=...
#   .github/scripts/act-local.sh certora-fast
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

EVENT="${ACT_EVENT:-pull_request}"
SECRET_FILE="${ACT_SECRET_FILE:-.github/act/.secrets}"

ACT_EXTRA=()

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

require_cmd act
require_cmd docker
docker info >/dev/null 2>&1 || {
  echo "Docker is not running. Start Docker Desktop (or the daemon) and retry." >&2
  exit 1
}

usage() {
  sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
}

run_act() {
  local -a args=("$EVENT" --directory "$ROOT")
  if [[ -f "$SECRET_FILE" ]]; then
    args+=(--secret-file "$SECRET_FILE")
  fi
  if [[ ${#ACT_EXTRA[@]} -gt 0 ]]; then
    args+=("${ACT_EXTRA[@]}")
  fi
  args+=("$@")
  echo "→ act ${args[*]}"
  act "${args[@]}"
}

if [[ $# -eq 0 ]]; then
  usage >&2
  exit 2
fi

while [[ $# -gt 0 && "$1" == -* ]]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    -n|--dryrun|-v|--verbose|--bind|--reuse)
      ACT_EXTRA+=("$1")
      shift
      ;;
    *)
      ACT_EXTRA+=("$1")
      shift
      ;;
  esac
done

cmd="${1:-}"
shift || true

case "$cmd" in
  list)
    run_act -l
    ;;
  ci)
    full=0
    if [[ "${1:-}" == "--full" ]]; then
      full=1
      shift
    fi
    if [[ "$full" -eq 1 ]]; then
      run_act -W .github/workflows/ci.yml "$@"
    else
      run_act -W .github/workflows/ci.yml -j build-and-test "$@"
    fi
    ;;
  certora-compile)
    run_act -W .github/workflows/certora-verification.yml -j compile-check "$@"
    ;;
  certora-fast)
    if [[ ! -f "$SECRET_FILE" ]] || ! grep -qE '^CERTORAKEY=.+$' "$SECRET_FILE" 2>/dev/null; then
      echo "certora-fast needs CERTORAKEY in $SECRET_FILE" >&2
      echo "  cp .github/act/.secrets.example .github/act/.secrets" >&2
      exit 1
    fi
    run_act -W .github/workflows/certora-fastRules.yml "$@"
    ;;
  fuzz-smoke)
    run_act -W .github/workflows/fuzz.yml -j pr-smoke "$@"
    ;;
  release-build)
    run_act workflow_dispatch -W .github/workflows/release.yml -j build "$@"
    ;;
  "")
    usage >&2
    exit 2
    ;;
  *)
    echo "unknown command: $cmd" >&2
    usage >&2
    exit 2
    ;;
esac