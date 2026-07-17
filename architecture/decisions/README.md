# Architecture Decision Records

Architecturally significant decisions for XOXNO Lending on Soroban. Each ADR
states context, the decision, alternatives, and consequences. References point
at the implementing code. Runtime properties that all changes must preserve live
in [INVARIANTS.md](../INVARIANTS.md).

## Index

| ADR | Title | Status |
|-----|-------|--------|
| 0001 | [Controller / pool ownership boundary](./0001-controller-pool-ownership-boundary.md) | Accepted |
| 0002 | [Per-side scaled-balance storage](./0002-per-side-scaled-balance-storage.md) | Accepted |
| 0003 | [Oracle dual-source with tolerance bands](./0003-oracle-dual-source-with-tolerance-bands.md) | Accepted |
| 0004 | [Oracle policy by flow](./0004-cache-permissiveness-policy.md) | Accepted |
| 0005 | [Strategy aggregator output by balance delta](./0005-strategy-aggregator-output-validated-by-balance-delta.md) | Accepted |
| 0006 | [Flash-loan balance snapshot](./0006-flash-loan-balance-snapshot.md) | Accepted |
| 0007 | [Bad-debt socialization with supply-index floor](./0007-bad-debt-socialization-with-index-floor.md) | Accepted |
| 0009 | [Mainnet launch hardening](./0009-mainnet-launch-hardening-and-operational-control.md) | Accepted |
| 0010 | [Governance timelock for protocol admin](./0010-governance-timelock-for-controller-admin.md) | Accepted |
| 0011 | [Pause and freeze matrix](./0011-pause-and-freeze-matrix.md) | Accepted |
| 0012 | [Per-spoke liquidation curve](./0012-per-spoke-liquidation-curve.md) | Accepted |

## Related

- [INVARIANTS.md](../INVARIANTS.md)  
- [SCF_BUILD_ARCHITECTURE.md](../../SCF_BUILD_ARCHITECTURE.md)  
- [SECURITY.md](../../SECURITY.md)  
