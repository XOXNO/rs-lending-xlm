#!/usr/bin/env bash
# Offline test for install-stellar-cli.sh. A fake curl serves a local tarball.
# Usage: bash .github/scripts/test_install_stellar_cli.sh [installer]

set -euo pipefail

installer="${1:-$(dirname "$0")/install-stellar-cli.sh}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

mkdir -p "$work/fakebin" "$work/pkg" "$work/home/.local/bin"
printf '#!/bin/sh\necho "stellar 28.0.0 (served)"\n' >"$work/pkg/stellar"
printf '#!/bin/sh\necho "stellar 28.0.0 (planted)"\n' >"$work/home/.local/bin/stellar"
chmod +x "$work/pkg/stellar" "$work/home/.local/bin/stellar"
tar -czf "$work/good.tar.gz" -C "$work/pkg" stellar
printf 'not a release tarball' >"$work/bad.tar.gz"
cat >"$work/fakebin/curl" <<'EOF'
#!/bin/sh
while [ $# -gt 0 ]; do [ "$1" = "-o" ] && cp "$SERVE" "$2"; shift; done
EOF
chmod +x "$work/fakebin/curl"

if command -v sha256sum >/dev/null 2>&1; then
  good_sha="$(sha256sum "$work/good.tar.gz")"
else
  good_sha="$(shasum -a 256 "$work/good.tar.gz")"
fi
sed -E "s/[0-9a-f]{64}/${good_sha%% *}/g" "$installer" >"$work/installer-good-digest.sh"

run() {
  env -u RUNNER_TEMP -u GITHUB_PATH HOME="$work/home" PATH="$work/fakebin:$PATH" "$@" >"$work/out" 2>&1
}
fail() {
  echo "FAIL: $1" >&2
  cat "$work/out" >&2
  exit 1
}

if run SERVE="$work/bad.tar.gz" bash "$installer"; then
  fail "a planted binary or an unverified download was accepted"
fi
grep -q "SHA-256 mismatch" "$work/out" || fail "no digest mismatch reported"

if run SERVE="$work/good.tar.gz" STELLAR_VERSION=0.0.0 bash "$installer"; then
  fail "a version with no pinned digest was installed"
fi

mkdir -p "$work/runner-temp"
run SERVE="$work/good.tar.gz" RUNNER_TEMP="$work/runner-temp" GITHUB_PATH="$work/github-path" \
  bash "$work/installer-good-digest.sh" || fail "a tarball with the pinned digest was rejected"
grep -q "(served)" "$work/out" || fail "the installed binary is not the verified one"
[ "$(cat "$work/github-path")" = "$work/runner-temp/stellar-cli" ] || fail "GITHUB_PATH does not name the per-job dir"
grep -q "(planted)" "$work/home/.local/bin/stellar" || fail "the persistent home was written in CI mode"

echo "install-stellar-cli: all checks passed"
