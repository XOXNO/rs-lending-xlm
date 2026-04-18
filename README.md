# XOXNO Lending

A multi-asset lending and borrowing protocol for the Stellar network, built
on Soroban. Users supply collateral, borrow against it, and earn yield from
borrowers' interest. Operators list assets and configure risk parameters
through a single controller contract.

## Capabilities

- **Lend & borrow** across listed assets with isolated, e-mode, or normal
  account modes.
- **Liquidations** that protect supplier solvency through a deterministic
  health-factor cascade.
- **Flash loans** with pool-side reserve verification and atomic repayment.
- **Strategy primitives** — leveraged positions, debt swaps, collateral
  swaps — routed through an operator-set aggregator.
- **Protocol revenue** that accrues with the supply index and forwards
  directly to a treasury accumulator on claim.

## Status

Pre-audit. Internal review and remediation are complete — see
[`audit/`](./audit/). External audit by Runtime Verification and Certora
is the next milestone.

| Signal | Value |
|---|---|
| Tests | 691 passing, 0 failing |
| Coverage | 95.43% (in-scope crates) |
| Static analysis | `cargo clippy -D warnings` clean; `cargo audit` 0 vulnerabilities |
| Production targets | `controller`, `pool`, `pool-interface`, `common` |

## Documentation

Technical reference lives in [`architecture/`](./architecture/):

- [`ARCHITECTURE.md`](./architecture/ARCHITECTURE.md) — system design,
  controller-pool boundary, sequence diagrams.
- [`INVARIANTS.md`](./architecture/INVARIANTS.md) — protocol algebra,
  fixed-point conventions, solvency math.
- [`DEPLOYMENT.md`](./architecture/DEPLOYMENT.md) — operator runbook for
  building, deploying, and configuring.
- [`ACTORS.md`](./architecture/ACTORS.md) — privileges, trust boundaries,
  off-chain operator policy.
- [`ENTRYPOINT_AUTH_MATRIX.md`](./architecture/ENTRYPOINT_AUTH_MATRIX.md) —
  every public function with its auth gate, invariants, and pool calls.
- [`CONFIG_INVARIANTS.md`](./architecture/CONFIG_INVARIANTS.md) —
  every operator-set field, valid range, and enforcement site.
- [`STELLAR_NOTES.md`](./architecture/STELLAR_NOTES.md) — Soroban platform
  assumptions and unresolved questions.
- [`MATH_REVIEW.md`](./architecture/MATH_REVIEW.md) — Certora rule
  coverage and remediation status.

Audit artifacts live in [`audit/`](./audit/).

## Build

```bash
make build           # compile contracts to wasm32v1-none
make optimize        # stellar contract optimize
make test            # cargo test --workspace
make coverage-merged # combined controller + pool coverage report
```

See [`architecture/DEPLOYMENT.md`](./architecture/DEPLOYMENT.md) for
detailed tooling and deployment.

## License

[PolyForm Noncommercial 1.0.0](./LICENSE). Research, testing, security
review, and contributions are permitted. Commercial use requires a
written agreement with XOXNO; contact `license@xoxno.com`.

## Security

See [`SECURITY.md`](./SECURITY.md). Report vulnerabilities privately to
`security@xoxno.com`.
