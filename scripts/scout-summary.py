#!/usr/bin/env python3
"""Render Scout JSON reports as Markdown (stdout / $GITHUB_STEP_SUMMARY).

Usage:
    scout-summary.py <dir-with-*.json>
    scout-summary.py report1.json report2.json
"""
from __future__ import annotations

import glob
import json
import os
import sys

SEV = ("critical", "medium", "minor", "enhancement")
EMOJI = {"critical": "🔴", "medium": "🟠", "minor": "🟡", "enhancement": "🔵"}

def collect(args: list[str]) -> list[str]:
    files: list[str] = []
    for a in args:
        files.extend(
            sorted(glob.glob(os.path.join(a, "*.json"))) if os.path.isdir(a) else [a]
        )
    if not files:
        raise ValueError("no Scout reports found")
    return sorted(set(files))

def span_line(span: str) -> str:
    left = (span or "").split(" - ", 1)[0]
    parts = left.rsplit(":", 2)
    return parts[1] if len(parts) >= 3 else ""

def main() -> None:
    rows: list[tuple] = []
    findings: list[tuple] = []
    totals = {s: 0 for s in SEV}

    for path in collect(sys.argv[1:]):
        try:
            with open(path, encoding="utf-8") as fh:
                data = json.load(fh)
        except (OSError, json.JSONDecodeError) as error:
            raise ValueError(f"invalid Scout report {path}: {error}") from error
        if not isinstance(data, dict) or not isinstance(data.get("summary"), dict):
            raise ValueError(f"missing Scout summary: {path}")
        by_sev = data["summary"].get("by_severity")
        if not isinstance(by_sev, dict) or not isinstance(data.get("findings"), list):
            raise ValueError(f"invalid Scout summary/findings: {path}")
        crate = os.path.basename(path)[:-5]
        if set(by_sev) - set(SEV) or any(type(n) is not int or n < 0 for n in by_sev.values()):
            raise ValueError(f"invalid Scout severity counts: {path}")
        counts = tuple(int(by_sev.get(s, 0)) for s in SEV)
        rows.append((crate, *counts))
        for s, n in zip(SEV, counts):
            totals[s] += n
        for x in data.get("findings") or []:
            findings.append(
                (
                    crate,
                    x.get("vulnerability_id", ""),
                    (x.get("error_message") or "").split("] ", 1)[-1][:100],
                    x.get("file_path", ""),
                    span_line(x.get("span", "")),
                )
            )

    out = ["## 🔍 Scout Audit", ""]
    grand = sum(totals.values())
    if grand == 0 and findings:
        raise ValueError("Scout findings disagree with severity counts")
    if grand == 0:
        out.append("✅ **No findings.**")
    else:
        badge = " · ".join(f"{EMOJI[s]} {totals[s]} {s}" for s in SEV if totals[s])
        out.append(f"**{grand} finding(s)** — {badge}")
    out += [
        "",
        "| Contract | 🔴 Critical | 🟠 Medium | 🟡 Minor | 🔵 Enhancement |",
        "|---|--:|--:|--:|--:|",
    ]
    for crate, cr, me, mi, en in rows:
        out.append(f"| `{crate}` | {cr} | {me} | {mi} | {en} |")

    if findings:
        out += [
            "",
            "<details><summary>All findings</summary>",
            "",
            "| Detector | Location | Message |",
            "|---|---|---|",
        ]
        for crate, det, msg, fp, line in findings:
            loc = f"`{fp}{':' + line if line else ''}`" if fp else ""
            out.append(f"| `{det}` | {loc} | {msg} |")
        out += ["", "</details>"]

    print("\n".join(out))

if __name__ == "__main__":
    main()
