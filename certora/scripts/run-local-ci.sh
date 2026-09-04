#!/usr/bin/env bash
#
# Local prover regression driver for CI (see .github/workflows/certora-local.yml).
#
# Runs the Certora Sunbeam prover locally on the self-hosted runner, batching
# each conf's rules in parallel (-j) with a hard per-rule budget, then
# aggregates results:
#   - VIOLATED or engine/tooling ERROR  -> failure (exit 1)
#   - VERIFIED                          -> pass
#   - TIMEOUT (prover's own verdict)    -> warning only; the rule is expected
#     to prove on the Certora cloud with the conf's cloud budgets
#   - KILLED (wrapper cap hit first)    -> warning only; the prover never
#     returned, so this says nothing about the rule
#
# Tuning (env): CERTORA_LOCAL_JOBS (default 10) parallel provers, each a JVM
# with -Xmx8g; CERTORA_RULE_TIMEOUT (default 900s) per-rule cap.
#
# Usage: run-local-ci.sh [CONFS] [RULES]
#   CONFS: space-separated conf paths relative to certora/ (without .conf);
#          empty = default set
#   RULES: optional space-separated rule names applied to every conf;
#          empty = all rules of each conf

set -uo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
log_dir="${CERTORA_LOG_DIR:-$repo_root/target/certora-local-logs}"
mkdir -p "$log_dir"

jobs="${CERTORA_LOCAL_JOBS:-10}"
rule_timeout="${CERTORA_RULE_TIMEOUT:-900}"

if [ $# -gt 0 ] && [ "$1" = "all" ]; then
  # Every conf in the tree. Only sensible with a raised CERTORA_RULE_TIMEOUT and
  # a job window to match — the default set below exists precisely because the
  # full sweep does not fit a short one.
  mapfile -t confs < <(cd "$repo_root/certora" && find . -name '*.conf' \
    | sed 's|^\./||; s|\.conf$||' | sort)
  echo "=== conf set: ALL (${#confs[@]} confs)"
elif [ $# -gt 0 ] && [ -n "$1" ]; then
  read -r -a confs <<< "$1"
else
  # Default set trimmed to the confs measured to fit a 2h job window on the
  # self-hosted runner (2026-08-13 run: 7 confs ≈ 90 min wall, incl. rules
  # that burn their full 5-min budget). The heavier accounting confs
  # (rate-accounting, rate-index-accounting) and tolerance-math are opt-in:
  # pass them explicitly to run them, or on the Certora cloud.
  confs=(
    common/confs/math common/confs/rates common/confs/lp-math
    common/confs/lp-math-stable common/confs/compound-interest
    common/confs/rate-indexes price-aggregator/confs/scaled-math
  )
fi
rules_arg="${2:-}"

failed=0
declare -a verdicts

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

  conf_base=${c##*/}
  echo "=== $c -- ${#rules[@]} rules (parallel jobs=$jobs, cap ${rule_timeout}s)"
  "$repo_root/certora/scripts/run-rules-local.sh" -j "$jobs" "$conf_path" "${rules[@]}" > "$log_dir/$conf_base-conf.log" 2>&1

  for rule in "${rules[@]}"; do
    safe=$(printf '%s' "$rule" | tr -c '[:alnum:]_.-' '_')
    rlog="$log_dir/$conf_base-$safe.log"
    # Classify on the rule's own sub-rules, never on a bare "Violated:" match.
    # Under `rule_sanity: advanced` the prover emits a vacuity sub-rule named
    # "<rule>-Assertions-rule_not_vacuous_tac", whose body is `assert false`.
    # Reaching that assert is what proves the rule is NOT vacuous, so the
    # prover reports it as Violated on a healthy rule. Matching "Violated:"
    # anywhere turned 17 verified rules red in run 33711445573.
    # Under `rule_sanity: none` there is no sub-rule and the line is a bare
    # "Violated: <rule>"; accept that exact form too so a real violation on
    # an `all`-mode conf cannot fall through to TIMEOUT.
    #
    # Unwinding is tested first: with optimistic_loop false the prover asserts
    # the unwinding condition, so a rule whose loop exceeds loop_iter prints
    # both "Violated: <rule>" and "Unwinding condition in a loop". That is a
    # config failure, not a counterexample: raise loop_iter for this conf.
    if grep -q "Unwinding condition in a loop" "$rlog" 2>/dev/null; then
      verdict="UNWIND"
    elif grep -qE "^ *Violated: ${rule}(-Assertions)?\$" "$rlog" 2>/dev/null; then
      verdict="VIOLATED"
    elif grep -qE "^ *Verified: ${rule}(-Assertions)?\$" "$rlog" 2>/dev/null; then
      verdict="VERIFIED"
    elif grep -q "^KILLED:" "$rlog" 2>/dev/null; then
      verdict="KILLED"
    elif [ ! -s "$rlog" ]; then
      verdict="NO-VERDICT"
    else
      verdict="TIMEOUT"
    fi

    echo "  -> $verdict"
    case "$verdict" in
      VIOLATED)
        echo "::error::$c/$rule [$verdict] — inspect $rlog"
        tail -25 "$rlog" | sed 's/^/    /'
        failed=1
        ;;
      UNWIND)
        echo "::error::$c/$rule [$verdict] — loop unwinding assertion failed; raise loop_iter in $c (never optimistic_loop) and inspect $rlog"
        tail -25 "$rlog" | sed 's/^/    /'
        failed=1
        ;;
      NO-VERDICT)
        echo "::warning::$c/$rule [$verdict] — prover did not start; inspect $rlog"
        tail -25 "$rlog" | sed 's/^/    /'
        failed=1
        ;;
      KILLED)
        echo "::warning::$c/$rule [$verdict] — hit the ${rule_timeout}s wrapper cap before the prover returned; raise CERTORA_RULE_TIMEOUT or shrink the rule"
        ;;
      TIMEOUT)
        echo "::warning::$c/$rule [$verdict] — the prover reported its own timeout; verify on Certora cloud"
        ;;
    esac
    verdicts+=("$c/$rule:$verdict")
  done
done

echo
echo "=== local prover summary ==="
printf '%s\n' "${verdicts[@]}" | sort

exit "$failed"
