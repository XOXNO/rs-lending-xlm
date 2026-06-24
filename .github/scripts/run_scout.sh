#!/usr/bin/env bash
# Run Scout against each Soroban contract crate and write per-contract reports.
set -euo pipefail

contracts=(
  contracts/pool/Cargo.toml
  contracts/controller/Cargo.toml
  contracts/governance/Cargo.toml
  contracts/defindex-strategy/Cargo.toml
  contracts/flash-loan-receiver/Cargo.toml
  contracts/mock-oracle/Cargo.toml
  contracts/mock-redstone/Cargo.toml
)

format="${SCOUT_OUTPUT_FORMAT:-md}"
out_dir="${SCOUT_OUTPUT_DIR:-target/scout-audit}"
repo_root="$(pwd)"
out_dir_abs="$repo_root/$out_dir"
work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

case "$out_dir_abs" in
  "$repo_root"/*) ;;
  *)
    echo "Refusing to clean Scout output outside repository: $out_dir_abs" >&2
    exit 1
    ;;
esac

rm -rf "$out_dir_abs"
mkdir -p "$out_dir_abs" "$HOME/.scout-audit/telemetry"
printf DONOTTRACK > "$HOME/.scout-audit/telemetry/user_id.txt"
export SOROBAN_SDK_BUILD_SYSTEM_SUPPORTS_SPEC_SHAKING_V2=1

tar --exclude './.git' --exclude './target' -cf - . | (cd "$work_dir" && tar -xf -)

# Scout analyzes compiler lints against a scan copy.
# Production manifests stay unchanged.
find "$work_dir/contracts" "$work_dir/common" -name Cargo.toml -print0 \
  | xargs -0 perl -0pi -e 's/crate-type = \["cdylib", "rlib"\]/crate-type = ["rlib"]/g'

# Detectors that are false positives by construction for this protocol, suppressed
# via Scout's --exclude (comma-separated). Deliberately NOT a .scout-audit/config.yaml:
# loading a config file makes Scout adopt the config's output_format and ignore
# --output-format, which silently corrupts non-md output (SCOUT_OUTPUT_FORMAT=json would
# write Markdown into .json files). --exclude suppresses without touching the format.
#   - dos-unexpected-revert-with-storage: supply/borrow/withdraw are intentionally
#     permissionless with per-user-keyed storage; the "storage op without require_auth
#     in this fn = DoS" model does not represent per-user keys / SAC-transfer auth.
#     (The pinned detector is now per-user-key aware and silent on the 3 core
#     contracts, but still fires on the mock contracts — kept excluded pending review.)
# integer-overflow-or-underflow was dropped from this list: the pinned detector is now
# [profile.release] overflow-checks-aware and reports 0 findings across all 7 contracts.
scout_exclude="dos-unexpected-revert-with-storage"

incomplete=0
for manifest in "${contracts[@]}"; do
  crate="$(basename "$(dirname "$manifest")")"
  out="$out_dir_abs/$crate.$format"
  log="$out_dir_abs/$crate.log"
  echo "Running Scout on $manifest"
  if ! cargo scout-audit \
    --manifest-path "$work_dir/$manifest" \
    --debug \
    --exclude "$scout_exclude" \
    --output-format "$format" \
    --output-path "$out" \
    -- --locked > "$log" 2>&1; then
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
  if [ "${SCOUT_STRICT:-0}" = "1" ]; then
    exit 1
  fi
fi
