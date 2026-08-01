#!/usr/bin/env bash

set -o pipefail

max_attempts="${SOROBAN_SCANNER_MAX_ATTEMPTS:-5}"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

for ((attempts = 1; attempts <= max_attempts; attempts++)); do
  if soroban-scanner scan . --project-root . \
      --exclude .claude/ vendor/ target/ .certora_internal \
      >"$tmp/scan.json" 2>"$tmp/scan.err"; then
    python3 "$script_dir/scope_scanner_output.py" <"$tmp/scan.json"
    exit 0
  fi
  echo "::warning::soroban-scanner failed on attempt $attempts/$max_attempts" >&2
  [ -s "$tmp/scan.err" ] && sed 's/^/[scanner stderr] /' "$tmp/scan.err" >&2
  [ "$attempts" -lt "$max_attempts" ] && sleep 2
done

echo "::error::soroban-scanner crashed $max_attempts times in a row" >&2
exit 1
