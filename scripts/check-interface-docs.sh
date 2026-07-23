#!/usr/bin/env bash
# Fail if a public trait method in interfaces/* lacks a preceding /// doc line.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
fail=0
while IFS= read -r -d '' file; do
  python3 - "$file" <<'PY' || fail=1
import sys
from pathlib import Path
path = Path(sys.argv[1])
lines = path.read_text().splitlines()
bad = []
i = 0
while i < len(lines):
    line = lines[i]
    if line.lstrip().startswith("fn ") and "(" in line:
        # look back skipping attributes
        j = i - 1
        while j >= 0 and (lines[j].strip().startswith("#[") or lines[j].strip() == ""):
            j -= 1
        if j < 0 or not lines[j].lstrip().startswith("///"):
            # ignore if inside a non-trait context heuristically: require 'trait' seen before with no closing at col0
            bad.append((i + 1, line.strip()))
    i += 1
if bad:
    print(f"{path}:")
    for ln, text in bad:
        print(f"  L{ln}: missing /// before {text}")
    sys.exit(1)
PY
done < <(find "$ROOT/interfaces" -name '*.rs' -print0)

if [[ "$fail" -ne 0 ]]; then
  echo "Interface rustdoc check failed. See docs/reference/doc-style.md" >&2
  exit 1
fi
echo "OK: interface trait methods have preceding ///"
