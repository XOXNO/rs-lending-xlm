# Architecture Decision Records

This directory contains Architecture Decision Records (ADRs) for architecturally significant
decisions in the XOXNO Lending protocol on Soroban. Each ADR records context,
decision, alternatives, consequences, and implementation paths.

## Index

| ADR  | Title                                                                                          | Status   |
| ---- | ---------------------------------------------------------------------------------------------- | -------- |
| 0001 | [Governance, Controller, and Central Pool Boundary](./0001-controller-pool-ownership-boundary.md) | Accepted |
| 0002 | [Per-Side Scaled-Balance Storage](./0002-per-side-scaled-balance-storage.md)                   | Accepted |
| 0003 | [Oracle Dual-Source Pricing With Tolerance Bands](./0003-oracle-dual-source-with-tolerance-bands.md) | Accepted |
| 0004 | [Cache Permissiveness Policy for Oracle Failures](./0004-cache-permissiveness-policy.md)       | Accepted |
| 0005 | [Validate Strategy Aggregator Output by Balance Delta](./0005-strategy-aggregator-output-validated-by-balance-delta.md) | Accepted |
| 0006 | [Flash-Loan Settlement by Pool Balance Snapshot](./0006-flash-loan-balance-snapshot.md)        | Accepted |
| 0007 | [Bad-Debt Socialization With Supply-Index Floor](./0007-bad-debt-socialization-with-index-floor.md) | Accepted |
| 0009 | [Mainnet Launch Hardening and Operational Control](./0009-mainnet-launch-hardening-and-operational-control.md) | Accepted |
| 0010 | [Governance Timelock for Protocol Admin](./0010-governance-timelock-for-controller-admin.md) | Accepted |

## Related Documents

- [Protocol invariants](../INVARIANTS.md)
- [Architecture reference](../../SCF_BUILD_ARCHITECTURE.md)
- [Security policy](../../SECURITY.md)
