# Architecture Decision Records

Decision records for the current design of XOXNO Lending on Soroban. Each ADR
states context, the decision as the code implements it, rejected alternatives,
and consequences. Runtime rules that must hold live in
[invariants.md](../../reference/invariants.md); why-level security reasoning
lives in the [threat model](../threat-model.md).

## Index

| ADR | Title | Status |
|-----|-------|--------|
| 0001 | [Governance → Controller → Pool ownership chain](./0001-governance-controller-pool-ownership-chain.md) | Accepted |
| 0002 | [One central pool, markets isolated by HubAssetKey](./0002-central-pool-hub-asset-key-isolation.md) | Accepted |
| 0003 | [RAY-scaled shares with directed rounding](./0003-ray-scaled-shares-directed-rounding.md) | Accepted |
| 0004 | [Dual-source oracle: tolerance band, midpoint blend, Partial-means-dead](./0004-dual-source-oracle-tolerance-midpoint.md) | Accepted |
| 0005 | [Fail-closed price consumption stalls liquidation](./0005-fail-closed-price-consumption.md) | Accepted |
| 0006 | [Timelock: typed propose, raw execute, delete-on-execute, recovery reset](./0006-timelock-typed-propose-raw-execute.md) | Accepted |
| 0007 | [Guardian ratchet: halt immediately, resume through the timelock](./0007-guardian-ratchet.md) | Accepted |
| 0008 | [Halt flags gate liquidation legs: paused stalls, frozen does not](./0008-halt-flags-gate-liquidation-legs.md) | Accepted |
| 0009 | [Spokes as an immutable per-account risk binding](./0009-spokes-immutable-account-risk-binding.md) | Accepted |
| 0010 | [Flash loans repay by allowance under exact balance assertions](./0010-flash-loan-allowance-repayment.md) | Accepted |
| 0011 | [Untrusted swap router verified by balance deltas](./0011-untrusted-swap-router-balance-deltas.md) | Accepted |
| 0012 | [Bad debt socialized by supply-index write-down with a floor](./0012-bad-debt-supply-index-writedown.md) | Accepted |
| 0013 | [Token custody split: measured inbound deltas, outbound-only pool](./0013-token-custody-split-measured-deltas.md) | Accepted |
| 0014 | [Oracle admission: write-time attestation, independence, smoothing](./0014-oracle-admission-attestation-independence-smoothing.md) | Accepted |
| 0015 | [Caps in asset units at the live index; zero means zero](./0015-caps-asset-units-zero-means-zero.md) | Accepted |
| 0016 | [Per-millisecond rates with year-capped chunked accrual](./0016-per-millisecond-rates-chunked-accrual.md) | Accepted |
| 0017 | [Testing surfaces behind cargo features and WASM-export gates](./0017-testing-surfaces-behind-features.md) | Accepted |

## Related

- [Invariants](../../reference/invariants.md)
- [Architecture](../../reference/architecture.md)
- [Formulas](../../reference/formulas.md)
- [Threat model](../threat-model.md)
- [SECURITY.md](../../../SECURITY.md)
