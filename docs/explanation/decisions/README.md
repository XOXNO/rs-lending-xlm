# Architecture decision records

These records explain the consequential choices behind the current protocol.
They complement the [architecture](../../reference/architecture.md),
[invariants](../../reference/invariants.md), [formulas](../../reference/formulas.md),
and [threat model](../threat-model.md).

| Decision | Subject |
|---|---|
| [0001](0001-governance-controller-pool-ownership-chain.md) | Governance, controller, and pool ownership |
| [0002](0002-central-pool-hub-asset-key-isolation.md) | Central custody and market isolation |
| [0003](0003-ray-scaled-shares-directed-rounding.md) | Scaled shares and rounding |
| [0004](0004-dual-source-oracle-tolerance-midpoint.md) | Dual-source price agreement |
| [0005](0005-fail-closed-price-consumption.md) | Fail-closed price use |
| [0006](0006-timelock-typed-propose-raw-execute.md) | Timelocked administration |
| [0007](0007-guardian-ratchet.md) | Emergency ratchet |
| [0008](0008-halt-flags-gate-liquidation-legs.md) | Halt semantics (proposed `no_seize → frozen` amendment, not shipped) |
| [0009](0009-spokes-immutable-account-risk-binding.md) | Immutable account risk regime |
| [0010](0010-flash-loan-allowance-repayment.md) | Flash-loan settlement |
| [0011](0011-untrusted-swap-router-balance-deltas.md) | Untrusted route execution |
| [0012](0012-bad-debt-supply-index-writedown.md) | Bad-debt socialization |
| [0013](0013-token-custody-split-measured-deltas.md) | Measured token receipt |
| [0014](0014-oracle-admission-attestation-independence-smoothing.md) | Oracle source admission |
| [0015](0015-caps-asset-units-zero-means-zero.md) | Literal asset caps |
| [0016](0016-per-millisecond-rates-chunked-accrual.md) | Bounded interest accrual |
| [0017](0017-testing-surfaces-behind-features.md) | Release-safe testing surfaces |
| [0018](0018-compact-instruction-payload-registry-indices.md) | Compact route payloads |
| [0019](0019-share-credit-liquidation.md) | Share-credit liquidation |
| [0020](0020-flash-position-callback-multiply.md) | Zero-fee flash-position callback |
