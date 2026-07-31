#!/usr/bin/env bash

set -euo pipefail

contracts=(
  contracts/pool/Cargo.toml
  contracts/controller/Cargo.toml
  contracts/governance/Cargo.toml
  contracts/defindex-strategy/Cargo.toml
  mock/flash-loan-receiver/Cargo.toml
  mock/mock-oracle/Cargo.toml
  mock/mock-redstone/Cargo.toml
)

format="${SCOUT_OUTPUT_FORMAT:-md}"
out_dir="${SCOUT_OUTPUT_DIR:-target/scout-audit}"
repo_root="$(pwd)"

out_dir_abs="$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$repo_root/$out_dir")"
case "$out_dir_abs" in
  "$repo_root"/*) ;;
  *)
    echo "Refusing to clean Scout output outside repository: $out_dir_abs" >&2
    exit 1
    ;;
esac

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT
rm -rf "$out_dir_abs"
mkdir -p "$out_dir_abs" "$HOME/.scout-audit/telemetry"
printf DONOTTRACK >"$HOME/.scout-audit/telemetry/user_id.txt"
export SOROBAN_SDK_BUILD_SYSTEM_SUPPORTS_SPEC_SHAKING_V2=1

tar --exclude './.git' --exclude './target' -cf - . | (cd "$work_dir" && tar -xf -)

find "$work_dir/contracts" "$work_dir/common" -name Cargo.toml -print0 |
  xargs -0 perl -0pi -e 's/crate-type = \["cdylib", "rlib"\]/crate-type = ["rlib"]/g'

scout_exclude="dos-unexpected-revert-with-storage"
scout_local_flags=()
if [ -n "${SCOUT_SOURCE_DIR:-}" ]; then
  scout_local_flags=(--scout-source "$SCOUT_SOURCE_DIR" --local-detectors "$SCOUT_SOURCE_DIR/nightly")
fi

incomplete=0
for manifest in "${contracts[@]}"; do
  crate="$(basename "$(dirname "$manifest")")"
  out="$out_dir_abs/$crate.$format"
  log="$out_dir_abs/$crate.log"
  echo "Running Scout on $manifest"
  if ! cargo scout-audit \
    --manifest-path "$work_dir/$manifest" \
    "${scout_local_flags[@]}" \
    --debug \
    --exclude "$scout_exclude" \
    --output-format "$format" \
    --output-path "$out" \
    -- --locked >"$log" 2>&1; then
    echo "Scout failed for $manifest; see $log" >&2
    incomplete=$((incomplete + 1))
    continue
  fi
  perl -0pi -e "s|\Q$work_dir\E|$repo_root|g" "$out" "$log"
  if grep -q "Compilation errors\\|report is incomplete" "$out"; then
    echo "Scout report for $manifest is incomplete; see $log" >&2
    incomplete=$((incomplete + 1))
  fi
done

echo "Scout reports written to $out_dir"
if [ "$incomplete" -gt 0 ]; then
  echo "Scout completed with $incomplete incomplete report(s)."
  [ "${SCOUT_STRICT:-0}" = "1" ] && exit 1
fi
