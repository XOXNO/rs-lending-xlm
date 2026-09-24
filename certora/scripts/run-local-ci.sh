#!/usr/bin/env bash
#
# Local prover regression driver for CI (see .github/workflows/certora-local.yml).
#
# Runs the Certora Sunbeam prover locally on the self-hosted runner, batching
# each conf's rules in parallel (-j) with a hard per-rule budget, then
# classifies each rule log:
#   - VIOLATED, UNWIND, SANITY_FAILED   -> failure (exit 1)
#   - NO-VERDICT (empty or missing log) -> failure (exit 1)
#   - ERROR (any other log: a prover,   -> failure (exit 1)
#     CLI or JVM error)
#   - VERIFIED                          -> pass
#   - KILLED (wrapper cap hit first)    -> warning only; the prover never
#     returned, so this says nothing about the rule
#   - TIMEOUT, UNKNOWN (the prover's    -> warning only; prove the rule on
#     own "<rule>: Solver timed out" or    the Certora cloud
#     "Solver failed" line)
# A missing conf, a RULES name the conf does not list, or a runner that stops
# before the provers (no Python, stale artifact) also fails the run. Each
# rule's log is deleted before its run, so an old verdict is never read again.
#
# Tuning (env): CERTORA_LOCAL_JOBS (default 10) parallel provers, each a JVM
# with -Xmx8g; CERTORA_RULE_TIMEOUT (default 900s) per-rule cap.
#
# Usage: run-local-ci.sh [CONFS] [RULES]
#   CONFS: space-separated conf paths relative to certora/ (without .conf);
#          `all` = every conf; empty = default set
#   RULES: optional space-separated rule names applied to every conf; each
#          conf must list every name; empty = all rules of each conf

set -uo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
log_dir="${CERTORA_LOG_DIR:-$repo_root/target/certora-local-logs}"
mkdir -p "$log_dir"

jobs="${CERTORA_LOCAL_JOBS:-10}"
rule_timeout="${CERTORA_RULE_TIMEOUT:-900}"

confs=()
if [ $# -gt 0 ] && [ "$1" = "all" ]; then
  # Every conf in certora/<layer>/confs/, the set check_orphans.py checks.
  # Needs a raised CERTORA_RULE_TIMEOUT and a job timeout to match; the
  # default set below fits a shorter one.
  while IFS= read -r c; do
    confs+=("$c")
  done < <(cd "$repo_root/certora" && printf '%s\n' */confs/*.conf | sed 's|\.conf$||' | sort)
  echo "=== conf set: ALL (${#confs[@]} confs)"
elif [ $# -gt 0 ] && [ -n "$1" ]; then
  read -r -a confs <<< "$1"
else
  # Default set: the confs that fit the pull-request job window. Pass the
  # heavier confs (rate-accounting, rate-index-accounting, tolerance-math)
  # explicitly, or prove them on the Certora cloud.
  confs=(
    common/confs/math common/confs/rates common/confs/lp-math
    common/confs/lp-math-stable common/confs/compound-interest
    common/confs/rate-indexes price-aggregator/confs/scaled-math
    pool/confs/pool-lifecycle
  )
fi
rules_arg="${2:-}"

failed=0
verdicts=()

for c in ${confs[@]+"${confs[@]}"}; do
  conf_path="$repo_root/certora/$c.conf"
  if [ ! -f "$conf_path" ]; then
    echo "::error::conf not found: $conf_path"
    failed=1
    continue
  fi

  conf_rules=()
  while IFS= read -r rule; do
    conf_rules+=("$rule")
  done < <(python3 -c "import json,sys; print(*json.load(open(sys.argv[1]))['rule'], sep='\n')" "$conf_path")

  rules=()
  if [ -n "$rules_arg" ]; then
    read -r -a requested <<< "$rules_arg"
    for rule in "${requested[@]}"; do
      if printf '%s\n' ${conf_rules[@]+"${conf_rules[@]}"} | grep -qxF -- "$rule"; then
        rules+=("$rule")
      else
        echo "::error::$c does not list rule $rule; fix the rules input"
        failed=1
      fi
    done
  else
    rules=(${conf_rules[@]+"${conf_rules[@]}"})
  fi
  if [ "${#rules[@]}" -eq 0 ]; then
    continue
  fi

  conf_base=${c##*/}
  for rule in "${rules[@]}"; do
    safe=$(printf '%s' "$rule" | tr -c '[:alnum:]_.-' '_')
    rm -f -- "$log_dir/$conf_base-$safe.log"
  done

  echo "=== $c -- ${#rules[@]} rules (parallel jobs=$jobs, cap ${rule_timeout}s)"
  "$repo_root/certora/scripts/run-rules-local.sh" -j "$jobs" "$conf_path" "${rules[@]}" > "$log_dir/$conf_base-conf.log" 2>&1
  runner_status=$?
  # Exit 1 means a prover run failed, and its rule log says how. Any other
  # non-zero exit comes from the runner itself.
  if [ "$runner_status" -gt 1 ]; then
    echo "::error::$c: run-rules-local.sh exited $runner_status; inspect $log_dir/$conf_base-conf.log"
    tail -25 "$log_dir/$conf_base-conf.log" | sed 's/^/    /'
    failed=1
  fi

  for rule in "${rules[@]}"; do
    safe=$(printf '%s' "$rule" | tr -c '[:alnum:]_.-' '_')
    rlog="$log_dir/$conf_base-$safe.log"
    # Anchor on the rule's own line: under rule_sanity advanced the vacuity
    # sub-rule "<rule>-Assertions-rule_not_vacuous_tac" prints "Violated:" on
    # a healthy rule. Unwinding and SANITY_FAILED are checked before the
    # verdict: the first is a loop_iter config failure that also prints
    # "Violated:", the second is a vacuous proof that also prints "Verified:".
    if grep -q "Unwinding condition in a loop" "$rlog" 2>/dev/null; then
      verdict="UNWIND"
    elif grep -q "SANITY_FAILED" "$rlog" 2>/dev/null; then
      verdict="SANITY_FAILED"
    elif grep -qE "^ *Violated: ${rule}(-Assertions)?\$" "$rlog" 2>/dev/null; then
      verdict="VIOLATED"
    elif grep -qE "^ *Verified: ${rule}(-Assertions)?\$" "$rlog" 2>/dev/null; then
      verdict="VERIFIED"
    elif grep -q "^KILLED:" "$rlog" 2>/dev/null; then
      verdict="KILLED"
    elif [ ! -s "$rlog" ]; then
      verdict="NO-VERDICT"
    elif grep -qE "^ *${rule}(-Assertions)?: Solver timed out\$" "$rlog"; then
      verdict="TIMEOUT"
    elif grep -qE "^ *${rule}(-Assertions)?: Solver failed\$" "$rlog"; then
      verdict="UNKNOWN"
    else
      verdict="ERROR"
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
      SANITY_FAILED)
        echo "::error::$c/$rule [$verdict] — the assertions verified but every path is cut by assumes or traps; the proof is vacuous. Inspect $rlog"
        tail -25 "$rlog" | sed 's/^/    /'
        failed=1
        ;;
      NO-VERDICT)
        echo "::error::$c/$rule [$verdict] — prover did not start; inspect $rlog"
        tail -25 "$rlog" 2>/dev/null | sed 's/^/    /'
        failed=1
        ;;
      ERROR)
        echo "::error::$c/$rule [$verdict] — the log has no prover verdict; a prover, CLI or JVM error ended the run. Inspect $rlog"
        tail -25 "$rlog" | sed 's/^/    /'
        failed=1
        ;;
      KILLED)
        echo "::warning::$c/$rule [$verdict] — hit the ${rule_timeout}s wrapper cap before the prover returned; raise CERTORA_RULE_TIMEOUT or shrink the rule"
        ;;
      TIMEOUT)
        echo "::warning::$c/$rule [$verdict] — the prover reported its own timeout; verify on Certora cloud"
        ;;
      UNKNOWN)
        echo "::warning::$c/$rule [$verdict] — the local solvers returned no answer; verify on Certora cloud"
        ;;
    esac
    verdicts+=("$c/$rule:$verdict")
  done
done

echo
echo "=== local prover summary ==="
printf '%s\n' ${verdicts[@]+"${verdicts[@]}"} | sort

exit "$failed"
