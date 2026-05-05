#!/usr/bin/env python3
"""Check that every configured Certora rule has a matching #[rule] function."""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
RULE_RE = re.compile(r"#\[rule\]\s*(?:pub\s+)?fn\s+(\w+)")
PROFILE_MANIFEST = ROOT / "profiles.json"


def read_rules(spec_dir: Path) -> set[str]:
    rules: set[str] = set()
    if not spec_dir.exists():
        return rules
    for source in spec_dir.rglob("*_rules.rs"):
        rules.update(RULE_RE.findall(source.read_text()))
    return rules


def conf_rules(conf: Path) -> list[str]:
    data = json.loads(conf.read_text())
    rules = data.get("rule", [])
    if isinstance(rules, str):
        return [rules]
    return list(rules)


def profile_rule_args(args: list[str]) -> list[str]:
    rules: list[str] = []
    index = 0
    while index < len(args):
        if args[index] in {"--rule", "--exclude_rule"}:
            index += 1
            while index < len(args) and not args[index].startswith("--"):
                rules.append(args[index])
                index += 1
            continue
        index += 1
    return rules


def expand_profile(
    profiles: dict[str, list[dict[str, object]]],
    profile: str,
    seen: tuple[str, ...] = (),
) -> list[dict[str, object]]:
    if profile not in profiles:
        return [{"profile_error": profile}]
    if profile in seen:
        return [{"profile_error": " -> ".join((*seen, profile))}]

    commands: list[dict[str, object]] = []
    for item in profiles[profile]:
        if "profile" in item:
            commands.extend(expand_profile(profiles, str(item["profile"]), (*seen, profile)))
        else:
            commands.append(item)
    return commands


def main() -> int:
    total_confs = 0
    total_rules = 0
    orphans: list[tuple[str, str]] = []
    profile_errors: list[str] = []
    conf_source_rules: dict[Path, set[str]] = {}

    for confs_dir in sorted(ROOT.glob("*/confs")):
        layer = confs_dir.parent.name
        source_rules = read_rules(confs_dir.parent / "spec")
        total_rules += len(source_rules)

        for conf in sorted(confs_dir.glob("*.conf")):
            total_confs += 1
            conf_source_rules[conf.resolve()] = source_rules
            for rule_name in conf_rules(conf):
                if rule_name not in source_rules:
                    orphans.append((f"{layer}/{conf.name}", rule_name))

    total_profiles = 0
    if PROFILE_MANIFEST.exists():
        profiles = json.loads(PROFILE_MANIFEST.read_text()).get("profiles", {})
        total_profiles = len(profiles)
        for profile in sorted(profiles):
            for item in expand_profile(profiles, profile):
                if "profile_error" in item:
                    profile_errors.append(f"{profile}: invalid profile include {item['profile_error']}")
                    continue

                conf_path = (ROOT.parent.parent / str(item["conf"])).resolve()
                if conf_path not in conf_source_rules:
                    profile_errors.append(f"{profile}: unknown conf {item['conf']}")
                    continue

                for rule_name in profile_rule_args([str(arg) for arg in item.get("args", [])]):
                    if rule_name not in conf_source_rules[conf_path]:
                        profile_errors.append(f"{profile}: {item['conf']} references unknown rule {rule_name}")

    if orphans:
        print("Orphan conf entries (listed in conf but no matching #[rule] in spec):")
        for conf, rule_name in orphans:
            print(f"  {conf}: {rule_name}")
        return 1

    if profile_errors:
        print("Profile errors:")
        for error in profile_errors:
            print(f"  {error}")
        return 1

    print(f"OK: {total_confs} confs, {total_rules} source rules, {total_profiles} profiles, zero orphans")
    return 0


if __name__ == "__main__":
    sys.exit(main())
