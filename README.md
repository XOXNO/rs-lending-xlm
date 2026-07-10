# XOXNO Lending

[![CI](https://img.shields.io/github/actions/workflow/status/XOXNO/rs-lending-xlm/tests.yml?label=CI&style=flat-square)](https://github.com/XOXNO/rs-lending-xlm/actions/workflows/tests.yml) ![Rust](https://img.shields.io/badge/Rust-1.95-orange?style=flat-square) ![Stellar Soroban](https://img.shields.io/badge/Stellar-Soroban-blue?style=flat-square)

A multi-asset lending protocol on Stellar Soroban, built around a single
central liquidity pool, hub-and-spoke market configuration, and timelocked
on-chain governance.

## Table of Contents

- [Documentation](#documentation)
- [Architecture](#architecture)
- [Repository Structure](#repository-structure)
- [Getting Started](#getting-started)
- [Development](#development)
- [Testing & Verification](#testing--verification)
- [Security](#security)
- [License](#license)
- [Contributing](#contributing)

## Documentation

- [Architecture reference](./SCF_BUILD_ARCHITECTURE.md) — topology, storage,
  and contract boundaries.
- [Protocol invariants](./architecture/INVARIANTS.md) — solvency, oracle, and
  accounting rules every change must preserve.
- [Architecture decisions](./architecture/decisions/README.md) — ADR index for
  protocol design choices.
- [Formal verification](./certora/README.md) — Certora proof domains, profiles,
  and prover commands.
- [Deployment & operations](./DEPLOYMENT.md) — runbook for deploying and
  operating the protocol.
- [Integration skills](./skills/README.md) — publishable AI-agent skills for
  third-party developers integrating or building on the protocol.

## Architecture

The protocol separates administration, risk logic, and liquidity into three
core contracts:

- **Governance** — owns the controller and timelocks all protocol-admin
  changes; pause and unpause remain immediate.
- **Controller** — the user-facing contract: accounts, risk checks, oracle
  validation, liquidations, flash loans, and strategies.
- **Pool** — a single central contract that holds all liquidity and per-market
  accounting.

Markets are keyed by hub and asset, so the same token can be listed
independently on different hubs; spokes bind user accounts to their risk
configuration. See the
[architecture reference](./SCF_BUILD_ARCHITECTURE.md) for the full design.

## Repository Structure

```text
rs-lending-xlm/
├── common/                  # Shared math, types, events, constants, errors
├── contracts/
│   ├── controller/          # Accounts, risk, oracle, liquidation, strategies
│   ├── governance/          # Timelocked protocol administration
│   ├── pool/                # Central pool accounting and flash loans
│   ├── aggregator/          # DEX aggregation router
│   ├── xoxno-oracle-adapter/# Multi-signer SEP-40 price-feed adapter
│   ├── defindex-strategy/   # Reference DeFindex vault strategy
│   └── flash-loan-receiver/ # Reference flash-loan receiver
├── interfaces/              # External ABI traits and generated clients
├── services/keeper/         # Off-chain keeper service (separate workspace)
├── certora/                 # Certora specs and harnesses
├── tests/                   # Integration harnesses and fuzz targets
├── architecture/            # Invariants, ADRs, and design material
└── configs/                 # Market, spoke, network, deployment inputs
```

## Getting Started

Requirements:

- Rust from [rust-toolchain.toml](./rust-toolchain.toml), including the
  `wasm32v1-none` target.
- Stellar CLI with Soroban contract support.

```bash
git clone https://github.com/XOXNO/rs-lending-xlm.git
cd rs-lending-xlm
cargo test --workspace
make build
```

Use `make help` for the full command surface.

## Development

- **Build WASM artifacts**: `make build`
- **Build optimized deployment binaries**: `make optimize`
- **Run workspace tests**: `cargo test --workspace`
- **Run Soroban integration tests**: `make test`
- **Lint and format**: `make clippy` and `make fmt`
- **Static analysis**: `scripts/scout-local.sh`

`services/keeper` is a separate workspace:

```bash
cargo test --manifest-path services/keeper/Cargo.toml
```

Deployment and day-to-day operations are Makefile-driven; see the
[deployment runbook](./DEPLOYMENT.md).

## Testing & Verification

Protocol correctness is enforced in independent layers:

- **Unit tests** — per-crate tests across all production contracts.
- **End-to-end integration tests** — full protocol flows against the Soroban
  test environment (`tests/test-harness`).
- **Live testnet end-to-end** — release scenarios run against live Stellar
  testnet, including liquidations and aggregator-routed strategies
  (`tests/integration`).
- **Invariant-driven fuzzing** — six `cargo-fuzz` targets covering protocol
  flows, strategies, pool accounting, rates, and fixed-point math
  (`tests/fuzz`).
- **Property-based tests** — randomized proptest suites for accounting
  conservation, auth gates, strategy safety, and liquidation math against an
  exact-rational reference (`make proptest`).
- **Mutation testing** — non-overlapping `cargo-mutants` scopes cover common,
  pool, governance, and controller production behavior (`make mutants`), with
  focused math/rates/pool-interest targets for local iteration.
- **Miri** — undefined-behavior checks on the pure fixed-point math core
  (`make miri-common`).
- **Formal verification** — Certora proofs over math, pool accounting,
  controller risk logic, oracle rules, flash loans, liquidation, and
  controller–pool consistency ([certora/](./certora/README.md)).
- **Static analysis** — Scout runs on every pull request and gates on
  critical findings (`scripts/scout-local.sh`).
- **Coverage** — merged controller, pool, and harness coverage reports
  (`make coverage`).
- **Threat modeling** — [STRIDE analysis](./STRIDE.md) alongside the
  [protocol invariants](./architecture/INVARIANTS.md).

The full verification surface is defined in
[SCF_BUILD_ARCHITECTURE.md](./SCF_BUILD_ARCHITECTURE.md#14-verification-surface).

## Security

Do not open public issues or pull requests for vulnerabilities. Report
security issues to `security@xoxno.com`. Safe-harbor terms and scope are in
[SECURITY.md](./SECURITY.md).

## License

This repository is licensed under
[PolyForm Noncommercial 1.0.0](./LICENSE). Commercial use requires a written
agreement with XOXNO.

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md). Protocol changes must preserve the
invariants in [INVARIANTS.md](./architecture/INVARIANTS.md) and include
relevant verification output.
