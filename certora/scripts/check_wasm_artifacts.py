#!/usr/bin/env python3
"""Ensure Certora conf files point at existing prebuilt WASM artifacts."""

from __future__ import annotations

import json
import sys
from pathlib import Path

from focused_wasm import PACKAGES, target_by_artifact, target_for_conf
from write_wasm_manifest import CERTORA_INPUTS, input_fingerprint, sha256

ROOT = Path(__file__).resolve().parents[2]
CERTORA_ROOT = ROOT / "certora"
TARGETS = target_by_artifact()
REQUIRED = {
    artifact: ROOT / "artifacts" / "wasm" / "certora" / artifact
    for artifact in TARGETS
}
MANIFEST = ROOT / "artifacts" / "wasm" / "manifest.json"


def main() -> int:
    missing = [path for path in REQUIRED.values() if not path.is_file()]
    if missing:
        print("Certora WASM artifacts missing:", file=sys.stderr)
        for path in missing:
            print(f"  {path.relative_to(ROOT)}", file=sys.stderr)
        print("Run: make certora-wasm", file=sys.stderr)
        return 1

    if not MANIFEST.is_file():
        print("Certora WASM manifest missing; run: make certora-wasm", file=sys.stderr)
        return 1

    manifest = json.loads(MANIFEST.read_text())
    certora_manifest = manifest.get("certora", {})
    provenance_errors: list[str] = []
    for artifact, wasm in REQUIRED.items():
        entry = certora_manifest.get(wasm.name)
        if not isinstance(entry, dict):
            provenance_errors.append(f"{wasm.name}: missing manifest entry")
            continue
        actual_hash = sha256(wasm)
        if entry.get("sha256") != actual_hash:
            provenance_errors.append(
                f"{wasm.name}: manifest sha256 {entry.get('sha256')} != {actual_hash}"
            )
        build = entry.get("build")
        if not isinstance(build, dict):
            provenance_errors.append(f"{wasm.name}: missing build provenance")
            continue
        expected_inputs = CERTORA_INPUTS[artifact]
        source_hash, file_count = input_fingerprint(expected_inputs)
        if build.get("source_inputs") != list(expected_inputs):
            provenance_errors.append(f"{wasm.name}: source input declaration drift")
        if build.get("source_input_sha256") != source_hash:
            provenance_errors.append(f"{wasm.name}: stale source input hash")
        if build.get("source_input_files") != file_count:
            provenance_errors.append(f"{wasm.name}: source input file-count drift")
        if build.get("cargo_features") != TARGETS[artifact].cargo_features:
            provenance_errors.append(f"{wasm.name}: focused feature provenance drift")
        if build.get("stellar_optimize") is not False:
            provenance_errors.append(f"{wasm.name}: optimizer provenance must be false")

    if provenance_errors:
        print("Certora WASM provenance drift:", file=sys.stderr)
        for line in provenance_errors:
            print(f"  {line}", file=sys.stderr)
        print("Run: make certora-wasm", file=sys.stderr)
        return 1

    bad_refs: list[str] = []
    for layer in PACKAGES:
        confs_dir = CERTORA_ROOT / layer / "confs"
        for conf in sorted(confs_dir.glob("*.conf")):
            target = target_for_conf(conf, layer)
            data = json.loads(conf.read_text())
            files = data.get("files", [])
            if files != [target.conf_relative_wasm]:
                bad_refs.append(f"{conf.relative_to(ROOT)}: files={files!r}")
            if data.get("cargo_features") != target.cargo_features:
                bad_refs.append(
                    f"{conf.relative_to(ROOT)}: cargo_features="
                    f"{data.get('cargo_features')!r}"
                )
            if "build_script" in data:
                bad_refs.append(f"{conf.relative_to(ROOT)}: still has build_script")

    if bad_refs:
        print("Conf focused-WASM provenance drift:", file=sys.stderr)
        for line in bad_refs:
            print(f"  {line}", file=sys.stderr)
        print("Run: python3 certora/scripts/sync_wasm_conf.py", file=sys.stderr)
        return 1

    print("certora wasm artifacts ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
