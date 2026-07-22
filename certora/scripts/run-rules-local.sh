#!/usr/bin/env bash
# Run every rule of a conf as its own local prover invocation.
# Default sequential; -j N runs N rules in isolated temporary work directories.
#
# Soundness note: Verified verdicts are final regardless of machine load;
# only Timeout verdicts are load-sensitive — retry those solo (-j 1).
#
# Usage: run-rules-local.sh [-j N] <path/to/conf> [rule ...]
set -euo pipefail
jobs=1
if [ "${1:-}" = "-j" ]; then jobs="$2"; shift 2; fi
conf_input="$1"; shift
conf_dir=$(cd "$(dirname "$conf_input")" && pwd -P)
conf="$conf_dir/$(basename "$conf_input")"
name=$(basename "$conf")
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)

# Bound each job's JVM heap so local preprocessing cannot exhaust the box.
# Raise this explicitly for a solo heavy rule when the machine has headroom.
heap="${CERTORA_JAVA_HEAP:--Xmx8g}"

# Local prover invocation (see README "Local prover"): CLI script + emv.jar.
# Override CERTORA_LOCAL with a full command prefix if the install moves.
install_dir="${CERTORA_INSTALL:-$HOME/certora-install}"
if [ -n "${CERTORA_LOCAL:-}" ]; then
  read -r -a local_cmd <<< "$CERTORA_LOCAL"
else
  certora_python="${CERTORA_PYTHON:-}"
  if [ -z "$certora_python" ]; then
    cli_path=$(command -v certoraSorobanProver 2>/dev/null || true)
    if [ -n "$cli_path" ]; then
      cli_shebang=$(head -1 "$cli_path" 2>/dev/null || true)
      case "$cli_shebang" in
        '#!'/*) certora_python=${cli_shebang#\#!} ;;
      esac
    fi
  fi
  if [ -z "$certora_python" ]; then
    for candidate in "${VIRTUAL_ENV:-}/bin/python" "$install_dir/.venv/bin/python" "$install_dir/venv/bin/python" python3; do
      [ "$candidate" = "/bin/python" ] && continue
      if "$candidate" -c 'import urllib3' >/dev/null 2>&1; then
        certora_python="$candidate"
        break
      fi
    done
  fi
  if [ -z "$certora_python" ] || ! "$certora_python" -c 'import urllib3' >/dev/null 2>&1; then
    echo "No Python with urllib3 found for the local Certora CLI." >&2
    echo "Set CERTORA_PYTHON to the interpreter from the certora-cli virtualenv." >&2
    exit 2
  fi
  local_cmd=("$certora_python" "$install_dir/certoraSorobanProver.py")
fi

# Every invocation gets a private working directory. Certora resets
# .certora_internal at startup, so sharing the conf directory makes parallel
# invocations delete each other's active build state. Absolute WASM paths keep
# the temporary config independent of its location.
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/certora-local.XXXXXX")
local_conf="$work_dir/local.conf"
python3 - "$conf" "$local_conf" "${CERTORA_LOCAL_SPLIT_PARALLEL:-false}" <<'PY'
import json
import sys
from pathlib import Path

source, target, keep_split = sys.argv[1:]
source_path = Path(source).resolve()
with open(source) as handle:
    data = json.load(handle)
files = data.get("files", [])
was_string = isinstance(files, str)
if was_string:
    files = [files]
data["files"] = [
    str((source_path.parent / file).resolve()) if not Path(file).is_absolute() else file
    for file in files
]
if was_string:
    data["files"] = data["files"][0]
if keep_split != "true":
    data["prover_args"] = [
        arg for arg in data.get("prover_args", []) if arg != "-splitParallel true"
    ]
with open(target, "w") as handle:
    json.dump(data, handle, indent=2)
    handle.write("\n")
PY

active_pids=()

terminate_tree() {
  local parent="$1"
  local child
  while IFS= read -r child; do
    [ -n "$child" ] && terminate_tree "$child"
  done < <(pgrep -P "$parent" 2>/dev/null || true)
  kill -TERM "$parent" 2>/dev/null || true
}

stop_children() {
  local pid
  for pid in "${active_pids[@]}"; do
    terminate_tree "$pid"
  done
  for pid in "${active_pids[@]}"; do
    wait "$pid" 2>/dev/null || true
  done
  active_pids=()
}

cleanup() {
  if [ -n "${work_dir:-}" ] && [ -d "$work_dir" ]; then
    case "$work_dir" in
      "${TMPDIR:-/tmp}"/certora-local.*) rm -rf -- "$work_dir" ;;
      *) echo "Refusing to remove unexpected local work directory: $work_dir" >&2 ;;
    esac
  fi
}

on_signal() {
  stop_children
  cleanup
  exit 130
}

trap cleanup EXIT
trap on_signal HUP INT TERM

if [ $# -gt 0 ]; then rules=("$@"); else
  rules=()
  while IFS= read -r rule; do
    rules+=("$rule")
  done < <(python3 -c "import json,sys; [print(r) for r in json.load(open(sys.argv[1]))['rule']]" "$conf")
fi

# Homebrew Java can occasionally be rejected by macOS library-signature
# policy. Prefer an installed Temurin/JDK 21 unless the caller pins Java.
java_home="${CERTORA_JAVA_HOME:-${JAVA_HOME:-}}"
if [ -z "$java_home" ] && [ -x /usr/libexec/java_home ]; then
  java_home=$(/usr/libexec/java_home -v 21 2>/dev/null || true)
fi

log_dir="${CERTORA_LOG_DIR:-$repo_root/target/certora-local-logs}"
mkdir -p "$log_dir"

run_one() {
  local r="$1"
  local safe_rule
  local run_dir
  local log
  local status
  safe_rule=$(printf '%s' "$r" | tr -c '[:alnum:]_.-' '_')
  run_dir="$work_dir/run-$safe_rule"
  mkdir -p "$run_dir"
  log="$log_dir/${name%.conf}-$safe_rule.log"
  set +e
  if [ -n "$java_home" ]; then
    (cd "$run_dir" && \
      JAVA_HOME="$java_home" \
      PATH="$install_dir:$java_home/bin:$PATH" \
      "${local_cmd[@]}" "$local_conf" --jar "$install_dir/emv.jar" \
        --rule "$r" --java_args "$heap") >"$log" 2>&1
  else
    (cd "$run_dir" && \
      PATH="$install_dir:$PATH" \
      "${local_cmd[@]}" "$local_conf" --jar "$install_dir/emv.jar" \
        --rule "$r" --java_args "$heap") >"$log" 2>&1
  fi
  status=$?
  set -e

  if ! grep -m 8 -E 'Verified|Violated|Timeout|ERROR|Error' "$log" \
      | sed "s|^|[$r] |"; then
    tail -8 "$log" | sed "s|^|[$r] |"
  fi
  if [ "$status" -ne 0 ]; then
    echo "[$r] local prover exited $status; full log: $log" >&2
  fi
  return "$status"
}

overall=0
if [ "$jobs" -le 1 ]; then
  for r in "${rules[@]}"; do
    echo "=== $name --rule $r"
    run_one "$r" &
    active_pids=("$!")
    wait "${active_pids[0]}" || overall=1
    active_pids=()
  done
else
  for r in "${rules[@]}"; do
    if [ "${#active_pids[@]}" -ge "$jobs" ]; then
      wait "${active_pids[0]}" || overall=1
      active_pids=("${active_pids[@]:1}")
    fi
    echo "=== $name --rule $r (parallel)"
    run_one "$r" &
    active_pids+=("$!")
  done
  for pid in "${active_pids[@]}"; do wait "$pid" || overall=1; done
  active_pids=()
fi

exit "$overall"
