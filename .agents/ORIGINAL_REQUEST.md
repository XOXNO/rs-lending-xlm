# Original User Request

## Initial Request — 2026-07-13T21:32:16Z

Audit fully the `@contracts/controller/src` codebase focusing on the Spoke architecture, its limits, configurations, endpoint checks, edge cases, state transitions, and weak points across all flag combinations.

Working directory: /Users/mihaieremia/GitHub/rs-lending-xlm/artifacts/spoke_audit
Integrity mode: development

## Requirements

### R1. Architecture & Configuration Mapping
Document the Spoke architecture including how `SpokeConfig`, `SpokeAssetConfig`, and `SpokeUsageRaw` structures are defined, stored, and retrieved.

### R2. Endpoint Verification & Check Matrix
Map each endpoint that reads or writes spoke configurations or checks spoke/asset flags (supply, borrow, withdraw, repay, liquidate, clean_bad_debt, strategy routes) and enumerate all checks, guards, and assertions.

### R3. Flag Combination & Flow Analysis
Analyze the effects of all flag combinations (spoke active/deprecated, asset paused/frozen, collateralizable, borrowable) across every flow, verifying which flows are allowed, blocked, or partially enabled.

### R4. Edge Case & Vulnerability Analysis
Perform a security and robustness analysis on potential edge cases, math rounding, overflow constraints, stale prices/oracle overrides, and state desynchronizations (e.g., between aggregated usage and actual user positions).

## Verification Plan

### Agent-as-Judge
An independent auditor agent will review the generated audit report against the following rubric:
1. Does it cover all 6 primary endpoints: `supply`, `borrow`, `withdraw`, `repay`, `liquidate`, and `clean_bad_debt`?
2. Does it map out a complete matrix of all flag combinations and their effects?
3. Does it reference specific codebase files and line ranges/symbols for every check it documents?
4. Does it identify at least 3 non-trivial edge cases/weak points (e.g., mathematical boundaries, state desync risks)?

## Acceptance Criteria

### Completeness & Accuracy
- [ ] Every endpoint check is traced to its exact Rust code location with a markdown file link.
- [ ] The flag combinations matrix covers at least:
  - Spoke deprecated vs. active
  - SpokeAsset paused vs. unpaused
  - SpokeAsset frozen vs. unfrozen
  - SpokeAsset collateralizable/borrowable flags
- [ ] The report details the math behind Spoke caps (supply/borrow caps) and how rounding is handled.
- [ ] The final output is stored in `/Users/mihaieremia/GitHub/rs-lending-xlm/artifacts/spoke_audit/spoke_audit_report.md`.
