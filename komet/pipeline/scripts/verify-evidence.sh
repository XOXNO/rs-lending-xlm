#!/usr/bin/env bash
# Re-check every committed Komet server evidence directory without re-proving.
# Usage: verify-evidence.sh            (REBUILD=1 also rebuilds each harness Wasm)
#
# For each komet/**/server*/ directory holding SHA256SUMS + summary.json:
#   1. sha256sum -c SHA256SUMS (records, logs, graphs.tar.zst);
#   2. unpack graphs.tar.zst and run scripts/check-proof.py on every graph with
#      its recorded expectation (pass / fail / reject + expected error codes
#      from proofs.json) inside the pinned prover image; the status, node and
#      rewrite-step counts must equal the recorded summary;
#   3. with REBUILD=1, rebuild the harness Wasm and compare with wasm.sha256.
# Then every Lean file is re-checked with --trust=0 (scripts/check-lean.sh).
# Prints one line per claim; exits non-zero on any mismatch.
set -euo pipefail

here=$(cd -- "$(dirname -- "$0")/.." && pwd)
repo=$(cd -- "$here/../.." && pwd)
eval "$("$here/scripts/build-images.sh" print-tags)"
if ! docker image inspect "$PROVER_IMAGE" >/dev/null 2>&1; then
  echo "prover image $PROVER_IMAGE not found: build it with komet/pipeline/scripts/build-images.sh prover" >&2
  echo "(several hours on a cold cache; the tag is a hash of pins.env and the Dockerfiles)" >&2
  exit 2
fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fail=0

mapfile -t dirs < <(find "$repo/komet" -type f -name summary.json -path '*/server*/summary.json' \
  -not -path '*/target*' -printf '%h\n' | sort)
for d in "${dirs[@]}"; do
  rel=${d#"$repo"/}
  if ! (cd "$d" && sha256sum --quiet -c SHA256SUMS); then
    echo "FAIL $rel: SHA256SUMS mismatch"; fail=1; continue
  fi
  slot="$work/$(echo "$rel" | tr '/' '_')"
  mkdir -p "$slot"
  zstd -q -dc "$d/graphs.tar.zst" | tar -xf - -C "$slot"
  cp "$d/summary.json" "$slot/summary.json"
  if [ "${REBUILD:-0}" = 1 ]; then
    name=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))[0]['proof'])" "$d/summary.json")
    read -r crate feature < <(python3 - "$here/proofs.json" "$name" <<'PY'
import json, sys
p = next(p for p in json.load(open(sys.argv[1]))['proofs'] if p['name'] == sys.argv[2])
print(p['crate'], p['feature'] or '-')
PY
)
    [ "$feature" = - ] && feature=
    sha=$("$here/scripts/build-wasm.sh" "$crate" "$feature" "$work/wasm-$name" 2>/dev/null)
    if [ "$sha" != "$(cut -d' ' -f1 "$d/wasm.sha256")" ]; then
      echo "FAIL $rel: rebuilt Wasm $sha != recorded $(cat "$d/wasm.sha256")"; fail=1
    else
      echo "OK   $rel: rebuilt Wasm matches $sha"
    fi
  fi
done

# One prover container re-checks every unpacked graph.
cp "$here/proofs.json" "$work/proofs.json"
docker run -i --rm --network=none --memory=4g --memory-swap=4g -u "$(id -u):$(id -g)" -e HOME=/tmp \
  -v "$work:/w" -v "$here/scripts:/scripts:ro" "$PROVER_IMAGE" python3 - <<'PY' || fail=1
import json, os, subprocess, sys
from pathlib import Path
codes = {t['id']: t.get('codes', []) for p in json.load(open('/w/proofs.json'))['proofs'] for t in p['tests']}
bad = 0
for summary in sorted(Path('/w').glob('*/summary.json')):
    slot = summary.parent
    for row in json.loads(summary.read_text()):
        tid = row['id']
        env = dict(os.environ, KOMET_REJECT_CODES=','.join(map(str, codes.get(tid, []))))
        out = slot / f'{tid}.recheck.json'
        r = subprocess.run(['python3', '/scripts/check-proof.py', str(slot / tid / 'proof'), tid,
                            row['expect'], str(out)], env=env, capture_output=True, text=True)
        c = json.loads(out.read_text()) if out.exists() else {}
        same = (r.returncode == 0 and c.get('verdict') == 'OK' and c.get('status') == row['status']
                and c.get('nodes') == row['nodes'] and c.get('rewrite_steps') == row['rewrite_steps'])
        bad += not same
        print(f"{'OK  ' if same else 'FAIL'} {slot.name}: {tid} expect={row['expect']} "
              f"status={c.get('status')} nodes={c.get('nodes')} steps={c.get('rewrite_steps')}"
              + ('' if same else f" recorded={row['status']}/{row['nodes']}/{row['rewrite_steps']} {r.stderr[-200:]}"))
sys.exit(1 if bad else 0)
PY

if "$here/scripts/check-lean.sh" "$work/lean.log" > /dev/null; then
  echo "OK   Lean: $(tail -1 "$work/lean.log")"
else
  echo "FAIL Lean: $(tail -1 "$work/lean.log")"; fail=1
fi
[ "$fail" = 0 ] && echo "komet-verify: all evidence re-checked" || echo "komet-verify: MISMATCH"
exit "$fail"
