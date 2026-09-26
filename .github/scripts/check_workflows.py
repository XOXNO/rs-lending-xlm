#!/usr/bin/env python3
"""Fail when a workflow action is not pinned to a full commit SHA, when the
workflow that runs the Static Gates job has a paths filter, or when a
self-hosted job of a pull_request workflow can run code from a fork.

Usage: python3 .github/scripts/check_workflows.py
"""
import re
import sys
from pathlib import Path

WORKFLOWS = Path(__file__).resolve().parents[1] / "workflows"
USES = re.compile(r"^\s*(?:-\s+)?uses:\s*['\"]?([^\s'\"#]+)")
PINNED = re.compile(r"[^@]+@[0-9a-f]{40}")
PATHS = re.compile(r"^\s+paths(?:-ignore)?:", re.MULTILINE)
STATIC_GATES = re.compile(r"^\s+name:\s*['\"]?Static Gates['\"]?\s*$", re.MULTILINE)
PULL_REQUEST = re.compile(r"\bpull_request(?:_target)?\b")
ON_BLOCK = re.compile(r"^['\"]?on['\"]?:(.*(?:\n(?:[ \t#].*|))*)", re.MULTILINE)
JOB = re.compile(r"^  ([A-Za-z0-9_-]+):\s*$", re.MULTILINE)
RUNS_ON = re.compile(r"^    runs-on:[ \t]*(.*(?:\n      .*)*)", re.MULTILINE)
JOB_IF = re.compile(r"^    if:[ \t]*(.*(?:\n      .*)*)", re.MULTILINE)
HOSTED = re.compile(r"^['\"]?(?:ubuntu|windows|macos)-[A-Za-z0-9._-]+['\"]?$")
EVENT_IS = re.compile(r"github\.event_name\s*==\s*'([a-z_]+)'")
FORK_GUARD = "github.event.pull_request.head.repo.full_name == github.repository"
NOT_PULL_REQUEST = "github.event_name != 'pull_request'"


def split_top(cond: str, op: str) -> list:
    """Splits `cond` on `op` outside parentheses."""
    parts, depth, start, i = [], 0, 0, 0
    while i < len(cond):
        if cond[i] == "(":
            depth += 1
        elif cond[i] == ")":
            depth -= 1
        elif depth == 0 and cond.startswith(op, i):
            parts.append(cond[start:i])
            start = i + len(op)
            i = start
            continue
        i += 1
    parts.append(cond[start:])
    return [strip_parens(part) for part in parts]


def strip_parens(text: str) -> str:
    text = " ".join(text.split())
    while text.startswith("(") and text.endswith(")"):
        inner = text[1:-1]
        depth = 0
        balanced = True
        for ch in inner:
            depth += ch == "("
            depth -= ch == ")"
            if depth < 0:
                balanced = False
                break
        if not balanced:
            break
        text = " ".join(inner.split())
    return text


def self_hosted(runs_on: str) -> bool:
    lines = [line.strip().lstrip("-").strip() for line in runs_on.splitlines() if line.strip()]
    return not lines or not all(HOSTED.match(line) for line in lines)


def guarded(cond: str) -> bool:
    """The job cannot run a fork pull request: every top-level clause admits
    only named non-PR events, or the same-repository guard is a required
    conjunct or the only alternative to a non-PR event."""
    cond = " ".join(cond.replace("${{", "").replace("}}", "").split())
    if cond.startswith(">"):
        cond = cond.lstrip(">-| ")
    clauses = split_top(cond, "||")
    if clauses == [NOT_PULL_REQUEST, FORK_GUARD]:
        return True
    if len(clauses) == 1 and FORK_GUARD in split_top(clauses[0], "&&"):
        return True
    return all(
        EVENT_IS.search(clause)
        and "pull_request" not in clause
        and "!" not in clause
        and FORK_GUARD not in clause
        for clause in clauses
    ) and bool(cond)


def fork_guard_errors(name: str, text: str) -> list:
    """A self-hosted job of a pull_request workflow must either never run on
    pull_request or carry the same-repository head guard."""
    triggers = ON_BLOCK.search(text)
    if not triggers or not PULL_REQUEST.search(triggers.group(1)):
        return []
    _, _, jobs = text.partition("\njobs:")
    errors = []
    parts = JOB.split(jobs)
    for job, body in zip(parts[1::2], parts[2::2]):
        runs_on = RUNS_ON.search(body)
        if not runs_on or not self_hosted(runs_on.group(1)):
            continue
        cond = JOB_IF.search(body)
        if not cond or not guarded(cond.group(1)):
            errors.append(f"{name}: self-hosted job {job} can run a fork pull_request; add `{FORK_GUARD}` to its if")
    return errors


def main() -> int:
    errors = []
    gates = []
    for wf in sorted(WORKFLOWS.glob("*.y*ml")):
        text = wf.read_text()
        for n, line in enumerate(text.splitlines(), 1):
            m = USES.match(line)
            if m and not m.group(1).startswith(("./", "docker://")) and not PINNED.fullmatch(m.group(1)):
                errors.append(f"{wf.name}:{n}: {m.group(1)} is not pinned to a full commit SHA")
        errors.extend(fork_guard_errors(wf.name, text))
        if STATIC_GATES.search(text):
            gates.append(wf.name)
            if PATHS.search(text):
                errors.append(f"{wf.name}: runs Static Gates, so it must not have a paths filter")
    if not gates:
        errors.append("no workflow defines the Static Gates job")
    for e in errors:
        print(e)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
