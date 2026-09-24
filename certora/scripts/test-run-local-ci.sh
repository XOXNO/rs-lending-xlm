#!/usr/bin/env bash
#
# Checks run-local-ci.sh's verdict classification against a stub prover. No
# prover runs. Usage: test-run-local-ci.sh (RUN_LOCAL_CI overrides the script
# under test).

set -uo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
runner="${RUN_LOCAL_CI:-$repo_root/certora/scripts/run-local-ci.sh}"
conf=common/confs/compound-interest
rule=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['rule'][0])" \
  "$repo_root/certora/$conf.conf")

tmp=$(mktemp -d "${TMPDIR:-/tmp}/run-local-ci-test.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

cat > "$tmp/prover" <<'EOF'
#!/usr/bin/env bash
while [ $# -gt 0 ]; do
  if [ "$1" = "--rule" ]; then rule="$2"; fi
  shift
done
case "$STUB_MODE" in
  verified) echo "Verified: $rule-Assertions"; exit 0 ;;
  violated) echo "Violated: $rule-Assertions"; exit 1 ;;
  timeout) echo "$rule-Assertions: Solver timed out"; exit 1 ;;
  unknown) echo "$rule-Assertions: Solver failed"; exit 1 ;;
  crash) echo 'Exception in thread "main" java.io.IOException: Cannot run program "z3"'; exit 1 ;;
esac
EOF
chmod +x "$tmp/prover"

failures=0

# expect <case> <wanted exit> <wanted verdict> <stub mode> <RULES> [VAR=value ...]
expect() {
  local name="$1" want="$2" verdict="$3" mode="$4" rules="$5"
  shift 5
  local logs="$tmp/$name"
  mkdir -p "$logs"
  if [ "$mode" = "stale" ]; then
    echo "Verified: $rule" > "$logs/${conf##*/}-$rule.log"
  fi
  env CERTORA_LOCAL="$tmp/prover" STUB_MODE="$mode" CERTORA_SKIP_ARTIFACT_CHECK=1 \
    CERTORA_LOG_DIR="$logs" CERTORA_LOCAL_JOBS=1 "$@" \
    bash "$runner" "$conf" "$rules" > "$logs/out.txt" 2>&1
  local got=$?
  if [ "$got" -eq "$want" ] && grep -q -- "$verdict" "$logs/out.txt"; then
    echo "ok   $name"
  else
    echo "FAIL $name: exit $got (want $want), output must contain '$verdict'"
    sed 's/^/    /' "$logs/out.txt"
    failures=1
  fi
}

expect verified 0 "-> VERIFIED" verified "$rule"
expect verified-conf-rules 0 "-> VERIFIED" verified ""
expect violated 1 "-> VIOLATED" violated "$rule"
expect timeout 0 "-> TIMEOUT" timeout "$rule"
expect solver-unknown 0 "-> UNKNOWN" unknown "$rule"
expect prover-crash 1 "-> ERROR" crash "$rule"
expect stale-log-runner-exits-early 1 "-> NO-VERDICT" stale "$rule" \
  CERTORA_LOCAL= CERTORA_PYTHON=/nonexistent
expect rule-not-in-conf 1 "does not list rule no_such_rule" verified "no_such_rule"

exit "$failures"
