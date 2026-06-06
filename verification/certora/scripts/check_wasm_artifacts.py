#!/usr/bin/env python3
"""Ensure Certora conf files point at existing prebuilt WASM artifacts."""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CERTORA_ROOT = ROOT / "verification" / "certora"
REQUIRED = {
    "common": ROOT / "artifacts" / "wasm" / "certora" / "common.wasm",
    "pool": ROOT / "artifacts" / "wasm" / "certora" / "pool.wasm",
    "controller": ROOT / "artifacts" / "wasm" / "certora" / "controller.wasm",
}


def main() -> int:
    missing = [path for path in REQUIRED.values() if not path.is_file()]
    if missing:
        print("Certora WASM artifacts missing:", file=sys.stderr)
        for path in missing:
            print(f"  {path.relative_to(ROOT)}", file=sys.stderr)
        print("Run: make certora-wasm", file=sys.stderr)
        return 1

    bad_refs: list[str] = []
    for layer in REQUIRED:
        confs_dir = CERTORA_ROOT / layer / "confs"
        expected = f"../../../../artifacts/wasm/certora/{layer}.wasm"
        for conf in sorted(confs_dir.glob("*.conf")):
            data = json.loads(conf.read_text())
            files = data.get("files", [])
            if files != [expected]:
                bad_refs.append(f"{conf.relative_to(ROOT)}: files={files!r}")
            if "build_script" in data:
                bad_refs.append(f"{conf.relative_to(ROOT)}: still has build_script")

    if bad_refs:
        print("Conf WASM path drift:", file=sys.stderr)
        for line in bad_refs:
            print(f"  {line}", file=sys.stderr)
        print("Run: python3 verification/certora/scripts/sync_wasm_conf.py", file=sys.stderr)
        return 1

    print("certora wasm artifacts ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())