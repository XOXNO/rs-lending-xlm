#!/usr/bin/env bash
# Wrapper that runs soroban-scanner against the workspace, retries on
# stack-overflow, and pipes the result through scope_scanner_output.py.
#
# Why the retry: upstream soroban-scanner's symbol resolver recurses
# unboundedly for some HashMap iteration orders. The XOXNO fork runs it on
# a 4 GiB-stack worker thread, which masks most of the overflow, but the
# Linux self-hosted runner still trips it occasionally. Stack-sizing past
# 4 GiB does not help because the recursion is effectively infinite for
# the pathological orderings; a retry picks a new ordering and completes.
#
# Exits 0 on first successful scan, 1 after SOROBAN_SCANNER_MAX_ATTEMPTS
# (default 5) consecutive failures.
set -o pipefail

max_attempts="${SOROBAN_SCANNER_MAX_ATTEMPTS:-5}"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

attempts=0
while :; do
  attempts=$((attempts + 1))
  if soroban-scanner scan . --project-root . \
      --exclude vendor/ target/ .certora_internal \
      > "$tmp/scan.json" 2> "$tmp/scan.err"; then
    python3 .github/scripts/scope_scanner_output.py < "$tmp/scan.json"
    exit 0
  fi

  echo "::warning::soroban-scanner failed on attempt $attempts/$max_attempts" >&2
  if [ -s "$tmp/scan.err" ]; then
    sed 's/^/[scanner stderr] /' "$tmp/scan.err" >&2
  fi

  if [ "$attempts" -ge "$max_attempts" ]; then
    echo "::error::soroban-scanner crashed $max_attempts times in a row" >&2
    exit 1
  fi
  sleep 2
done
