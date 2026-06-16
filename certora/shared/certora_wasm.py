"""Resolve prebuilt Certora WASM under artifacts/wasm/certora/."""

from __future__ import annotations

import os
from pathlib import Path


def certora_wasm_path(package: str, root_dir: str | Path) -> Path:
    root = Path(root_dir)
    env = os.environ.get("CERTORA_WASM_DIR")
    base = Path(env) if env else root / "artifacts" / "wasm" / "certora"
    return base / f"{package}.wasm"