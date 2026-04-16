#!/usr/bin/env python3
"""
Certora Sunbeam build script for the rs-lending Stellar controller.

This script is invoked by the Certora Prover to compile the controller
contract to WASM. The prover provides the `cvlr` crate in its build
environment, so the `certora` feature can compile the spec rules.
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile

PROJECT_DIR = os.path.dirname(os.path.abspath(__file__))
STELLAR_DIR = os.path.dirname(PROJECT_DIR)

# Build command: target wasm32v1-none, matching our contract build target.
# Works only with a #![no_std]-patched cvlr-spec (see stellar/vendor/cvlr).
# The [patch."https://github.com/Certora/cvlr.git"] entry in the workspace
# Cargo.toml redirects cvlr-spec to the vendored copy.
BUILD_CMD = (
    f"cd {STELLAR_DIR} && "
    f"stellar contract build --package controller --features certora"
)

# Source files the prover tracks for cache invalidation
SOURCES = []
for source_root in ("src", "certora"):
    root_path = os.path.join(PROJECT_DIR, source_root)
    if not os.path.isdir(root_path):
        continue
    for root, dirs, files in os.walk(root_path):
        for f in files:
            if f.endswith(".rs") or f.endswith(".py") or f.endswith(".conf"):
                SOURCES.append(os.path.join(root, f))

COMMON_DIR = os.path.join(STELLAR_DIR, "common", "src")
if os.path.isdir(COMMON_DIR):
    for root, dirs, files in os.walk(COMMON_DIR):
        for f in files:
            if f.endswith(".rs"):
                SOURCES.append(os.path.join(root, f))

SOURCES.append(os.path.join(STELLAR_DIR, "Cargo.toml"))
SOURCES.append(os.path.join(PROJECT_DIR, "Cargo.toml"))

EXECUTABLE = os.path.join(
    STELLAR_DIR, "target", "wasm32v1-none", "release", "controller.wasm"
)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("-o", "--output")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("-l", "--log", action="store_true")
    parser.add_argument("-v", "--verbose", action="store_true")
    parser.add_argument("--cargo_features", nargs="*", default=[])
    args = parser.parse_args()

    # Build with stellar contract build (wasm32v1-none). Relies on the
    # workspace [patch] redirect of cvlr-spec to the #![no_std]-patched
    # vendored copy at stellar/vendor/cvlr.
    cmd = BUILD_CMD
    if args.cargo_features:
        features = ",".join(args.cargo_features)
        if "certora" not in args.cargo_features:
            features = "certora," + features
        cmd = (
            f"cd {STELLAR_DIR} && "
            f"stellar contract build --package controller --features {features}"
        )

    # Run the build
    stdout_file = tempfile.NamedTemporaryFile(mode="w", suffix=".log", delete=False) if not args.log else None
    stderr_file = tempfile.NamedTemporaryFile(mode="w", suffix=".log", delete=False) if not args.log else None

    result = subprocess.run(
        cmd,
        shell=True,
        cwd=STELLAR_DIR,
        stdout=stdout_file if stdout_file else None,
        stderr=stderr_file if stderr_file else None,
    )

    stdout_path = stdout_file.name if stdout_file else None
    stderr_path = stderr_file.name if stderr_file else None
    if stdout_file: stdout_file.close()
    if stderr_file: stderr_file.close()

    success = result.returncode == 0 and os.path.exists(EXECUTABLE)

    output = {
        "project_directory": STELLAR_DIR,
        "sources": SOURCES,
        # The prover expects a single path string, not a list. See
        # Certora/sunbeam-tutorials certora_build.py for reference shape.
        "executables": EXECUTABLE if success else "",
        "success": success,
        "return_code": result.returncode,
    }
    if stdout_path: output["stdout_log"] = stdout_path
    if stderr_path: output["stderr_log"] = stderr_path

    text = json.dumps(output, indent=2)
    if args.output:
        with open(args.output, "w") as f:
            f.write(text)
    elif args.json:
        print(text)

    return 0 if success else 1


if __name__ == "__main__":
    sys.exit(main())
