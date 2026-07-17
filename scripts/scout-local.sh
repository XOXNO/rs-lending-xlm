#!/usr/bin/env bash
# Local Scout scan → SARIF for the IDE (SARIF Viewer: ms-sarifvscode.sarif-viewer).
#
# Matches CI: pinned scout-audit rev from .github/workflows/scout.yml (cached via
# --scout-source); patched tree copy like .github/scripts/run_scout.sh (production
# manifests untouched).
#
# Usage:
#   scripts/scout-local.sh                       # all contracts
#   scripts/scout-local.sh contracts/controller  # one crate
#
# Output: target/scout-audit/<crate>.sarif
#
# First run clones + builds detectors (minutes); later runs reuse the cache.
# Bumping the pin in scout.yml re-clones automatically.
# SCOUT_CACHE=/path overrides cache (e.g. local scout-audit checkout).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

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
  # Native --output-format sarif is broken in the pinned rev (0-byte file);
  # emit JSON and convert. JSON file_path is already repo-relative.
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
