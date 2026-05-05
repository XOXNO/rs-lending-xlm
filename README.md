# XOXNO Lending

XOXNO Lending is a multi-asset lending protocol for Stellar/Soroban. The
protocol uses a controller-and-pool architecture: the controller owns account
state, risk checks, oracle validation, liquidations, and strategy entrypoints;
each pool owns liquidity, indexes, scaled balances, and reserve accounting for
one asset.

The repository is structured for protocol review, local development, testing,
and formal-verification work. It is not a documentation site.

## Table of Contents

- [Documentation](#documentation)
- [Architecture](#architecture)
- [Design Model](#design-model)
- [Repository Structure](#repository-structure)
- [Dependencies](#dependencies)
- [Quickstart](#quickstart)
- [Development](#development)
- [Verification](#verification)
- [Security](#security)
- [License](#license)
- [Contributing](#contributing)

## Documentation

- [Protocol invariants](./architecture/INVARIANTS.md) - fixed-point domains,
  accounting invariants, solvency rules, liquidation constraints, and security
  properties.
- [Security policy](./SECURITY.md) - private vulnerability reporting process,
  scope, response timeline, and safe harbor.
- [SCF build architecture](./SCF_BUILD_ARCHITECTURE.md) - grant-facing system
  architecture and implementation context.

## Architecture

The protocol separates user-facing risk coordination from asset-specific
liquidity accounting.

- **Controller**: account lifecycle, position validation, oracle reads,
  collateral/debt accounting, liquidations, isolated mode, e-mode, keeper
  operations, and strategy orchestration.
- **Pool**: one deployed pool per listed asset. Pools manage supply and borrow
  indexes, scaled balances, liquidity transfers, flash-loan reserve checks,
  bad-debt absorption, and protocol revenue accrual.
- **Common**: shared fixed-point math, protocol types, events, errors, and
  constants used by deployed contracts.
- **Pool interface**: cross-contract ABI between controller and pools.
- **Test harness**: end-to-end scenarios, mixed-decimal coverage, fuzz-style
  property tests, and regression fixtures.

## Design Model

- **Scaled balance accounting**: user balances enter accounting in RAY
  precision and are stored as scaled amounts against supply or borrow indexes.
- **Price and solvency domain**: oracle prices, USD values, LTV, liquidation
  thresholds, and health-factor calculations use WAD precision.
- **Token boundary discipline**: asset-decimal conversion is reserved for token
  transfers, refunds, events, and views.
- **Risk modes**: accounts can operate in normal, isolated, or e-mode
  configurations, with validation enforced by the controller.
- **Liquidation model**: liquidations are health-factor gated, cap repayment by
  actual debt, seize collateral proportionally, and charge protocol fees only
  on the bonus portion.
- **Revenue model**: protocol revenue accrues through pool-side scaled supply
  accounting and is claimed through the controller to the configured
  accumulator.

## Repository Structure

```text
rs-lending-xlm/
├── common/           # Shared math, types, events, constants, and errors
├── controller/       # Account, risk, oracle, liquidation, and strategy logic
├── pool/             # Asset pool, indexes, liquidity, revenue, flash loans
├── pool-interface/   # Cross-contract interface used by controller
├── verification/     # Certora specs, integration harness, fuzzing, corpora
├── architecture/     # Core invariants and protocol architecture material
├── configs/          # Coverage scripts and deployment/config inputs
└── vendor/           # Pinned local dependencies used by the workspace
```

## Dependencies

Required:

- Rust toolchain from [rust-toolchain.toml](./rust-toolchain.toml).
- Stellar CLI for Soroban contract builds and deployment commands.
- `wasm32v1-none` support through the Stellar contract toolchain.

Optional:

- `cargo-llvm-cov` for coverage reports.
- Certora Soroban tooling for prover runs.
- `cargo-audit` and `cargo clippy` for local security and quality checks.

## Quickstart

```bash
git clone https://github.com/XOXNO/rs-lending-xlm.git
cd rs-lending-xlm

cargo test --workspace
make build
```

## Development

```bash
make build              # Build controller and pool WASM artifacts
make optimize           # Optimize WASM artifacts for deployment
cargo test --workspace  # Run the complete Rust test suite
make test               # Run the integration harness serially
make test-pool          # Run pool unit tests
make fmt                # Format the workspace
make clippy             # Run clippy checks
make coverage-merged    # Generate merged controller, pool, and harness coverage
```

Deployment and operator commands are exposed through the Makefile. They require
the Stellar CLI, configured network settings, and a funded signer.

## Verification

The repository includes unit tests, integration scenarios, mixed-decimal
coverage, fuzz-style property tests, and Certora specification sources.

Recommended local checks before review:

```bash
cargo test --workspace
cargo check -p common -p pool -p controller
./verification/certora/compile_all.sh
```

Coverage reports can be generated with:

```bash
make coverage-merged
```

Certora prover profiles and rule-authoring guidance are documented in
[`verification/certora/README.md`](verification/certora/README.md).

The protocol is pre-audit. External audit artifacts will be linked from this
repository once published.

## Security

Do not open public issues or pull requests for vulnerabilities. Report security
issues privately to `security@xoxno.com`.

See [SECURITY.md](./SECURITY.md) for scope, response timelines, safe harbor,
and coordinated-disclosure policy.

## License

This repository is licensed under the
[PolyForm Noncommercial 1.0.0](./LICENSE). Research, testing, security review,
and contributions are permitted. Commercial use requires a written agreement
with XOXNO; contact `license@xoxno.com`.

## Contributing

Contributions should preserve the protocol's accounting, authorization, oracle,
and solvency invariants. Before opening a pull request, run the verification
commands above and include any relevant risk, test, or migration notes.
