#!/usr/bin/env python3
"""Write SHA-256 manifest for artifacts/wasm deploy and certora binaries."""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
DEPLOY_DIR = ROOT / "artifacts" / "wasm" / "deploy"
CERTORA_DIR = ROOT / "artifacts" / "wasm" / "certora"
MANIFEST = ROOT / "artifacts" / "wasm" / "manifest.json"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def tool_version(cmd: list[str]) -> str | None:
    try:
        return subprocess.check_output(cmd, text=True, stderr=subprocess.STDOUT).strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def section(dir_path: Path) -> dict[str, object]:
    files: dict[str, object] = {}
    for wasm in sorted(dir_path.glob("*.wasm")):
        files[wasm.name] = {
            "path": str(wasm.relative_to(ROOT)),
            "bytes": wasm.stat().st_size,
            "sha256": sha256(wasm),
        }
    return files


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--deploy", action="store_true")
    parser.add_argument("--certora", action="store_true")
    args = parser.parse_args()

    if not args.deploy and not args.certora:
        args.deploy = args.certora = True

    manifest: dict[str, object] = {}
    if MANIFEST.exists():
        manifest = json.loads(MANIFEST.read_text())

    manifest.setdefault("generated_at", datetime.now(timezone.utc).isoformat())
    manifest["toolchain"] = {
        "rustc": tool_version(["rustc", "--version"]),
        "stellar": tool_version(["stellar", "--version"]),
        "platform": platform.platform(),
    }

    if args.deploy:
        if not DEPLOY_DIR.is_dir():
            print(f"missing deploy dir: {DEPLOY_DIR}", file=sys.stderr)
            return 1
        manifest["deploy"] = section(DEPLOY_DIR)

    if args.certora:
        if not CERTORA_DIR.is_dir():
            print(f"missing certora dir: {CERTORA_DIR}", file=sys.stderr)
            return 1
        manifest["certora"] = section(CERTORA_DIR)

    MANIFEST.parent.mkdir(parents=True, exist_ok=True)
    MANIFEST.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"wrote {MANIFEST.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())