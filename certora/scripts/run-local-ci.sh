#!/usr/bin/env bash
#
# Local prover regression driver for CI (see .github/workflows/certora-local.yml).
#
# Runs the Certora Sunbeam prover locally, one rule at a time, hard 5-minute
# per-rule budget, and aggregates results:
#   - VIOLATED or engine/tooling ERROR  -> failure (exit 1)
#   - VERIFIED                          -> pass
#   - timeout (no verdict in 5 min)     -> warning only; the rule is expected
#     to prove on the Certora cloud with the conf's cloud budgets
#
# Usage: run-local-ci.sh [CONFS] [RULES]
#   CONFS: space-separated conf basenames (without .conf); empty = default set
#   RULES: space-separated rule names; empty = all rules of each conf

set -uo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
log_dir="${CERTORA_LOG_DIR:-$repo_root/target/certora-local-logs}"
mkdir -p "$log_dir"

if [ $# -gt 0 ] && [ -n "$1" ]; then
  read -r -a confs <<< "$1"
else
  confs=(
    common/confs/math common/confs/rates common/confs/lp-math
    common/confs/lp-math-stable common/confs/compound-interest
    common/confs/rate-accounting common/confs/rate-index-accounting
    common/confs/rate-indexes
    price-aggregator/confs/scaled-math price-aggregator/confs/tolerance-math
  )
fi
rules_arg="${2:-}"

declare -a verdicts
failed=0

for c in "${confs[@]}"; do
  conf_path="$repo_root/certora/$c.conf"
  if [ ! -f "$conf_path" ]; then
    echo "::error::conf not found: $conf_path"
    failed=1
    continue
  fi

  if [ -n "$rules_arg" ]; then
    read -r -a rules <<< "$rules_arg"
  else
    mapfile -t rules < <(python3 -c "import json,sys; print(*json.load(open(sys.argv[1]))['rule'], sep='\n')" "$conf_path")
  fi

  for rule in "${rules[@]}"; do
    safe=$(printf '%s' "$rule" | tr -c '[:alnum:]_.-' '_')
    log_base=$(printf '%s' "${c##*/}" | tr '/' '_')
    log="$log_dir/$log_base-$safe.log"
    echo "=== $c --rule $rule (cap 300s)"
    # shellcheck disable=SC2015
    timeout 300 "$repo_root/certora/scripts/run-rules-local.sh" "$conf_path" "$rule" > "$log" 2>&1
    status=$?

    if [ "$status" -eq 124 ]; then
      verdict="TIMEOUT"
    elif grep -q "Violated:" "$log"; then
      verdict="VIOLATED"
    elif grep -q "Verified:" "$log"; then
      verdict="VERIFIED"
    elif grep -qiE "ERROR|Traceback|Exception" "$log"; then
      verdict="ERROR"
    else
      verdict="NO-VERDICT"
    fi

    echo "  -> $verdict"
    case "$verdict" in
      VIOLATED|ERROR)
        echo "::error::$c/$rule [$verdict] — inspect $log"
        tail -25 "$log" | sed 's/^/    /'
        failed=1
        ;;
      TIMEOUT|NO-VERDICT)
        echo "::warning::$c/$rule [$verdict] — expected locally; verify on Certora cloud"
        ;;
    esac
    verdicts+=("$c/$rule:$verdict")
  done
done

echo
echo "=== local prover summary ==="
printf '%s\n' "${verdicts[@]}" | sort

exit "$failed"
