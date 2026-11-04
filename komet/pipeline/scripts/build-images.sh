#!/usr/bin/env bash
# Build the pinned Komet images.
# Usage: build-images.sh [all|prover|backend|semantics|wasm|print-tags]
#   backend   : kore-rpc-booster from HASKELL_BACKEND_COMMIT (docker/backend.Dockerfile)
#   semantics : Komet fork + compiled semantics at KOMET_COMMIT (docker/semantics.Dockerfile)
#   prover    : semantics + backend + Z3 (docker/prover.Dockerfile); builds missing inputs
#   wasm      : Rust 1.95.0 + Stellar CLI harness builder (docker/wasm.Dockerfile)
# Each tag is a hash of the pins and Dockerfile that define the image, so a
# changed pin yields a new image instead of silently reusing an old one.
set -euo pipefail

here=$(cd -- "$(dirname -- "$0")/.." && pwd)
docker_dir="$here/docker"
# shellcheck source=../pins.env
set -a; . "$here/pins.env"; set +a
what=${1:-all}

hash_of() { printf '%s\n' "$@" | sha256sum | cut -c1-16; }
df_text() { cat "$docker_dir/$1.Dockerfile"; }

BACKEND_IMAGE="komet-backend:$(hash_of "$K_IMAGE" "$HASKELL_BACKEND_REPO" "$HASKELL_BACKEND_DEPS_COMMIT" \
  "$HASKELL_BACKEND_COMMIT" "$STACK_VERSION" "$STACK_SHA256" "$(df_text backend)")"
SEMANTICS_IMAGE="komet-semantics:$(hash_of "$K_IMAGE" "$UV_VERSION" "$UV_SHA256" "$STELLAR_VERSION" \
  "$STELLAR_SHA256" "$KOMET_REPO" "$KOMET_COMMIT" "$KOMET_LEMMAS_SHA256" "$KOMET_PROOF_PY_SHA256" \
  "$(df_text semantics)")"
PROVER_IMAGE="komet-prover:$(hash_of "$BACKEND_IMAGE" "$SEMANTICS_IMAGE" "$Z3_VERSION" "$Z3_ZIP_SHA256" \
  "$Z3_BIN_SHA256" "$BOOSTER_FLAGS" "$(df_text prover)")"
WASM_IMAGE="komet-wasm:$(hash_of "$RUST_IMAGE" "$STELLAR_VERSION" "$STELLAR_SHA256" "$(df_text wasm)")"

have() { docker image inspect "$1" >/dev/null 2>&1; }

build() { # build <dockerfile-name> <tag> <context-dir> <build-arg names...>
  local file=$1 tag=$2 context=$3 name
  shift 3
  local args=()
  for name in "$@"; do args+=(--build-arg "$name=${!name}"); done
  # Network fetches (e.g. uv cloning pykwasm's nested git submodules) fail
  # transiently now and then; completed layers are cached, so retry the build.
  local attempt
  for attempt in 1 2 3; do
    docker build "${args[@]}" -t "$tag" -f "$docker_dir/$file.Dockerfile" "$context" && return 0
    echo "docker build $file failed (attempt $attempt)" >&2
    sleep 30
  done
  return 1
}

build_backend() {
  build backend "$BACKEND_IMAGE" "$docker_dir" K_IMAGE HASKELL_BACKEND_REPO HASKELL_BACKEND_DEPS_COMMIT \
    HASKELL_BACKEND_COMMIT STACK_VERSION STACK_SHA256
}
build_semantics() {
  build semantics "$SEMANTICS_IMAGE" "$docker_dir" K_IMAGE UV_VERSION UV_SHA256 STELLAR_VERSION STELLAR_SHA256 \
    KOMET_REPO KOMET_COMMIT KOMET_LEMMAS_SHA256 KOMET_PROOF_PY_SHA256
}
build_prover() {
  have "$BACKEND_IMAGE" || build_backend
  have "$SEMANTICS_IMAGE" || build_semantics
  build prover "$PROVER_IMAGE" "$docker_dir" BACKEND_IMAGE SEMANTICS_IMAGE Z3_VERSION Z3_ZIP_SHA256 Z3_BIN_SHA256 BOOSTER_FLAGS
  docker run --rm "$PROVER_IMAGE" cat /opt/toolchain.txt /opt/semantics.sha256
}
build_wasm() {
  build wasm "$WASM_IMAGE" "$docker_dir" RUST_IMAGE STELLAR_VERSION STELLAR_SHA256
}

case "$what" in
  print-tags)
    printf 'BACKEND_IMAGE=%s\nSEMANTICS_IMAGE=%s\nPROVER_IMAGE=%s\nWASM_IMAGE=%s\n' \
      "$BACKEND_IMAGE" "$SEMANTICS_IMAGE" "$PROVER_IMAGE" "$WASM_IMAGE"
    ;;
  backend) build_backend ;;
  semantics) build_semantics ;;
  prover) build_prover ;;
  wasm) build_wasm ;;
  all)
    have "$PROVER_IMAGE" || build_prover
    have "$WASM_IMAGE" || build_wasm
    ;;
  *) echo "unknown target $what" >&2; exit 2 ;;
esac
