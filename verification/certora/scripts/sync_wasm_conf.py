#!/usr/bin/env python3
"""Point Certora conf files at prebuilt WASM under artifacts/wasm/certora/."""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CERTORA_ROOT = ROOT / "verification" / "certora"
WASM_REL = {
    "common": "../../../../artifacts/wasm/certora/common.wasm",
    "pool": "../../../../artifacts/wasm/certora/pool.wasm",
    "controller": "../../../../artifacts/wasm/certora/controller.wasm",
}


def patch_conf(conf: Path, layer: str) -> bool:
    data = json.loads(conf.read_text())
    wasm_path = WASM_REL[layer]
    changed = False

    if data.pop("build_script", None) is not None:
        changed = True

    if data.get("files") != [wasm_path]:
        data["files"] = [wasm_path]
        changed = True

    if changed:
        conf.write_text(json.dumps(data, indent=4) + "\n")
    return changed


def main() -> int:
    changed = 0
    for layer, _wasm in WASM_REL.items():
        confs_dir = CERTORA_ROOT / layer / "confs"
        if not confs_dir.is_dir():
            print(f"missing conf dir: {confs_dir}", file=sys.stderr)
            return 1
        for conf in sorted(confs_dir.glob("*.conf")):
            if patch_conf(conf, layer):
                changed += 1
                print(f"updated {conf.relative_to(ROOT)}")
    print(f"patched {changed} conf file(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())