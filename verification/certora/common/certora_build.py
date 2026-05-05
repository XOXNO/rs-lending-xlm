#!/usr/bin/env python3
"""Certora Soroban build script for the shared common crate."""

import argparse
import json
import os
import subprocess
import sys
import tempfile

PROJECT_DIR = os.path.dirname(os.path.abspath(__file__))
ROOT_DIR = os.path.abspath(os.path.join(PROJECT_DIR, "..", "..", ".."))
PACKAGE = "common"
PACKAGE_DIR = os.path.join(ROOT_DIR, PACKAGE)
TARGET_DIR = os.path.join(ROOT_DIR, "target", "certora", PACKAGE)
EXECUTABLE = os.path.join(TARGET_DIR, "wasm32v1-none", "release", f"{PACKAGE}.wasm")


def tracked_sources():
    sources = [os.path.join(ROOT_DIR, "Cargo.toml"), os.path.join(PACKAGE_DIR, "Cargo.toml")]
    for root_path in (
        os.path.join(PACKAGE_DIR, "src"),
        os.path.join(PROJECT_DIR, "spec"),
        os.path.join(PROJECT_DIR, "confs"),
    ):
        for root, _dirs, files in os.walk(root_path):
            for name in files:
                if name.endswith((".rs", ".py", ".conf")):
                    sources.append(os.path.join(root, name))
    for name in os.listdir(PROJECT_DIR):
        if name.endswith(".py"):
            sources.append(os.path.join(PROJECT_DIR, name))
    return sources


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("-o", "--output")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("-l", "--log", action="store_true")
    parser.add_argument("-v", "--verbose", action="store_true")
    parser.add_argument("--cargo_features", nargs="*", default=[])
    args = parser.parse_args()

    features = args.cargo_features or ["certora"]
    if "certora" not in features:
        features = ["certora", *features]
    cmd = (
        f"cd {ROOT_DIR} && "
        f"stellar contract build --package {PACKAGE} --features {','.join(features)}"
    )

    stdout_file = tempfile.NamedTemporaryFile(mode="w", suffix=".log", delete=False) if not args.log else None
    stderr_file = tempfile.NamedTemporaryFile(mode="w", suffix=".log", delete=False) if not args.log else None
    result = subprocess.run(
        cmd,
        shell=True,
        cwd=ROOT_DIR,
        env={**os.environ, "CARGO_TARGET_DIR": TARGET_DIR},
        stdout=stdout_file if stdout_file else None,
        stderr=stderr_file if stderr_file else None,
    )

    stdout_path = stdout_file.name if stdout_file else None
    stderr_path = stderr_file.name if stderr_file else None
    if stdout_file:
        stdout_file.close()
    if stderr_file:
        stderr_file.close()

    success = result.returncode == 0 and os.path.exists(EXECUTABLE)
    output = {
        "project_directory": ROOT_DIR,
        "sources": tracked_sources(),
        "executables": EXECUTABLE if success else "",
        "success": success,
        "return_code": result.returncode,
    }
    if stdout_path:
        output["stdout_log"] = stdout_path
    if stderr_path:
        output["stderr_log"] = stderr_path

    text = json.dumps(output, indent=2)
    if args.output:
        with open(args.output, "w") as f:
            f.write(text)
    elif args.json:
        print(text)

    return 0 if success else 1


if __name__ == "__main__":
    sys.exit(main())
