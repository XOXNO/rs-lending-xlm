#!/usr/bin/env python3
"""Check conf ↔ spec rule alignment in both directions.

- Orphan conf entries: rule listed in a conf with no matching #[rule] in spec.
- Dead spec rules: #[rule] function not referenced by any conf (never runs).
- Duplicated rules: a rule may run in exactly one non-satisfy conf.
- Sanity policy: assert confs prove nothing without `rule_sanity: advanced`,
  revert-shaped confs must disable it, and every revert-shaped rule needs a
  reachability witness on its fixture.
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RULE_RE = re.compile(r"#\[rule\]\s*(?:#\[[^\]]*\]\s*)*(?:pub\s+)?fn\s+(\w+)")
PROFILE_MANIFEST = ROOT / "profiles.json"
# Keys certoraSorobanProver 8.17.1 accepts for a Soroban run. Anything outside
# SorobanProverAttributes is rejected with "not a known attribute", so this set
# must only ever grow with keys confirmed against the pinned wheel.
SOROBAN_CONF_KEYS = {
    "build_script",
    "cargo_build_verbose",
    "cargo_features",
    "cargo_tools_version",
    "coverage_info",
    "files",
    "global_timeout",
    "group_id",
    "independent_satisfy",
    "loop_iter",
    "msg",
    "multi_assert_check",
    "multi_example",
    "optimistic_loop",
    "override_base_config",
    "precise_bitwise_ops",
    "protocol_author",
    "protocol_name",
    "prover_args",
    "prover_version",
    "rule",
    "rule_sanity",
    "server",
    "short_output",
    "smt_timeout",
    "smt_use_bv",
    "tool_output",
    "url_visibility",
    "wait_for_results",
}

MIN_HOST_STATE_LOOP_ITER = 28

OPTIMISTIC_LOOP_CONFS = {"lp-math-stable.conf"}
PURE_CONTROLLER_CONFS = {
    "boundary-bad-debt-sanity.conf",
    "boundary-compound-sanity.conf",
    "boundary-math-sanity.conf",
    "boundary-math.conf",
    "boundary-oracle.conf",
    "boundary-rates.conf",
    "compound-output.conf",
    "hf-lemmas-sanity.conf",
    "hf-lemmas.conf",
    "interest-compound.conf",
    "interest-index.conf",
    "interest.conf",
    "liquidation-accounting-math.conf",
    "liquidation-bonus.conf",
    "math-bv.conf",
    "math-reverts-sanity.conf",
    "math-reverts.conf",
    "math.conf",
    "scaled-reconstruction.conf",
    "solvency-roundtrip.conf",
    "supply-dust-sanity.conf",
}
PURE_POOL_CONFS: set[str] = set()
PURE_PRICE_AGGREGATOR_CONFS = {
    "freshness-reverts-sanity.conf",
    "freshness-reverts.conf",
    "freshness.conf",
    "oracle.conf",
    "scaled-math.conf",
    "tolerance-math-reverts-sanity.conf",
    "tolerance-math-reverts.conf",
    "tolerance-math.conf",
}

# A revert-shaped rule is `call(...); cvlr_assert!(false);`. The TAC vacuity
# check removes user asserts and asserts false at every sink, so it reports
# SANITY_FAILED on this shape by construction (note §7). Those rules therefore
# live in confs with `rule_sanity: none`, and their reachability evidence is a
# satisfy witness that completes the same fixture. Two forms are accepted: the
# `<rule>_fixture_completes` twin in the sibling `-sanity` conf, or the
# module's existing success witness listed here. An entry is only valid when
# that witness drives the same verb, or is the module's only witness.
EXISTING_WITNESS = {
    # controller/spec/flash_loan_rules.rs — the guard is the whole fixture.
    "flash_loan_guard_blocks_callers": "flash_loan_guard_allows_when_clear",
    # controller/spec/market_guard_rules.rs — one module-level success witness.
    "disabled_market_blocks_new_supply": "market_guard_reachability",
    "supply_new_slot_requires_owner_or_delegate": "market_guard_reachability",
    # controller/spec/solvency_rules.rs — witness per verb.
    "supply_rejects_zero_amount": "solvency_sanity_supply",
    "borrow_rejects_zero_amount": "solvency_sanity_borrow",
    "repay_rejects_zero_amount": "solvency_sanity_repay",
    # controller/spec/spoke_rules.rs — witness per verb.
    "spoke_only_registered_assets": "spoke_supply_sanity",
    "spoke_only_collateralizable_assets": "spoke_supply_sanity",
    "deprecated_spoke_blocks_new_supply": "spoke_supply_sanity",
    "spoke_borrow_only_registered_assets": "spoke_borrow_sanity",
    "spoke_only_borrowable_assets": "spoke_borrow_sanity",
    "deprecated_spoke_blocks_new_borrow": "spoke_borrow_sanity",
    "bulk_borrow_distinct_legs_exceed_limit_reverts": (
        "bulk_borrow_duplicate_leg_not_double_counted"
    ),
    # controller/spec/strategy_rules.rs — witness per verb.
    "flash_position_guard_blocks_entrypoint": "flash_position_sanity",
    "flash_position_rejects_all_zero_mins": "flash_position_sanity",
    "flash_position_rejects_duplicate_collateral_asset": "flash_position_sanity",
    "flash_position_rejects_empty_collaterals": "flash_position_sanity",
    "flash_position_rejects_non_flashloanable_market": "flash_position_sanity",
    "flash_position_rejects_normal_mode": "flash_position_sanity",
    "flash_position_rejects_zero_amount": "flash_position_sanity",
    "multiply_rejects_same_tokens": "multiply_sanity",
    "multiply_requires_collateralizable": "multiply_sanity",
    "swap_collateral_rejects_same_token": "swap_collateral_sanity",
    "swap_debt_rejects_same_token": "swap_debt_sanity",
    # price-aggregator/spec/oracle_rules.rs — one module-level success witness.
    "empty_legs_force_reverts": "price_endpoint_sanity",
    "missing_oracle_config_reverts": "price_endpoint_sanity",
    "partial_legs_force_reverts": "price_endpoint_sanity",
}

def read_rules(spec_dir: Path) -> set[str]:
    rules: set[str] = set()
    if not spec_dir.exists():
        return rules
    for source in spec_dir.rglob("*_rules.rs"):
        rules.update(RULE_RE.findall(source.read_text()))
    return rules

def rule_body(text: str, start: int) -> str:
    """Return the brace-matched body of the rule whose signature starts at `start`.

    Slicing to the next `#[rule]` instead would swallow the helper functions
    between two rules, and their asserts would be read as the earlier rule's.
    """
    open_brace = text.find("{", start)
    if open_brace < 0:
        return ""
    depth = 0
    for index in range(open_brace, len(text)):
        if text[index] == "{":
            depth += 1
        elif text[index] == "}":
            depth -= 1
            if depth == 0:
                return text[open_brace : index + 1]
    return text[open_brace:]

def read_rule_kinds(spec_dir: Path) -> dict[str, str]:
    """Map rule name to "assert", "revert", "satisfy" or "mixed".

    "revert" is the `call(...); cvlr_assert!(false);` shape: every assert in the
    body is `false`, so the rule proves that the call cannot return.
    """
    kinds: dict[str, str] = {}
    if not spec_dir.exists():
        return kinds
    for source in spec_dir.rglob("*_rules.rs"):
        text = source.read_text()
        for match in RULE_RE.finditer(text):
            # Classify on code only: a doc comment naming cvlr_satisfy!/
            # cvlr_assert! must not change a rule's kind.
            body = rule_body(text, match.end())
            body = re.sub(r"/\*.*?\*/", "", body, flags=re.S)
            body = re.sub(r"//[^\n]*", "", body)
            asserts = re.findall(r"cvlr_assert!\s*\(", body)
            false_asserts = re.findall(r"cvlr_assert!\s*\(\s*false\s*\)", body)
            has_satisfy = "cvlr_satisfy!" in body
            if asserts and has_satisfy:
                kinds[match.group(1)] = "mixed"
            elif has_satisfy:
                kinds[match.group(1)] = "satisfy"
            elif asserts and len(asserts) == len(false_asserts):
                kinds[match.group(1)] = "revert"
            else:
                kinds[match.group(1)] = "assert"
    return kinds

def conf_kind(conf: Path, rules: list[str], kinds: dict[str, str]) -> str:
    """Classify a conf as "revert", "satisfy" or "assert".

    Falls back to the file-name convention while a rule the conf names has not
    been written yet; check_orphans reports those separately as orphans.
    """
    known = {kinds[rule] for rule in rules if rule in kinds}
    if known == {"revert"}:
        return "revert"
    if known == {"satisfy"}:
        return "satisfy"
    if not known:
        if conf.name.endswith("-reverts.conf"):
            return "revert"
        if conf.name.endswith("-sanity.conf"):
            return "satisfy"
    return "assert"

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
        # Per layer, because two layers may define a rule with the same name.
        owning_confs: dict[str, list[str]] = {}
        satisfy_rules: set[str] = set()
        for conf in sorted(confs_dir.glob("*.conf")):
            rules = conf_rules(conf)
            kind = conf_kind(conf, rules, source_kinds)
            if kind == "satisfy":
                satisfy_rules.update(rules)
            else:
                for rule_name in rules:
                    owning_confs.setdefault(rule_name, []).append(conf.name)

        for rule_name, owners in sorted(owning_confs.items()):
            if len(owners) > 1:
                config_errors.append(
                    f"{layer}: rule {rule_name} runs in {len(owners)} non-satisfy "
                    f"confs ({', '.join(owners)}); move it, do not copy it"
                )

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
            if (
                conf.name not in OPTIMISTIC_LOOP_CONFS
                and data.get("optimistic_loop") is not False
            ):
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
            # A satisfy rule has its generated asserts rewritten into assumes,
            # the loop-unwinding assertion included. An under-unrolled witness
            # therefore truncates the search silently instead of failing, so a
            # sanity conf never runs below its assert twin's bound.
            if conf.name.endswith("-sanity.conf"):
                twin = confs_dir / conf.name.replace("-sanity.conf", ".conf")
                if twin.is_file():
                    twin_iter = int(json.loads(twin.read_text()).get("loop_iter", 0))
                    if loop_iter < twin_iter:
                        config_errors.append(
                            f"{layer}/{conf.name}: loop_iter {loop_iter} is below "
                            f"{twin.name}'s {twin_iter}; a satisfy rule assumes the "
                            "unwinding condition instead of asserting it"
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
                    f"{layer}/{conf.name}: mixes {', '.join(sorted(kinds))} rule "
                    "shapes; one conf holds one shape"
                )

            # Sanity policy. On WASM the prover emits its vacuity sub-rule only
            # at `advanced`; `basic` emits nothing at all (note §7). A revert
            # shaped rule is vacuous to that check by construction, so its conf
            # turns the check off and pairs each rule with a satisfy witness.
            kind = conf_kind(conf, rules, source_kinds)
            sanity = data.get("rule_sanity")
            expected_sanity = "advanced" if kind == "assert" else "none"
            if sanity != expected_sanity:
                config_errors.append(
                    f"{layer}/{conf.name}: {kind} conf needs "
                    f"rule_sanity {expected_sanity!r}, found {sanity!r}"
                )
            if kind == "revert":
                twin_conf = confs_dir / conf.name.replace(".conf", "-sanity.conf")
                twins = set(conf_rules(twin_conf)) if twin_conf.is_file() else set()
                for rule_name in rules:
                    witness = EXISTING_WITNESS.get(rule_name)
                    if witness is not None:
                        if source_kinds.get(witness) not in (None, "satisfy"):
                            config_errors.append(
                                f"{layer}/{conf.name}: witness {witness} for "
                                f"{rule_name} is not a satisfy rule"
                            )
                        elif witness not in satisfy_rules:
                            config_errors.append(
                                f"{layer}/{conf.name}: witness {witness} for "
                                f"{rule_name} is not run by any satisfy conf"
                            )
                        continue
                    if f"{rule_name}_fixture_completes" not in twins:
                        config_errors.append(
                            f"{layer}/{conf.name}: revert-shaped {rule_name} has no "
                            f"witness; add {rule_name}_fixture_completes to "
                            f"{twin_conf.name} or an EXISTING_WITNESS entry"
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

    # Every group is reported in one pass: an orphan entry left by a spec change
    # in flight must not hide a sanity-policy or duplicate-rule regression.
    if orphans:
        print("Orphan conf entries (listed in conf but no matching #[rule] in spec):")
        for conf, rule_name in orphans:
            print(f"  {conf}: {rule_name}")

    if dead_rules:
        print("Dead spec rules (#[rule] not referenced by any conf — wire in or delete):")
        for layer, rule_name in dead_rules:
            print(f"  {layer}: {rule_name}")

    if profile_errors:
        print("Profile errors:")
        for error in profile_errors:
            print(f"  {error}")

    if config_errors:
        print("Soroban conf integrity errors:")
        for error in config_errors:
            print(f"  {error}")

    if orphans or dead_rules or profile_errors or config_errors:
        return 1

    print(
        f"OK: {total_confs} confs, {total_rules} source rules, "
        f"{total_profiles} profiles, zero orphans, zero dead rules"
    )
    return 0

if __name__ == "__main__":
    sys.exit(main())
