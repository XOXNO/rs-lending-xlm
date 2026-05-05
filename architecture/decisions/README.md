# Architecture Decision Records

This directory contains Architecture Decision Records (ADRs) for the
load-bearing decisions in the XOXNO Lending protocol on Soroban. Each ADR
captures the context, the decision itself, alternatives considered, and
the consequences. The Rust source remains the authoritative specification;
ADRs explain *why* the implementation looks the way it does.

ADR format follows the Nygard/MADR convention: status, date, deciders,
context, decision, alternatives considered, consequences, and source
references keyed to module paths.

## Index

| ADR  | Title                                                                                          | Status   |
| ---- | ---------------------------------------------------------------------------------------------- | -------- |
| 0001 | [Controller / Pool Ownership Boundary](./0001-controller-pool-ownership-boundary.md)           | Accepted |
| 0002 | [Per-Side Scaled-Balance Storage](./0002-per-side-scaled-balance-storage.md)                   | Accepted |
| 0003 | [Oracle Dual-Source Pricing With Tolerance Bands](./0003-oracle-dual-source-with-tolerance-bands.md) | Accepted |
| 0004 | [Cache Permissiveness Policy for Oracle Failures](./0004-cache-permissiveness-policy.md)       | Accepted |
| 0005 | [Validate Strategy Aggregator Output by Balance Delta](./0005-strategy-aggregator-output-validated-by-balance-delta.md) | Accepted |
| 0006 | [Flash-Loan Settlement by Pool Balance Snapshot](./0006-flash-loan-balance-snapshot.md)        | Accepted |
| 0007 | [Bad-Debt Socialization With Supply-Index Floor](./0007-bad-debt-socialization-with-index-floor.md) | Accepted |
| 0008 | [Isolation and E-Mode Coexistence Model](./0008-isolation-and-emode-coexistence.md)            | Accepted |

## Conventions

- Filenames are `NNNN-kebab-case-title.md`, four-digit zero-padded id.
- Status values: `Proposed`, `Accepted`, `Superseded by ADR-NNNN`,
  `Deprecated`.
- An ADR is immutable after acceptance. New decisions create a new ADR
  that may supersede an older one. Edits to accepted ADRs are limited to
  fixing references to source paths after refactors and to recording
  status transitions in the header.
- New ADRs SHOULD link to the relevant `SCF_BUILD_ARCHITECTURE.md`
  section and to specific module paths so reviewers can trace decisions
  to code.

## Out of Scope

Per-function documentation lives in source. Invariant statements live in
[`../INVARIANTS.md`](../INVARIANTS.md). The high-level system map lives in
[`../../SCF_BUILD_ARCHITECTURE.md`](../../SCF_BUILD_ARCHITECTURE.md).
