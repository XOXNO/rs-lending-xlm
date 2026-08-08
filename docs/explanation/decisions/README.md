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
| 0002 | [One central pool, markets isolated by `HubAssetKey`](./0002-central-pool-hub-asset-key-isolation.md) | Accepted |
| 0003 | [RAY-scaled shares with directed rounding that favors the protocol](./0003-ray-scaled-shares-directed-rounding.md) | Accepted |
| 0004 | [Dual-source oracle: reciprocal tolerance band, midpoint blend, Partial-means-dead](./0004-dual-source-oracle-tolerance-midpoint.md) | Accepted |
| 0005 | [Fail-closed price consumption on every mutating path](./0005-fail-closed-price-consumption.md) | Accepted |
| 0006 | [Timelock with typed propose, raw execute, delete-on-execute, and a non-cancellable recovery reset](./0006-timelock-typed-propose-raw-execute.md) | Accepted |
| 0007 | [Guardian ratchet: halting is immediate, resuming is timelocked](./0007-guardian-ratchet.md) | Accepted |
| 0008 | [Halt flags gate liquidation legs: paused stalls liquidation, frozen does not](./0008-halt-flags-gate-liquidation-legs.md) | Accepted |
| 0009 | [Spokes as an immutable per-account risk binding, chosen once at account creation](./0009-spokes-immutable-account-risk-binding.md) | Accepted |
| 0010 | [Flash loans repay by allowance, verified by exact balance assertions, with reentrancy blocked one layer up](./0010-flash-loan-allowance-repayment.md) | Accepted |
| 0011 | [The swap router is untrusted: routes computed off-chain, verified on-chain by balance deltas](./0011-untrusted-swap-router-balance-deltas.md) | Accepted |
| 0012 | [Bad debt is socialized by writing the supply index down, with a hard floor](./0012-bad-debt-supply-index-writedown.md) | Accepted |
| 0013 | [Token custody split: the controller pre-transfers and credits measured deltas; the pool only transfers out](./0013-token-custody-split-measured-deltas.md) | Accepted |
| 0014 | [Composable oracle sources are admitted only through write-time attestation, independence, and smoothing policy](./0014-oracle-admission-attestation-independence-smoothing.md) | Accepted |
| 0015 | [Caps are declared in asset units, converted at the live index per call, and zero means zero](./0015-caps-asset-units-zero-means-zero.md) | Accepted |
| 0016 | [Interest rates are per-millisecond RAY values, accrued in year-capped chunks of a truncated exponential series](./0016-per-millisecond-rates-chunked-accrual.md) | Accepted |
| 0017 | [Test and verification surfaces live behind cargo features, guarded by WASM-export build gates](./0017-testing-surfaces-behind-features.md) | Accepted |
| 0018 | [The swap payload is a packed instruction stream over address and amount registries](./0018-compact-instruction-payload-registry-indices.md) | Accepted |

## Related

- [Invariants](../../reference/invariants.md)
- [Architecture](../../reference/architecture.md)
- [Formulas](../../reference/formulas.md)
- [Threat model](../threat-model.md)
- [SECURITY.md](../../../SECURITY.md)
