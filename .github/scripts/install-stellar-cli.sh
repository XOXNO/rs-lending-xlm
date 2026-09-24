#!/usr/bin/env bash

set -euo pipefail

STELLAR_VERSION="${STELLAR_VERSION:-28.0.0}"
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux) TARGET="x86_64-unknown-linux-gnu" ;;
  Darwin)
    if [ "$arch" = "arm64" ] || [ "$arch" = "aarch64" ]; then
      TARGET="aarch64-apple-darwin"
    else
      TARGET="x86_64-apple-darwin"
    fi
    ;;
  *)
    echo "Unsupported OS for prebuilt stellar-cli: $os" >&2
    exit 1
    ;;
esac

# SHA-256 of each release tarball, as the GitHub release asset `digest` reports it.
case "${STELLAR_VERSION}/${TARGET}" in
  28.0.0/x86_64-unknown-linux-gnu) SHA256="207544486734fccb4df1afc4a7745478f9f1e21688b2f9506f0ef36f60ce3fdc" ;;
  28.0.0/aarch64-apple-darwin) SHA256="416483409db89d9bf58163023d2c9d67cda24a5645042a98c0b8c1378cfc659d" ;;
  28.0.0/x86_64-apple-darwin) SHA256="9408343ed9cad961e529b3fdcd124ddc38fe1fa4766098e3c109d78e8d847305" ;;
  *)
    echo "No pinned SHA-256 for stellar-cli ${STELLAR_VERSION} (${TARGET})" >&2
    exit 1
    ;;
esac

if [ -n "${RUNNER_TEMP:-}" ]; then
  BIN_DIR="$RUNNER_TEMP/stellar-cli"
else
  BIN_DIR="$HOME/.local/bin"
fi
STELLAR_BIN="$BIN_DIR/stellar"
URL="https://github.com/stellar/stellar-cli/releases/download/v${STELLAR_VERSION}/stellar-cli-${STELLAR_VERSION}-${TARGET}.tar.gz"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Installing stellar-cli v${STELLAR_VERSION} (${TARGET})..."
curl -fsSL "$URL" -o "$tmp/stellar-cli.tar.gz"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp/stellar-cli.tar.gz")"
else
  actual="$(shasum -a 256 "$tmp/stellar-cli.tar.gz")"
fi
actual="${actual%% *}"
if [ "$actual" != "$SHA256" ]; then
  echo "stellar-cli SHA-256 mismatch: expected $SHA256, got $actual" >&2
  exit 1
fi

mkdir -p "$tmp/x" "$BIN_DIR"
tar -xzf "$tmp/stellar-cli.tar.gz" -C "$tmp/x"
mv -f "$tmp/x/stellar" "$STELLAR_BIN"
chmod +x "$STELLAR_BIN"

if [ -n "${GITHUB_PATH:-}" ]; then
  echo "$BIN_DIR" >>"$GITHUB_PATH"
fi

"$STELLAR_BIN" --version
