
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
    return json.loads(MANIFEST.read_text())["profiles"]


def expand_profile(
    profiles: dict[str, list[dict[str, object]]],
    profile: str,
    seen: tuple[str, ...] = (),
) -> list[dict[str, object]]:
    if profile not in profiles:
        known = ", ".join(sorted(profiles))
        raise SystemExit(f"unknown profile '{profile}'. Known profiles: {known}")
    if profile in seen:
        raise SystemExit(f"recursive profile include: {' -> '.join((*seen, profile))}")

    commands: list[dict[str, object]] = []
    for item in profiles[profile]:
        if "profile" in item:
            commands.extend(
                expand_profile(profiles, str(item["profile"]), (*seen, profile))
            )
        else:
            commands.append(item)
    return commands


def command_line(
    item: dict[str, object], extra_args: list[str], *, local: bool
) -> tuple[Path, list[str]]:
    conf_path = ROOT / str(item["conf"])
    args = [str(a) for a in item.get("args", [])]
    if local:
        runner = ROOT / "certora" / "scripts" / "run-rules-local.sh"
        return ROOT, [str(runner), str(conf_path), *args, *extra_args]
    return conf_path.parent, ["certoraSorobanProver", conf_path.name, *args, *extra_args]


def _strip_flag(extra: list[str], flag: str) -> tuple[list[str], bool]:
    if flag not in extra:
        return extra, False
    return [a for a in extra if a != flag], True


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", nargs="?", help="profile name from profiles.json")
    parser.add_argument("--list", action="store_true", help="list available profiles")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument(
        "--no-key-check",
        action="store_true",
        help="do not require CERTORAKEY before executing",
    )
    parser.add_argument(
        "--local",
        action="store_true",
        help="run each rule via run-rules-local.sh + local Prover JAR",
    )
    parser.add_argument("extra_args", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    profiles = load_profiles()
    if args.list:
        print("\n".join(sorted(profiles)))
        return 0
    if not args.profile:
        parser.error("profile is required unless --list is used")

    extra = list(args.extra_args)
    for flag, attr in (
        ("--dry-run", "dry_run"),
        ("--no-key-check", "no_key_check"),
        ("--local", "local"),
    ):
        extra, hit = _strip_flag(extra, flag)
        if hit:
            setattr(args, attr, True)
    if args.local:
        args.no_key_check = True
    if extra and extra[0] == "--":
        extra = extra[1:]

    if not args.no_key_check and not args.dry_run and not os.environ.get("CERTORAKEY"):
        raise SystemExit("error: CERTORAKEY is not set")
    if (
        not args.dry_run
        and not args.local
        and shutil.which("certoraSorobanProver") is None
    ):
        raise SystemExit("error: certoraSorobanProver is not installed or not on PATH")

    for item in expand_profile(profiles, args.profile):
        cwd, cmd = command_line(item, extra, local=args.local)
        print(f"=== {item['conf']} {' '.join(cmd[2:])} ===", flush=True)
        if args.dry_run:
            print(f"cd {cwd} && {' '.join(cmd)}")
            continue
        result = subprocess.run(cmd, cwd=cwd)
        if result.returncode != 0:
            return result.returncode
    return 0


if __name__ == "__main__":
    sys.exit(main())
