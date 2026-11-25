#!/usr/bin/env bash
# RUSTC_WRAPPER for libFuzzer builds: coverage instrumentation for the
# workspace crates only.
#
# cargo-fuzz instruments every crate for coverage. The Soroban host, wasmi,
# serde and the crypto crates then pay for a counter on each edge, and their
# edges reward inputs for host internals, not for protocol behaviour. This
# wrapper removes the coverage flags from crates that come from the cargo
# registry or a git checkout. Sanitizer, debug-assertion and overflow-check
# flags stay on every crate.
set -euo pipefail

rustc="$1"
shift
# A wrapper that the build already used, such as sccache, runs after this one.
next="${FUZZ_NEXT_RUSTC_WRAPPER:-}"

external=0
for arg in "$@"; do
  case "$arg" in
    */.cargo/registry/src/* | */.cargo/git/checkouts/* | */cargo/registry/src/* | */cargo/git/checkouts/*)
      external=1
      break
      ;;
  esac
done

if [ "$external" = 0 ]; then
  exec ${next:+"$next"} "$rustc" "$@"
fi

args=()
skip_next=0
for arg in "$@"; do
  if [ "$skip_next" = 1 ]; then
    skip_next=0
    case "$arg" in
      passes=sancov-module | llvm-args=-sanitizer-coverage-*) continue ;;
      *) args+=("-C" "$arg"); continue ;;
    esac
  fi
  case "$arg" in
    -C) skip_next=1 ;;
    -Cpasses=sancov-module | -Cllvm-args=-sanitizer-coverage-*) ;;
    *) args+=("$arg") ;;
  esac
done
exec ${next:+"$next"} "$rustc" "${args[@]}"
