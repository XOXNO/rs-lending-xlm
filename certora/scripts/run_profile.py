#!/usr/bin/env python3
"""Run Certora Soroban configs from the centralized profile manifest."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "certora" / "profiles.json"


def load_profiles() -> dict[str, list[dict[str, object]]]:
    data = json.loads(MANIFEST.read_text())
    return data["profiles"]


def expand_profile(
    profiles: dict[str, list[dict[str, object]]],
    profile: str,
    seen: tuple[str, ...] = (),
) -> list[dict[str, object]]:
    if profile not in profiles:
        known = ", ".join(sorted(profiles))
        raise SystemExit(f"unknown profile '{profile}'. Known profiles: {known}")
    if profile in seen:
        chain = " -> ".join((*seen, profile))
        raise SystemExit(f"recursive profile include: {chain}")

    commands: list[dict[str, object]] = []
    for item in profiles[profile]:
        if "profile" in item:
            commands.extend(expand_profile(profiles, str(item["profile"]), (*seen, profile)))
        else:
            commands.append(item)
    return commands


def command_line(
    item: dict[str, object], extra_args: list[str], local: bool = False
) -> tuple[Path, list[str]]:
    conf_path = ROOT / str(item["conf"])
    args = [str(arg) for arg in item.get("args", [])]
    prover = "certoraSorobanLocal" if local else "certoraSorobanProver"
    return conf_path.parent, [prover, conf_path.name, *args, *extra_args]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", nargs="?", help="profile name from profiles.json")
    parser.add_argument("--list", action="store_true", help="list available profiles")
    parser.add_argument("--dry-run", action="store_true", help="print commands without executing")
    parser.add_argument(
        "--no-key-check",
        action="store_true",
        help="do not require CERTORAKEY before executing",
    )
    parser.add_argument(
        "--local",
        action="store_true",
        help="run with the locally built prover (certoraSorobanLocal) instead of the cloud",
    )
    parser.add_argument("extra_args", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    profiles = load_profiles()
    if args.list:
        for name in sorted(profiles):
            print(name)
        return 0

    if not args.profile:
        parser.error("profile is required unless --list is used")

    extra_args = list(args.extra_args)
    if "--dry-run" in extra_args:
        args.dry_run = True
        extra_args = [arg for arg in extra_args if arg != "--dry-run"]
    if "--no-key-check" in extra_args:
        args.no_key_check = True
        extra_args = [arg for arg in extra_args if arg != "--no-key-check"]
    if extra_args and extra_args[0] == "--":
        extra_args = extra_args[1:]

    commands = expand_profile(profiles, args.profile)
    if not args.no_key_check and not args.dry_run and not os.environ.get("CERTORAKEY"):
        raise SystemExit("error: CERTORAKEY is not set")
    if "--local" in extra_args:
        args.local = True
        extra_args = [arg for arg in extra_args if arg != "--local"]
    if args.local:
        args.no_key_check = True  # local runs never touch the cloud
    binary = "certoraSorobanLocal" if args.local else "certoraSorobanProver"
    if not args.dry_run and shutil.which(binary) is None:
        raise SystemExit(f"error: {binary} is not installed or not on PATH")

    for item in commands:
        cwd, cmd = command_line(item, extra_args, local=args.local)
        print(f"=== {cwd.relative_to(ROOT)}/{cmd[1]} {' '.join(cmd[2:])} ===", flush=True)
        if args.dry_run:
            print(f"cd {cwd} && {' '.join(cmd)}")
            continue
        result = subprocess.run(cmd, cwd=cwd)
        if result.returncode != 0:
            return result.returncode
    return 0


if __name__ == "__main__":
    sys.exit(main())
