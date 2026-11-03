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
# Summary of the completed, checked tests only (a run may have been stopped
# before writing summary.json; unfinished tests are not packed).
python3 - "$src" "$name" "$dest/summary.json" <<'PY'
import json, sys
from pathlib import Path
rows = []
for check in sorted(Path(sys.argv[1]).glob('test_*/check.json')):
    c = json.loads(check.read_text())
    run = json.loads((check.parent / 'run.json').read_text())
    if run['exit_code'] in (137, 143):  # stopped by the operator: not a result
        continue
    rows.append({'proof': sys.argv[2], 'id': c['id'], 'expect': c['expect'], 'status': c['status'],
                 'verdict': c['verdict'], 'exit_code': run['exit_code'], 'wall_seconds': run['wall_seconds'],
                 'smt_timeout_ms': run.get('smt_timeout_ms', 5000), 'nodes': c['nodes'],
                 'rewrite_steps': c['rewrite_steps'], 'errors': c['errors']})
json.dump(rows, open(sys.argv[3], 'w'), indent=1)
PY
for t in "$src"/test_*/; do
  [ -f "$t/check.json" ] || continue
  grep -q '"exit_code": 1[34][37]' "$t/run.json" && continue
  id=$(basename "$t")
  mkdir -p "$dest/$id"
  cp "$t/run.json" "$t/check.json" "$t/stdout.log" "$t/stderr.log" "$dest/$id/"
done
(cd "$src" && tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
  -cf - $(cd "$src" && for t in test_*/; do [ -f "$t/check.json" ] && ! grep -q '"exit_code": 1[34][37]' "$t/run.json" && echo "${t%/}/proof"; done) | zstd -q -19 -T0 -o "$dest/graphs.tar.zst" -f)
(cd "$dest" && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS)
du -sh "$dest"
