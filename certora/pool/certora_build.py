#!/usr/bin/env python3
"""Certora Soroban build script — returns prebuilt pool WASM (no cloud rebuild)."""

import argparse
import json
import os
import sys

PROJECT_DIR = os.path.dirname(os.path.abspath(__file__))
ROOT_DIR = os.path.abspath(os.path.join(PROJECT_DIR, "..", ".."))
sys.path.insert(0, os.path.join(ROOT_DIR, "certora", "shared"))

from certora_wasm import certora_wasm_path  # noqa: E402

PACKAGE = "pool"
PACKAGE_DIR = os.path.join(ROOT_DIR, "contracts", PACKAGE)
EXECUTABLE = certora_wasm_path(PACKAGE, ROOT_DIR)


def tracked_sources():
    sources = [os.path.join(ROOT_DIR, "Cargo.toml"), os.path.join(PACKAGE_DIR, "Cargo.toml")]
    for path in (
        os.path.join(PACKAGE_DIR, "src"),
        os.path.join(ROOT_DIR, "common", "src"),
        os.path.join(PROJECT_DIR, "spec"),
        os.path.join(PROJECT_DIR, "confs"),
    ):
        for root, _dirs, files in os.walk(path):
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

    success = EXECUTABLE.is_file()
    output = {
        "project_directory": ROOT_DIR,
        "sources": tracked_sources(),
        "executables": str(EXECUTABLE) if success else "",
        "success": success,
        "return_code": 0 if success else 1,
    }
    if not success:
        output["error"] = (
            f"missing {EXECUTABLE}; run `make certora-wasm` before submitting jobs"
        )

    text = json.dumps(output, indent=2)
    if args.output:
        with open(args.output, "w") as handle:
            handle.write(text)
    elif args.json:
        print(text)

    return 0 if success else 1


if __name__ == "__main__":
    sys.exit(main())