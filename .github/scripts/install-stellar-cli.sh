#!/usr/bin/env bash
# Install (or verify) a pinned version of stellar-cli.
#
# - Idempotent: skips download if the exact version is already on PATH.
# - In GitHub Actions: appends $HOME/.local/bin to $GITHUB_PATH so later steps in the job see it.
# - Locally / via Makefile: updates PATH for the current shell when possible.
# - Cross-platform: selects the correct release asset for Linux (gnu) and macOS (darwin x86_64/aarch64).
# - Version is controlled by $STELLAR_VERSION (defaults to the CI-pinned version).
#
# Usage:
#   STELLAR_VERSION=27.0.0 bash .github/scripts/install-stellar-cli.sh
#   # or from Makefile (see target below)
set -euo pipefail

STELLAR_VERSION="${STELLAR_VERSION:-27.0.0}"

detect_target() {
  local os
  local arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Linux)
      echo "x86_64-unknown-linux-gnu"
      ;;
    Darwin)
      if [ "$arch" = "arm64" ] || [ "$arch" = "aarch64" ]; then
        echo "aarch64-apple-darwin"
      else
        echo "x86_64-apple-darwin"
      fi
      ;;
    *)
      echo "Unsupported OS for prebuilt stellar-cli: $os" >&2
      echo "Please install stellar CLI ${STELLAR_VERSION} manually from https://github.com/stellar/stellar-cli/releases" >&2
      exit 1
      ;;
  esac
}

TARGET="$(detect_target)"
BIN_DIR="$HOME/.local/bin"
STELLAR_BIN="$BIN_DIR/stellar"
TARBALL="stellar-cli-${STELLAR_VERSION}-${TARGET}.tar.gz"
URL="https://github.com/stellar/stellar-cli/releases/download/v${STELLAR_VERSION}/${TARBALL}"

need_install() {
  # If our canonical binary already exists and is the exact version, skip
  # (covers re-invocation in the same job after GITHUB_PATH update, or
  # repeated local calls, even if the dir is not yet on PATH).
  if [ -x "$STELLAR_BIN" ]; then
    if "$STELLAR_BIN" --version 2>/dev/null | grep -qE "^stellar ${STELLAR_VERSION}"; then
      return 1
    fi
  fi
  if ! command -v stellar >/dev/null 2>&1; then
    return 0
  fi
  # Match the leading "stellar X.Y.Z" exactly (the original CI check).
  if ! stellar --version 2>/dev/null | grep -qE "^stellar ${STELLAR_VERSION}"; then
    return 0
  fi
  return 1
}

if need_install; then
  echo "Installing stellar-cli v${STELLAR_VERSION} (${TARGET})..."
  mkdir -p "$BIN_DIR"
  curl -fsSL "$URL" | tar -xz -C "$BIN_DIR"
  chmod +x "$STELLAR_BIN"
else
  echo "stellar-cli v${STELLAR_VERSION} already present."
fi

# Make the binary visible to subsequent steps/jobs in GitHub Actions.
if [ -n "${GITHUB_PATH:-}" ]; then
  echo "$BIN_DIR" >> "$GITHUB_PATH"
else
  # Best-effort for local invocation from Makefile or shell.
  export PATH="$BIN_DIR:$PATH"
fi

# Final verification using the just-installed (or discovered) binary location when possible.
if [ -x "$STELLAR_BIN" ]; then
  "$STELLAR_BIN" --version
else
  command -v stellar >/dev/null 2>&1 && stellar --version || {
    echo "stellar CLI not found after install step" >&2
    exit 1
  }
fi
