#!/usr/bin/env bash
# Local Scout scan -> SARIF for the IDE.
#
# Install the "SARIF Viewer" extension (ms-sarifvscode.sarif-viewer); findings
# then show inline in the editor plus a navigable Results panel.
#
# Same analysis as CI: detectors come from the pinned scout-audit rev referenced
# in .github/workflows/scout.yml (cloned into a local cache and passed via
# --scout-source, so detectors are never fetched ad-hoc), and the scan runs
# against a patched copy of the tree exactly like .github/scripts/run_scout.sh
# (production manifests are never modified).
#
# Usage:
#   scripts/scout-local.sh                       # scan all contracts
#   scripts/scout-local.sh contracts/controller  # scan one (fast IDE loop)
#
# Output: target/scout-audit/<crate>.sarif
#
# Notes:
#   - First run clones the pinned scout rev and builds the detector driver
#     (slow, minutes); later runs reuse the cache (seconds-ish).
#   - Bumping the pin in scout.yml makes this re-clone the new tag automatically.
#   - Override the cache location with SCOUT_CACHE=/path (e.g. point it at a
#     local scout-audit checkout to test unpushed detector changes).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# --- contracts to scan ---
if [ "$#" -gt 0 ]; then
  contracts=("$@")
else
  contracts=(
    contracts/pool
    contracts/controller
    contracts/governance
    contracts/defindex-strategy
    contracts/flash-loan-receiver
    mock/mock-oracle
    mock/mock-redstone
  )
fi

# --- resolve + cache the pinned scout-audit rev (matches CI) ---
workflow=".github/workflows/scout.yml"
pin="$(grep -oE '[A-Za-z0-9._/-]+/scout-audit@[^[:space:]"]+' "$workflow" | head -1)"
[ -n "$pin" ] || { echo "No scout-audit pin found in $workflow" >&2; exit 1; }
scout_repo="${pin%@*}"
scout_ref="${pin#*@}"
cache="${SCOUT_CACHE:-$HOME/.cache/scout-audit/${scout_repo//\//_}@${scout_ref}}"
if [ ! -d "$cache/.git" ]; then
  echo "Cloning $scout_repo@$scout_ref -> $cache"
  mkdir -p "$(dirname "$cache")"
  git clone --quiet --depth 1 --branch "$scout_ref" "https://github.com/$scout_repo.git" "$cache"
fi
echo "Detectors: $scout_repo@$scout_ref"

# --- patched scan copy (production manifests stay untouched) ---
out_dir="$repo_root/target/scout-audit"
mkdir -p "$out_dir"
work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT
tar --exclude './.git' --exclude './target' -cf - . | (cd "$work_dir" && tar -xf -)
find "$work_dir/contracts" "$work_dir/common" -name Cargo.toml -print0 \
  | xargs -0 perl -0pi -e 's/crate-type = \["cdylib", "rlib"\]/crate-type = ["rlib"]/g'

export SOROBAN_SDK_BUILD_SYSTEM_SUPPORTS_SPEC_SHAKING_V2=1
mkdir -p "$HOME/.scout-audit/telemetry"
printf DONOTTRACK > "$HOME/.scout-audit/telemetry/user_id.txt"

rc=0
jsons=()
for c in "${contracts[@]}"; do
  name="$(basename "$c")"
  out="$out_dir/$name.json"
  echo "Scout -> $c"
  # NOTE: scout's native `--output-format sarif` is broken in the pinned version
  # (0-byte file); emit JSON and convert below. file_path in the JSON is already
  # repo-relative, so no scan-copy path rewrite is needed.
  if cargo scout-audit \
      --manifest-path "$work_dir/$c/Cargo.toml" \
      --scout-source "$cache" \
      --local-detectors "$cache/nightly" \
      --exclude dos-unexpected-revert-with-storage \
      --output-format json \
      --output-path "$out" \
      -- --locked; then
    jsons+=("$out")
  else
    echo "  scout failed for $c" >&2
    rc=1
  fi
done

sarif="$out_dir/scout.sarif"
if [ "${#jsons[@]}" -gt 0 ]; then
  python3 "$repo_root/scripts/scout-sarif.py" --root "$repo_root" "${jsons[@]}" > "$sarif"
  echo
  echo "SARIF: $sarif"
  echo "Open with SARIF Viewer (Command Palette -> 'SARIF: Open SARIF Log' if not auto-loaded)."
fi
exit "$rc"
