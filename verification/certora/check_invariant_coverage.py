#!/usr/bin/env python3
"""Check INVARIANTS.md Certora targets have rules and conf coverage."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
INVARIANTS = REPO / "architecture" / "INVARIANTS.md"
RULE_RE = re.compile(r"#\[rule\]\s*(?:#\[[^\]]*\]\s*)*(?:pub\s+)?fn\s+(\w+)")

# Satisfied outside Certora (fuzz, integration tests, build graph).
NON_CERTORA = {
    "fp_math",
    "fp_ops",
    "flow_e2e",
    "flow_strategy",
    "fuzz_liquidation_differential",
    "fuzz_multi_asset_solvency",
    "fuzz_strategy_flashloan",
    "oracle tests",
    "config tests",
    "build graph",
    "controller-to-pool tests",
    "storage tests",
    "account_ttl_regression_tests",
}

ALIASES = {
    "rates_and_index": "rates_rules",
}


def parse_invariants_targets() -> set[str]:
    targets: set[str] = set()
    for line in INVARIANTS.read_text().splitlines():
        if not line.startswith("|"):
            continue
        parts = [part.strip() for part in line.split("|")]
        if len(parts) < 4:
            continue
        runtime, verification = parts[1], parts[2]
        if runtime.lower() == "runtime" or runtime.startswith("---"):
            continue
        for chunk in verification.split(","):
            name = chunk.strip().strip("`").strip()
            if name:
                targets.add(name)
    return targets


def certora_module(name: str) -> str | None:
    if name in NON_CERTORA:
        return None
    if name in ALIASES:
        name = ALIASES[name]
    if name.endswith("_rules"):
        return name
    return None


def load_spec_rules() -> dict[str, set[str]]:
    rules_by_module: dict[str, set[str]] = {}
    for spec in ROOT.glob("**/spec/*_rules.rs"):
        module = spec.stem
        rules = set(RULE_RE.findall(spec.read_text()))
        rules_by_module[module] = rules_by_module.get(module, set()) | rules
    return rules_by_module


def load_conf_rule_names() -> set[str]:
    names: set[str] = set()
    for conf in ROOT.glob("**/confs/*.conf"):
        data = json.loads(conf.read_text())
        rules = data.get("rule", [])
        if isinstance(rules, str):
            rules = [rules]
        names.update(rules)
    return names


def main() -> int:
    if not INVARIANTS.exists():
        print(f"error: missing {INVARIANTS}")
        return 1

    required_modules = {
        module
        for target in parse_invariants_targets()
        if (module := certora_module(target)) is not None
    }
    rules_by_module = load_spec_rules()
    conf_rules = load_conf_rule_names()

    missing_modules: list[str] = []
    empty_modules: list[str] = []
    unconfigured_modules: list[str] = []

    for module in sorted(required_modules):
        rules = rules_by_module.get(module)
        if rules is None:
            missing_modules.append(module)
            continue
        if not rules:
            empty_modules.append(module)
            continue
        if not rules & conf_rules:
            unconfigured_modules.append(module)

    if missing_modules or empty_modules or unconfigured_modules:
        if missing_modules:
            print("INVARIANTS Certora targets without spec modules:")
            for module in missing_modules:
                print(f"  {module}: expected verification/certora/**/spec/{module}.rs")
        if empty_modules:
            print("INVARIANTS Certora spec modules without #[rule] functions:")
            for module in empty_modules:
                print(f"  {module}")
        if unconfigured_modules:
            print("INVARIANTS Certora spec modules not referenced by any conf:")
            for module in unconfigured_modules:
                print(f"  {module}")
        return 1

    skipped = sorted(target for target in parse_invariants_targets() if target in NON_CERTORA)
    print(
        f"OK: {len(required_modules)} INVARIANTS Certora modules covered "
        f"({len(skipped)} non-Certora targets skipped)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())