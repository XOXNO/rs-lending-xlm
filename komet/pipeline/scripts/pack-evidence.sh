#!/usr/bin/env bash
# Copy one proof's server evidence from a run-proofs.sh bundle into the repo.
# Usage: pack-evidence.sh <evidence-dir> <proof-name> <dest-dir>
# dest receives: context/toolchain/semantics records, per-test run.json,
# check.json and logs, the Wasm SHA-256, and graphs.tar.zst with every proof
# graph (including inputs.json), plus a SHA256SUMS over everything.
set -euo pipefail
ev=$(cd -- "$1" && pwd) name=$2 dest=$3
src="$ev/$name"
test -d "$src" || { echo "no $src" >&2; exit 2; }
mkdir -p "$dest"
dest=$(cd -- "$dest" && pwd)
cp "$ev/context.txt" "$ev/toolchain.txt" "$ev/semantics.sha256" "$ev/pins.env" "$dest/"
cp "$src/wasm.sha256" "$dest/"
python3 - "$ev/summary.json" "$name" "$dest/summary.json" <<'PY'
import json, sys
rows = [r for r in json.load(open(sys.argv[1])) if r['proof'] == sys.argv[2]]
json.dump(rows, open(sys.argv[3], 'w'), indent=1)
PY
for t in "$src"/test_*/; do
  id=$(basename "$t")
  mkdir -p "$dest/$id"
  cp "$t/run.json" "$t/check.json" "$t/stdout.log" "$t/stderr.log" "$dest/$id/"
done
(cd "$src" && tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
  -cf - test_*/proof | zstd -q -19 -T0 -o "$dest/graphs.tar.zst" -f)
(cd "$dest" && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS)
du -sh "$dest"
