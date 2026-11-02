#!/bin/sh
# Apply the Komet pyk patch to an installed kframework 7.1.337 `pyk` package.
# Usage: vendor/pyk-7.1.337/apply.sh <site-packages>/pyk
# Checks the stock file hashes first and the patched hashes afterwards.
set -eu
here=$(cd -- "$(dirname -- "$0")" && pwd)
pyk=$1
check() { # check <stock|patched>
  grep "  $1/" "$here/SHA256SUMS" | while read -r sum path; do
    echo "$sum  $pyk/${path#*/}"
  done | sha256sum -c -
}
check stock
patch -d "$pyk" -p1 --no-backup-if-mismatch < "$here/komet-pyk.patch"
check patched
