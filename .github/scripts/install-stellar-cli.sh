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

BIN_DIR="$HOME/.local/bin"
STELLAR_BIN="$BIN_DIR/stellar"
URL="https://github.com/stellar/stellar-cli/releases/download/v${STELLAR_VERSION}/stellar-cli-${STELLAR_VERSION}-${TARGET}.tar.gz"

version_ok() {
  "$1" --version 2>/dev/null | grep -qE "^stellar ${STELLAR_VERSION}"
}

if [ -x "$STELLAR_BIN" ] && version_ok "$STELLAR_BIN"; then
  echo "stellar-cli v${STELLAR_VERSION} already present."
elif command -v stellar >/dev/null 2>&1 && version_ok stellar; then
  echo "stellar-cli v${STELLAR_VERSION} already present."
else
  echo "Installing stellar-cli v${STELLAR_VERSION} (${TARGET})..."
  mkdir -p "$BIN_DIR"
  curl -fsSL "$URL" | tar -xz -C "$BIN_DIR"
  chmod +x "$STELLAR_BIN"
fi

if [ -n "${GITHUB_PATH:-}" ]; then
  echo "$BIN_DIR" >>"$GITHUB_PATH"
else
  export PATH="$BIN_DIR:$PATH"
fi

if [ -x "$STELLAR_BIN" ]; then
  "$STELLAR_BIN" --version
elif command -v stellar >/dev/null 2>&1; then
  stellar --version
else
  echo "stellar CLI not found after install step" >&2
  exit 1
fi
