#!/usr/bin/env python3
"""Check conf ↔ spec rule alignment in both directions.

- Orphan conf entries: rule listed in a conf with no matching #[rule] in spec.
- Dead spec rules: #[rule] function not referenced by any conf (never runs).
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RULE_RE = re.compile(r"#\[rule\]\s*(?:#\[[^\]]*\]\s*)*(?:pub\s+)?fn\s+(\w+)")
PROFILE_MANIFEST = ROOT / "profiles.json"
SOROBAN_CONF_KEYS = {
    "cargo_features",
    "files",
    "global_timeout",
    "independent_satisfy",
    "loop_iter",
    "msg",
    "multi_assert_check",
    "optimistic_loop",
    "precise_bitwise_ops",
    "prover_args",
    "rule",
    "rule_sanity",
    "server",
    "smt_timeout",
    "smt_use_bv",
}

# Soroban host-value encoding contains fixed loops longer than ten iterations
# (the pool fixture currently needs 28).  New controller/oracle jobs default to
# the sound host-state bound unless they are explicitly classified as pure math.
MIN_HOST_STATE_LOOP_ITER = 28
PURE_CONTROLLER_CONFS = {
    "boundary-compound-sanity.conf",
    "boundary-math-sanity.conf",
    "boundary-math.conf",
    "boundary-oracle.conf",
    "boundary-rates.conf",
    "compound-output.conf",
    "hf-lemmas-sanity.conf",
    "hf-lemmas.conf",
    "indexes.conf",
    "interest-compound.conf",
    "interest-index.conf",
    "interest.conf",
    "liquidation-bonus.conf",
    "math-bv.conf",
    "math.conf",
    "scaled-reconstruction.conf",
    "supply-dust-sanity.conf",
}
PURE_POOL_CONFS = {
    "rate-index-accounting.conf",
}
PURE_PRICE_AGGREGATOR_CONFS = {
    "freshness.conf",
    "oracle.conf",
    "tolerance-math.conf",
}


def read_rules(spec_dir: Path) -> set[str]:
    rules: set[str] = set()
    if not spec_dir.exists():
        return rules
    for source in spec_dir.rglob("*_rules.rs"):
        rules.update(RULE_RE.findall(source.read_text()))
    return rules


def read_rule_kinds(spec_dir: Path) -> dict[str, str]:
    kinds: dict[str, str] = {}
    if not spec_dir.exists():
        return kinds
    for source in spec_dir.rglob("*_rules.rs"):
        text = source.read_text()
        matches = list(RULE_RE.finditer(text))
        for index, match in enumerate(matches):
            end = matches[index + 1].start() if index + 1 < len(matches) else len(text)
            body = text[match.start() : end]
            has_assert = "cvlr_assert!" in body
            has_satisfy = "cvlr_satisfy!" in body
            if has_assert and has_satisfy:
                kinds[match.group(1)] = "mixed"
            elif has_satisfy:
                kinds[match.group(1)] = "satisfy"
            else:
                kinds[match.group(1)] = "assert"
    return kinds


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
    dead_rules: list[tuple[str, str]] = []
    profile_errors: list[str] = []
    config_errors: list[str] = []
    conf_source_rules: dict[Path, set[str]] = {}
    profiled_confs: set[Path] = set()

    for confs_dir in sorted(ROOT.glob("*/confs")):
        layer = confs_dir.parent.name
        source_rules = read_rules(confs_dir.parent / "spec")
        source_kinds = read_rule_kinds(confs_dir.parent / "spec")
        total_rules += len(source_rules)

        configured_rules: set[str] = set()
        for conf in sorted(confs_dir.glob("*.conf")):
            total_confs += 1
            conf_source_rules[conf.resolve()] = source_rules
            data = json.loads(conf.read_text())
            rules = conf_rules(conf)
            unknown_keys = sorted(set(data) - SOROBAN_CONF_KEYS)
            if unknown_keys:
                config_errors.append(
                    f"{layer}/{conf.name}: unsupported keys {', '.join(unknown_keys)}"
                )
            if not isinstance(data.get("msg"), str) or not data["msg"].strip():
                config_errors.append(f"{layer}/{conf.name}: missing short msg")
            if data.get("optimistic_loop") is not False:
                config_errors.append(
                    f"{layer}/{conf.name}: optimistic_loop must stay false for authoritative proofs"
                )
            try:
                loop_iter = int(data.get("loop_iter", 0))
                if loop_iter <= 0:
                    raise ValueError
            except (TypeError, ValueError):
                config_errors.append(f"{layer}/{conf.name}: loop_iter must be positive")
                loop_iter = 0
            needs_host_state_bound = (
                (layer == "pool" and conf.name not in PURE_POOL_CONFS)
                or (layer == "controller" and conf.name not in PURE_CONTROLLER_CONFS)
                or (
                    layer == "price-aggregator"
                    and conf.name not in PURE_PRICE_AGGREGATOR_CONFS
                )
            )
            if needs_host_state_bound and loop_iter < MIN_HOST_STATE_LOOP_ITER:
                config_errors.append(
                    f"{layer}/{conf.name}: loop_iter must be at least "
                    f"{MIN_HOST_STATE_LOOP_ITER} for Soroban host-state encoding"
                )
            prover_args = " ".join(str(arg) for arg in data.get("prover_args", []))
            for required_arg in ("-mediumTimeout", "-maxCommandCount"):
                if required_arg not in prover_args:
                    config_errors.append(
                        f"{layer}/{conf.name}: missing {required_arg} tuning"
                    )
            kinds = {source_kinds.get(rule_name) for rule_name in rules}
            kinds.discard(None)
            if "mixed" in kinds or len(kinds) > 1:
                config_errors.append(
                    f"{layer}/{conf.name}: mixes assert and satisfy semantics"
                )

            for rule_name in rules:
                configured_rules.add(rule_name)
                if rule_name not in source_rules:
                    orphans.append((f"{layer}/{conf.name}", rule_name))

        for rule_name in sorted(source_rules - configured_rules):
            dead_rules.append((layer, rule_name))

    total_profiles = 0
    if PROFILE_MANIFEST.exists():
        profiles = json.loads(PROFILE_MANIFEST.read_text()).get("profiles", {})
        total_profiles = len(profiles)
        for profile in sorted(profiles):
            for item in expand_profile(profiles, profile):
                if "profile_error" in item:
                    profile_errors.append(f"{profile}: invalid profile include {item['profile_error']}")
                    continue

                conf_path = (ROOT.parent / str(item["conf"])).resolve()
                if conf_path not in conf_source_rules:
                    profile_errors.append(f"{profile}: unknown conf {item['conf']}")
                    continue
                profiled_confs.add(conf_path)

                for rule_name in profile_rule_args([str(arg) for arg in item.get("args", [])]):
                    if rule_name not in conf_source_rules[conf_path]:
                        profile_errors.append(f"{profile}: {item['conf']} references unknown rule {rule_name}")

        for conf_path in sorted(set(conf_source_rules) - profiled_confs):
            profile_errors.append(
                f"unprofiled conf {conf_path.relative_to(ROOT.parent)}"
            )

    if orphans:
        print("Orphan conf entries (listed in conf but no matching #[rule] in spec):")
        for conf, rule_name in orphans:
            print(f"  {conf}: {rule_name}")
        return 1

    if dead_rules:
        print("Dead spec rules (#[rule] not referenced by any conf — wire in or delete):")
        for layer, rule_name in dead_rules:
            print(f"  {layer}: {rule_name}")
        return 1

    if profile_errors:
        print("Profile errors:")
        for error in profile_errors:
            print(f"  {error}")
        return 1

    if config_errors:
        print("Soroban conf integrity errors:")
        for error in config_errors:
            print(f"  {error}")
        return 1

    print(
        f"OK: {total_confs} confs, {total_rules} source rules, "
        f"{total_profiles} profiles, zero orphans, zero dead rules"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
