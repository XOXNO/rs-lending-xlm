# XOXNO Lending

[![CI](https://img.shields.io/github/actions/workflow/status/XOXNO/rs-lending-xlm/tests.yml?label=CI&style=flat-square)](https://github.com/XOXNO/rs-lending-xlm/actions/workflows/tests.yml)

XOXNO Lending is an over-collateralized money market built on Stellar Soroban.
Suppliers provide liquidity and earn interest. Borrowers use accepted
collateral to take loans within an account-specific risk regime. Liquidators
can reduce unhealthy positions, and governance manages the protocol through a
timelock.

## How the protocol is organized

| Concept | Role |
|---|---|
| Central pool | Custodies liquidity while preserving isolated accounting for every market |
| Markets | Track their own cash, shares, interest indexes, revenue, and rate policy |
| Accounts | Hold positions and bind once to a spoke, the account's risk regime |
| Controller | Applies authorization, price, risk, liquidation, and strategy rules |
| Price system | Supplies complete validated prices; risk-taking fails closed on invalid prices |
| Governance | Delays ordinary changes; emergency power can tighten protection but not reopen it |

The protocol uses RAY for interest and scaled shares, WAD for USD values and
health factor, and BPS for ratios and fees. See the formula reference for the
precise arithmetic and rounding policy.

## Documentation

| If you want to understand… | Start here |
|---|---|
| Protocol components, authority, and value flow | [Architecture](docs/reference/architecture.md) |
| Properties that must hold | [Runtime invariants](docs/reference/invariants.md) |
| Prices, interest, health, and liquidation math | [Formulas](docs/reference/formulas.md) |
| Threats, trust boundaries, and residual risks | [Threat model](docs/explanation/threat-model.md) |
| Why the design has this shape | [Decision records](docs/explanation/decisions/README.md) |
| How to contribute safely | [Contributing](CONTRIBUTING.md) |
| How to report a vulnerability | [Security policy](SECURITY.md) |

## Security

Protocol changes are security-sensitive. Do not report vulnerabilities in a
public issue, pull request, or discussion. Use the private process in
[SECURITY.md](SECURITY.md).

Before changing money movement, risk, authorization, prices, governance,
storage, or strategies, identify the affected invariant and add verification
that matches the change's risk.

## Development

Requirements:

- Rust from [rust-toolchain.toml](rust-toolchain.toml), including the
  wasm32v1-none target.
- Stellar CLI with Soroban support.

    git clone https://github.com/XOXNO/rs-lending-xlm.git
    cd rs-lending-xlm
    cargo test --workspace
    make build
    make help

| Task | Command |
|---|---|
| Build contracts | make build |
| Build optimized WASM | make optimize |
| Run workspace tests | cargo test --workspace |
| Run the integration harness | make test |
| Lint and format | make clippy, make fmt |
| View deployment and operations help | make help |

The keeper and lending exporter are separate Cargo workspaces. Network
configuration is environment-specific; resolve deployed addresses from the
active network configuration instead of hard-coding them.

## Repository guide

| Area | Purpose |
|---|---|
| Contracts | On-chain protocol components |
| Common and interfaces | Shared arithmetic, types, errors, and public client interfaces |
| Tests | Harness, fuzzing, and live scenarios |
| Services | Permissionless maintenance and operational metrics |
| Formal verification | Specifications and proof-oriented checks |
| Docs | Auditor and integrator documentation |

## License

Licensed under [PolyForm Noncommercial 1.0.0](LICENSE). Commercial use requires
a written agreement with XOXNO.
