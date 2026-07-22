#!/usr/bin/env python3
"""Derive one focused Certora WASM target per rule-source module."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CERTORA_ROOT = ROOT / "certora"
RULE_RE = re.compile(r"#\[rule\]\s*(?:#\[[^\]]*\]\s*)*(?:pub\s+)?fn\s+(\w+)")

PACKAGES = {
    "common": "common",
    "pool": "pool",
    "controller": "controller",
    "price-aggregator": "price-aggregator",
}

BASE_INPUTS: dict[str, tuple[str, ...]] = {
    "common": (
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "Makefile",
        "certora/scripts/focused_wasm.py",
        "common/Cargo.toml",
        "common/src",
        "vendor/cvlr-log",
        "certora/common/spec",
        "certora/shared/summaries",
    ),
    "pool": (
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "Makefile",
        "certora/scripts/focused_wasm.py",
        "contracts/pool/Cargo.toml",
        "contracts/pool/src",
        "common/Cargo.toml",
        "common/src",
        "vendor/cvlr-log",
        "interfaces/pool",
        "certora/pool/spec",
        "certora/shared/summaries",
    ),
    "controller": (
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "Makefile",
        "certora/scripts/focused_wasm.py",
        "contracts/controller/Cargo.toml",
        "contracts/controller/src",
        "common/Cargo.toml",
        "common/src",
        "vendor/cvlr-log",
        "interfaces/controller",
        "interfaces/pool",
        "interfaces/price-aggregator",
        "certora/controller/harness",
        "certora/controller/spec",
        "certora/shared/summaries",
    ),
    "price-aggregator": (
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "Makefile",
        "certora/scripts/focused_wasm.py",
        "contracts/price-aggregator/Cargo.toml",
        "contracts/price-aggregator/src",
        "common/Cargo.toml",
        "common/src",
        "vendor/cvlr-log",
        "interfaces/price-aggregator",
        "certora/price-aggregator/spec",
    ),
}


@dataclass(frozen=True)
class FocusedTarget:
    layer: str
    module: str

    @property
    def package(self) -> str:
        return PACKAGES[self.layer]

    @property
    def feature(self) -> str:
        return f"certora-{self.module.replace('_', '-')}"

    @property
    def cargo_features(self) -> list[str]:
        return ["certora", "certora-focused", self.feature]

    @property
    def artifact(self) -> str:
        return f"{self.layer}-{self.module.replace('_', '-')}.wasm"

    @property
    def build_key(self) -> str:
        return self.artifact.removesuffix(".wasm")

    @property
    def inputs(self) -> tuple[str, ...]:
        return BASE_INPUTS[self.layer]

    @property
    def conf_relative_wasm(self) -> str:
        return f"../../../artifacts/wasm/certora/{self.artifact}"


def rules_by_module(layer: str) -> dict[str, str]:
    mapping: dict[str, str] = {}
    for source in sorted((CERTORA_ROOT / layer / "spec").glob("*_rules.rs")):
        for rule in RULE_RE.findall(source.read_text()):
            if rule in mapping:
                raise ValueError(f"duplicate rule {rule} in {layer}")
            mapping[rule] = source.stem
    return mapping


def target_for_conf(conf: Path, layer: str) -> FocusedTarget:
    data = json.loads(conf.read_text())
    rules = data.get("rule", [])
    if isinstance(rules, str):
        rules = [rules]
    mapping = rules_by_module(layer)
    modules = {mapping[rule] for rule in rules}
    if len(modules) != 1:
        raise ValueError(
            f"{conf.relative_to(ROOT)} spans rule modules: {', '.join(sorted(modules))}"
        )
    return FocusedTarget(layer, modules.pop())


def all_targets() -> list[FocusedTarget]:
    targets: set[FocusedTarget] = set()
    for layer in PACKAGES:
        for conf in sorted((CERTORA_ROOT / layer / "confs").glob("*.conf")):
            targets.add(target_for_conf(conf, layer))
    return sorted(targets, key=lambda target: (target.layer, target.module))


def target_by_artifact() -> dict[str, FocusedTarget]:
    return {target.artifact: target for target in all_targets()}


if __name__ == "__main__":
    for target in all_targets():
        print(
            "|".join(
                (
                    target.layer,
                    target.package,
                    target.feature,
                    target.artifact,
                    target.build_key,
                )
            )
        )
