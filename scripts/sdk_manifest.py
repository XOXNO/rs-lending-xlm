#!/usr/bin/env python3
"""Package the WASM bundle that xoxno-contract-sdk embeds.

Copies the optimized builds, which keep their contract spec docs and error
enums, and the test oracle mocks into OUT_DIR as sdk-<name>.wasm, and writes
OUT_DIR/sdk-manifest.json. Each contract's code, with every custom section
removed, must equal the code of its stripped deploy artifact; the manifest
records both hashes.

Run after `make wasm-size-check`. Usage: sdk_manifest.py OUT_DIR
"""
import hashlib
import json
import pathlib
import sys

from strip_spec_docs import read_leb128

ROOT = pathlib.Path(__file__).resolve().parent.parent
OPTIMIZED = ROOT / "target" / "optimized"
DEPLOY = ROOT / "artifacts" / "wasm" / "deploy"
RELEASE = ROOT / "target" / "wasm32v1-none" / "release"

CONTRACTS = ["controller", "pool", "position_nft", "price_aggregator", "governance"]
MOCKS = {"mock_reflector": "mock_oracle", "mock_redstone": "mock_redstone"}


def code_hash(data: bytes) -> str:
    """SHA-256 of the module with every custom section (id 0) removed."""
    out = bytearray(data[:8])
    pos = 8
    while pos < len(data):
        start = pos
        size, payload = read_leb128(data, pos + 1)
        pos = payload + size
        if data[start] != 0:
            out += data[start:pos]
    return hashlib.sha256(bytes(out)).hexdigest()


def main() -> None:
    out = pathlib.Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    manifest = {}
    for name in CONTRACTS:
        data = (OPTIMIZED / f"{name}.wasm").read_bytes()
        deploy = (DEPLOY / f"{name}.wasm").read_bytes()
        if code_hash(data) != code_hash(deploy):
            sys.exit(f"sdk_manifest: {name}: optimized build and deploy artifact have different code")
        (out / f"sdk-{name}.wasm").write_bytes(data)
        manifest[name] = {
            "sha256": hashlib.sha256(data).hexdigest(),
            "code_sha256": code_hash(data),
            "deploy_sha256": hashlib.sha256(deploy).hexdigest(),
        }
    for sdk_name, crate_name in MOCKS.items():
        data = (RELEASE / f"{crate_name}.wasm").read_bytes()
        (out / f"sdk-{sdk_name}.wasm").write_bytes(data)
        manifest[sdk_name] = {"sha256": hashlib.sha256(data).hexdigest()}
    (out / "sdk-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"sdk_manifest: wrote {len(manifest)} files to {out}")


if __name__ == "__main__":
    main()
