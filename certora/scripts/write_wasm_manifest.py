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

from focused_wasm import target_by_artifact

ROOT = Path(__file__).resolve().parents[2]
DEPLOY_DIR = ROOT / "artifacts" / "wasm" / "deploy"
CERTORA_DIR = ROOT / "artifacts" / "wasm" / "certora"
MANIFEST = ROOT / "artifacts" / "wasm" / "manifest.json"

CERTORA_TARGETS = target_by_artifact()
CERTORA_INPUTS: dict[str, tuple[str, ...]] = {
    artifact: target.inputs for artifact, target in CERTORA_TARGETS.items()
}


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


def git_output(args: list[str]) -> str | None:
    return tool_version(["git", "-C", str(ROOT), *args])


def resolved_inputs(inputs: tuple[str, ...]) -> list[Path]:
    files: set[Path] = set()
    for relative in inputs:
        candidate = ROOT / relative
        if candidate.is_file():
            files.add(candidate)
        elif candidate.is_dir():
            files.update(path for path in candidate.rglob("*") if path.is_file())
        else:
            raise FileNotFoundError(f"manifest input does not exist: {relative}")
    return sorted(files)


def input_fingerprint(inputs: tuple[str, ...]) -> tuple[str, int]:
    digest = hashlib.sha256()
    files = resolved_inputs(inputs)
    for path in files:
        relative = path.relative_to(ROOT).as_posix().encode()
        digest.update(len(relative).to_bytes(4, "big"))
        digest.update(relative)
        digest.update(bytes.fromhex(sha256(path)))
    return digest.hexdigest(), len(files)


def input_snapshot() -> dict[str, dict[str, object]]:
    snapshot: dict[str, dict[str, object]] = {}
    for artifact, inputs in sorted(CERTORA_INPUTS.items()):
        fingerprint, file_count = input_fingerprint(inputs)
        snapshot[artifact] = {
            "source_inputs": list(inputs),
            "source_input_files": file_count,
            "source_input_sha256": fingerprint,
        }
    return snapshot


def snapshot_errors(expected: dict[str, object]) -> list[str]:
    current = input_snapshot()
    errors: list[str] = []
    expected_names = set(expected)
    current_names = set(current)
    for artifact in sorted(current_names - expected_names):
        errors.append(f"{artifact}: missing from input snapshot")
    for artifact in sorted(expected_names - current_names):
        errors.append(f"{artifact}: unexpected input snapshot entry")
    for artifact in sorted(current_names & expected_names):
        if expected[artifact] != current[artifact]:
            errors.append(f"{artifact}: source inputs changed during build")
    return errors


def build_metadata() -> dict[str, object]:
    return {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "toolchain": {
            "rustc": tool_version(["rustc", "--version"]),
            "stellar": tool_version(["stellar", "--version"]),
            "platform": platform.platform(),
        },
    }


def section(
    dir_path: Path,
    *,
    certora: bool = False,
    source_snapshot: dict[str, object] | None = None,
) -> dict[str, object]:
    wasm_names = {wasm.name for wasm in dir_path.glob("*.wasm")}
    if certora:
        expected = set(CERTORA_INPUTS)
        if wasm_names != expected:
            missing = ", ".join(sorted(expected - wasm_names)) or "none"
            unexpected = ", ".join(sorted(wasm_names - expected)) or "none"
            raise RuntimeError(
                f"Certora artifact set mismatch; missing: {missing}; "
                f"unexpected: {unexpected}"
            )

    files: dict[str, object] = {"_metadata": build_metadata()}
    for wasm in sorted(dir_path.glob("*.wasm")):
        entry: dict[str, object] = {
            "path": str(wasm.relative_to(ROOT)),
            "bytes": wasm.stat().st_size,
            "sha256": sha256(wasm),
        }
        if certora:
            inputs = CERTORA_INPUTS.get(wasm.name)
            if inputs is None:
                raise KeyError(f"missing Certora input declaration for {wasm.name}")
            if source_snapshot is None:
                fingerprint, file_count = input_fingerprint(inputs)
            else:
                snapshot_entry = source_snapshot.get(wasm.name)
                if not isinstance(snapshot_entry, dict):
                    raise KeyError(f"missing input snapshot for {wasm.name}")
                fingerprint = snapshot_entry["source_input_sha256"]
                file_count = snapshot_entry["source_input_files"]
            entry["build"] = {
                "source_revision": git_output(["rev-parse", "HEAD"]),
                "worktree_dirty": bool(git_output(["status", "--porcelain", "--untracked-files=all"])),
                "cargo_features": CERTORA_TARGETS[wasm.name].cargo_features,
                "no_default_features": False,
                "stellar_optimize": False,
                "source_inputs": list(inputs),
                "source_input_files": file_count,
                "source_input_sha256": fingerprint,
            }
        files[wasm.name] = entry
    return files


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--deploy", action="store_true")
    parser.add_argument("--certora", action="store_true")
    parser.add_argument("--write-input-snapshot", type=Path)
    parser.add_argument("--check-input-snapshot", type=Path)
    parser.add_argument("--input-snapshot", type=Path)
    args = parser.parse_args()

    if args.write_input_snapshot is not None:
        args.write_input_snapshot.write_text(json.dumps(input_snapshot(), indent=2) + "\n")
        print(f"wrote Certora input snapshot {args.write_input_snapshot}")
        return 0

    if args.check_input_snapshot is not None:
        expected = json.loads(args.check_input_snapshot.read_text())
        errors = snapshot_errors(expected)
        if errors:
            print("Certora sources changed during artifact build:", file=sys.stderr)
            for error in errors:
                print(f"  {error}", file=sys.stderr)
            return 1
        print("Certora input snapshot unchanged")
        return 0

    if not args.deploy and not args.certora:
        args.deploy = args.certora = True

    source_snapshot: dict[str, object] | None = None
    if args.input_snapshot is not None:
        if not args.certora:
            parser.error("--input-snapshot requires --certora")
        source_snapshot = json.loads(args.input_snapshot.read_text())
        errors = snapshot_errors(source_snapshot)
        if errors:
            print("Certora sources changed during artifact build:", file=sys.stderr)
            for error in errors:
                print(f"  {error}", file=sys.stderr)
            return 1

    manifest: dict[str, object] = {}
    if MANIFEST.exists():
        manifest = json.loads(MANIFEST.read_text())
    legacy_metadata = {
        "generated_at": manifest.get("generated_at"),
        "toolchain": manifest.get("toolchain"),
    }
    if all(value is not None for value in legacy_metadata.values()):
        for section_name in ("deploy", "certora"):
            existing_section = manifest.get(section_name)
            if isinstance(existing_section, dict) and "_metadata" not in existing_section:
                existing_section["_metadata"] = legacy_metadata
    manifest.pop("generated_at", None)
    manifest.pop("toolchain", None)

    if args.deploy:
        if not DEPLOY_DIR.is_dir():
            print(f"missing deploy dir: {DEPLOY_DIR}", file=sys.stderr)
            return 1
        manifest["deploy"] = section(DEPLOY_DIR)

    if args.certora:
        if not CERTORA_DIR.is_dir():
            print(f"missing certora dir: {CERTORA_DIR}", file=sys.stderr)
            return 1
        manifest["certora"] = section(
            CERTORA_DIR,
            certora=True,
            source_snapshot=source_snapshot,
        )

    MANIFEST.parent.mkdir(parents=True, exist_ok=True)
    MANIFEST.write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"wrote {MANIFEST.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
