#!/usr/bin/env bash
# Runs libFuzzer targets for FUZZ_TIME seconds each, FUZZ_JOBS targets at a
# time. Each libFuzzer process uses one core, so parallel targets get the same
# fuzzing time each in less wall-clock time.
#
# Usage: run_fuzz_targets.sh <target>...
# Environment: FUZZ_DIR, FUZZ_FLAGS, FUZZ_TIME, FUZZ_MAX_LEN, FUZZ_LEN_CONTROL,
# FUZZ_JOBS (default 1).
#
# Every target runs to its end. The script then prints the last lines of each
# log in target order, prints the last 80 lines of each failed log, and exits 1
# if any target failed.
set -uo pipefail

: "${FUZZ_DIR:?}" "${FUZZ_TIME:?}" "${FUZZ_MAX_LEN:?}" "${FUZZ_LEN_CONTROL:?}"
flags="${FUZZ_FLAGS:-}"
jobs="${FUZZ_JOBS:-1}"
case "$jobs" in
  '' | *[!0-9]* | 0) echo "FUZZ_JOBS must be a positive integer, got '$jobs'" >&2; exit 2 ;;
esac
[ "$#" -gt 0 ] || { echo "run_fuzz_targets.sh: no targets" >&2; exit 2; }

logs=$(mktemp -d "${TMPDIR:-/tmp}/fuzz-logs.XXXXXX")
trap 'rm -rf -- "$logs"' EXIT

run_target() {
  local target="$1"
  mkdir -p "$FUZZ_DIR/corpus/$target"
  # FUZZ_FLAGS holds several flags, so it is split on purpose.
  # shellcheck disable=SC2086
  cargo +nightly fuzz run --fuzz-dir "$FUZZ_DIR" $flags "$target" \
    "$FUZZ_DIR/corpus/$target" "$FUZZ_DIR/seeds/$target" -- \
    -max_total_time="$FUZZ_TIME" -max_len="$FUZZ_MAX_LEN" \
    -len_control="$FUZZ_LEN_CONTROL" > "$logs/$target.log" 2>&1
  echo "$?" > "$logs/$target.status"
}

running=0
for target in "$@"; do
  if [ "$running" -ge "$jobs" ]; then
    wait -n
    running=$((running - 1))
  fi
  echo "=== $target: started ($FUZZ_TIME s) ==="
  run_target "$target" &
  running=$((running + 1))
done
wait

failed=0
for target in "$@"; do
  echo "=== $target ==="
  status=$(cat "$logs/$target.status" 2>/dev/null || echo missing)
  if [ "$status" = 0 ]; then
    tail -3 "$logs/$target.log"
  else
    echo "::error::fuzz $target crashed (exit $status):"
    tail -80 "$logs/$target.log" 2>/dev/null
    failed=1
  fi
done
exit "$failed"
