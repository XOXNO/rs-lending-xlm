#!/usr/bin/env bash
# Build one harness feature into a Wasm with the pinned builder image.
# Usage: build-wasm.sh <crate-dir relative to repo root> <feature> <out-dir>
# Prints the Wasm SHA-256. The repository is mounted read-only.
set -euo pipefail

here=$(cd -- "$(dirname -- "$0")/.." && pwd)
repo=$(cd -- "$here/../.." && pwd)
crate=$1 feature=$2 out=$3
eval "$("$here/scripts/build-images.sh" print-tags)"
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
cache=${KOMET_PIPELINE_CACHE:-$repo/target-komet/pipeline-cache}
mkdir -p "$cache/cargo" "$cache/target"

docker run --rm --network=host \
  -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/cache/cargo -e CARGO_TARGET_DIR=/cache/target -e HOME=/cache \
  -v "$repo:/src:ro" -v "$cache:/cache" -v "$out:/out" \
  -w /src "$WASM_IMAGE" \
  stellar contract build --locked --manifest-path "$crate/Cargo.toml" \
    --no-default-features --features "$feature" --out-dir /out >&2
wasm=$(ls "$out"/*.wasm)
test "$(echo "$wasm" | wc -l)" -eq 1
sha256sum "$wasm" | cut -d' ' -f1
