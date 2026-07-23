#!/usr/bin/env python3
"""Point Certora conf files at prebuilt WASM under artifacts/wasm/certora/."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from focused_wasm import PACKAGES, target_for_conf

ROOT = Path(__file__).resolve().parents[2]
CERTORA_ROOT = ROOT / "certora"


def patch_conf(conf: Path, layer: str) -> bool:
    data = json.loads(conf.read_text())
    target = target_for_conf(conf, layer)
    changed = False

    if data.pop("build_script", None) is not None:
        changed = True

    if data.get("files") != [target.conf_relative_wasm]:
        data["files"] = [target.conf_relative_wasm]
        changed = True

    if data.get("cargo_features") != target.cargo_features:
        data["cargo_features"] = target.cargo_features
        changed = True

    if changed:
        conf.write_text(json.dumps(data, indent=4) + "\n")
    return changed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="report drift without modifying conf files",
    )
    args = parser.parse_args()

    changed = 0
    for layer in PACKAGES:
        confs_dir = CERTORA_ROOT / layer / "confs"
        if not confs_dir.is_dir():
            print(f"missing conf dir: {confs_dir}", file=sys.stderr)
            return 1
        for conf in sorted(confs_dir.glob("*.conf")):
            data = json.loads(conf.read_text())
            target = target_for_conf(conf, layer)
            drifted = (
                "build_script" in data
                or data.get("files") != [target.conf_relative_wasm]
                or data.get("cargo_features") != target.cargo_features
            )
            if drifted and args.check:
                changed += 1
                print(f"drift: {conf.relative_to(ROOT)}", file=sys.stderr)
            elif drifted and patch_conf(conf, layer):
                changed += 1
                print(f"updated {conf.relative_to(ROOT)}")
    if args.check:
        if changed:
            print(f"{changed} conf file(s) need sync", file=sys.stderr)
            return 1
        print("OK: all conf files use canonical focused WASM paths and features")
    else:
        print(f"patched {changed} conf file(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
