#!/usr/bin/env python3
"""Convert Scout JSON reports to one SARIF 2.1.0 file for the IDE.

Native scout --output-format sarif is broken in the pinned rev (0-byte file);
scout-local.sh emits JSON and this converts it for SARIF Viewer
(ms-sarifvscode.sarif-viewer).

Usage:
    scout-sarif.py --root <repo_root> <report.json> [<report.json> ...] > out.sarif

No third-party dependencies.
"""
import argparse
import json
import os
import re
import sys

# Scout "[SEVERITY] msg" prefix → SARIF level.
LEVEL = {
    "CRITICAL": "error",
    "MEDIUM": "warning",
    "MINOR": "note",
    "ENHANCEMENT": "note",
}
_PREFIX = re.compile(r"^\s*\[([A-Z]+)\]\s*(.*)$", re.S)


def parse_point(token):
    """`file.rs:LINE:COL` or `LINE:COL` → (line, col), 1-based; None on failure."""
    parts = token.strip().split(":")
    if len(parts) < 2:
        return None
    try:
        return int(parts[-2]), int(parts[-1])
    except ValueError:
        return None


def region_from_span(span):
    """`a.rs:35:5 - 37:54` → SARIF region dict, or None."""
    if not span or " - " not in span:
        return None
    left, right = span.split(" - ", 1)
    start = parse_point(left)
    end = parse_point(right)
    if not start:
        return None
    region = {"startLine": start[0], "startColumn": start[1]}
    if end:
        region["endLine"] = end[0]
        end_col = end[1]
        if end[0] == start[0] and end_col <= start[1]:
            end_col = start[1] + 1
        region["endColumn"] = end_col
    return region


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", required=True, help="Repository root for absolute file URIs")
    ap.add_argument("reports", nargs="+", help="Scout JSON report files")
    args = ap.parse_args()
    root = os.path.abspath(args.root)

    results = []
    rules = {}
    for path in args.reports:
        try:
            with open(path, encoding="utf-8") as fh:
                data = json.load(fh)
        except (OSError, json.JSONDecodeError) as exc:
            print(f"scout-sarif: skipping {path}: {exc}", file=sys.stderr)
            continue
        for finding in data.get("findings", []):
            rule_id = finding.get("vulnerability_id") or "scout"
            raw = finding.get("error_message", "") or ""
            m = _PREFIX.match(raw)
            sev, text = (m.group(1), m.group(2)) if m else ("MEDIUM", raw)
            rules.setdefault(rule_id, {
                "id": rule_id,
                "name": rule_id,
                "shortDescription": {"text": text[:120] or rule_id},
                "properties": {"category": finding.get("category_id", "")},
            })
            file_path = finding.get("file_path", "")
            uri = "file://" + os.path.join(root, file_path) if file_path else ""
            loc = {"physicalLocation": {"artifactLocation": {"uri": uri}}}
            region = region_from_span(finding.get("span", ""))
            if region:
                loc["physicalLocation"]["region"] = region
            result = {
                "ruleId": rule_id,
                "level": LEVEL.get(sev, "warning"),
                "message": {"text": text or rule_id},
                "locations": [loc],
            }
            snippet = finding.get("code_snippet")
            if snippet and region:
                loc["physicalLocation"]["region"]["snippet"] = {"text": snippet}
            results.append(result)

    sarif = {
        "version": "2.1.0",
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "runs": [{
            "tool": {"driver": {
                "name": "Scout",
                "informationUri": "https://github.com/CoinFabrik/scout-audit",
                "rules": list(rules.values()),
            }},
            "results": results,
        }],
    }
    json.dump(sarif, sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
