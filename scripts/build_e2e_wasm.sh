#!/usr/bin/env bash
# One canonical build for release and manual E2E. Fixtures never write candidates.
set -euo pipefail
mode="${1:-candidate}"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
# Cargo gives these variables precedence over build.rustflags. Pin both paths
# so a developer's inherited flags cannot change the candidate build.
unset CARGO_ENCODED_RUSTFLAGS
export RUSTFLAGS="-C link-arg=-zstack-size=16384"
export CARGO_BUILD_RUSTFLAGS="$RUSTFLAGS"
case "$mode" in
candidate)
    out=artifacts/wasm/deploy
    mkdir -p "$out" target/optimized
    for pkg in controller pool governance price-aggregator position-nft defindex-strategy swap-aggregator xoxno-oracle; do
        stellar contract build --package "$pkg" --optimize --out-dir target/optimized
        name="${pkg//-/_}"
        case "$pkg" in swap-aggregator) name=aggregator;; xoxno-oracle) name=xoxno-oracle-adapter;; esac
        if [ "$name" != "${pkg//-/_}" ]; then
            mv "target/optimized/${pkg//-/_}.wasm" "target/optimized/$name.wasm"
        fi
        python3 scripts/strip_spec_docs.py "target/optimized/$name.wasm" "$out/$name.wasm"
    done
    python3 tests/integration/artifacts.py create "$out"
    ;;
fixtures)
    out=artifacts/wasm/fixtures
    mkdir -p "$out"
    for pkg in mock-oracle mock-redstone flash-loan-receiver flash-position-receiver script-runner production-fixture; do
        stellar contract build --package "$pkg" --optimize --out-dir "$out"
    done
    ;;
*) echo "unknown build mode: $mode" >&2; exit 2;;
esac
