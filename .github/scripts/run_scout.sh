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

patch_soroban_sdk_macros() {
  local version cargo_home source_dir vendor_dir
  version="$(awk '
    $0 == "[[package]]" { found = 0 }
    $1 == "name" && $3 == "\"soroban-sdk-macros\"" { found = 1 }
    found && $1 == "version" { gsub(/"/, "", $3); print $3; exit }
  ' "$work_dir/Cargo.lock")"

  if [ -z "$version" ]; then
    echo "Could not determine soroban-sdk-macros version from Cargo.lock" >&2
    exit 1
  fi

  cargo fetch --manifest-path "$work_dir/Cargo.toml" --locked >/dev/null

  cargo_home="${CARGO_HOME:-$HOME/.cargo}"
  source_dir="$(find "$cargo_home/registry/src" -type d -name "soroban-sdk-macros-$version" -print -quit)"
  if [ -z "$source_dir" ]; then
    echo "Could not find soroban-sdk-macros $version in Cargo registry" >&2
    exit 1
  fi

  vendor_dir="$work_dir/vendor/soroban-sdk-macros"
  mkdir -p "$work_dir/vendor"
  cp -R "$source_dir" "$vendor_dir"
  perl -0pi -e 's/let safe_len = docs\.floor_char_boundary\(max\);/let safe_len = if max >= docs.len() {\n        docs.len()\n    } else {\n        let mut end = max;\n        while !docs.is_char_boundary(end) {\n            end -= 1;\n        }\n        end\n    };/g' \
    "$vendor_dir/src/doc.rs"

  cat >> "$work_dir/Cargo.toml" <<'TOML'

[patch.crates-io]
soroban-sdk-macros = { path = "vendor/soroban-sdk-macros" }
TOML
}

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
patch_soroban_sdk_macros

# Detectors that are false positives by construction for this protocol, suppressed
# via Scout's --exclude (comma-separated). Deliberately NOT a .scout-audit/config.yaml:
# loading a config file makes Scout adopt the config's output_format and ignore
# --output-format, which silently corrupts non-md output (SCOUT_OUTPUT_FORMAT=json would
# write Markdown into .json files). --exclude suppresses without touching the format.
#   - integer-overflow-or-underflow: [profile.release] sets overflow-checks = true +
#     panic = abort, so every overflow traps -> reverts (non-exploitable).
#   - dos-unexpected-revert-with-storage: supply/borrow/withdraw are intentionally
#     permissionless with per-user-keyed storage; the "storage op without require_auth
#     in this fn = DoS" model does not represent per-user keys / SAC-transfer auth.
scout_exclude="integer-overflow-or-underflow,dos-unexpected-revert-with-storage"

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
