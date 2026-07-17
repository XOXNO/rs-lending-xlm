#!/usr/bin/env python3
"""Render Scout JSON reports as a Markdown summary (stdout).

CI writes this to $GITHUB_STEP_SUMMARY; usable locally without SARIF Viewer.

Usage:
    scout-summary.py <dir-with-*.json>        # e.g. target/scout-audit
    scout-summary.py report1.json report2.json

No third-party dependencies.
"""
import glob
import json
import os
import sys

SEV_ORDER = ["critical", "medium", "minor", "enhancement"]
SEV_EMOJI = {"critical": "🔴", "medium": "🟠", "minor": "🟡", "enhancement": "🔵"}


def collect(args):
    files = []
    for a in args:
        files.extend(sorted(glob.glob(os.path.join(a, "*.json"))) if os.path.isdir(a) else [a])
    return [f for f in files if os.path.isfile(f)]


def span_line(span):
    """`a.rs:84:9 - 84:33` → `84`; '' on failure."""
    left = (span or "").split(" - ", 1)[0]
    parts = left.rsplit(":", 2)
    return parts[1] if len(parts) >= 3 else ""


def main():
    files = collect(sys.argv[1:])
    rows, findings, totals = [], [], {k: 0 for k in SEV_ORDER}
    for f in files:
        crate = os.path.basename(f)[:-5]
        try:
            with open(f, encoding="utf-8") as fh:
                data = json.load(fh)
        except (OSError, json.JSONDecodeError):
            continue
        by_sev = (data.get("summary", {}) or {}).get("by_severity", {}) or {}
        rows.append((crate, *[int(by_sev.get(s, 0)) for s in SEV_ORDER]))
        for s in SEV_ORDER:
            totals[s] += int(by_sev.get(s, 0))
        for x in data.get("findings", []):
            findings.append((
                crate,
                x.get("vulnerability_id", ""),
                (x.get("error_message", "") or "").split("] ", 1)[-1][:100],
                x.get("file_path", ""),
                span_line(x.get("span", "")),
            ))

    out = ["## 🔍 Scout Audit", ""]
    if not rows:
        out.append("_No Scout reports found._")
        print("\n".join(out))
        return

    grand = sum(totals.values())
    if grand == 0:
        out.append("✅ **No findings.**")
    else:
        badge = " · ".join(f"{SEV_EMOJI[s]} {totals[s]} {s}" for s in SEV_ORDER if totals[s])
        out.append(f"**{grand} finding(s)** — {badge}")
    out += ["", "| Contract | 🔴 Critical | 🟠 Medium | 🟡 Minor | 🔵 Enhancement |",
            "|---|--:|--:|--:|--:|"]
    for crate, cr, me, mi, en in rows:
        out.append(f"| `{crate}` | {cr} | {me} | {mi} | {en} |")

    if findings:
        out += ["", "<details><summary>All findings</summary>", "",
                "| Detector | Location | Message |", "|---|---|---|"]
        for crate, det, msg, fp, line in findings:
            loc = f"`{fp}{':' + line if line else ''}`" if fp else ""
            out.append(f"| `{det}` | {loc} | {msg} |")
        out += ["", "</details>"]

    print("\n".join(out))


if __name__ == "__main__":
    main()
