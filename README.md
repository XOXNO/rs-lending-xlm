# XOXNO Lending

[![CI](https://img.shields.io/github/actions/workflow/status/XOXNO/rs-lending-xlm/tests.yml?label=CI&style=flat-square)](https://github.com/XOXNO/rs-lending-xlm/actions/workflows/tests.yml) ![Rust](https://img.shields.io/badge/Rust-1.95-orange?style=flat-square) ![Stellar Soroban](https://img.shields.io/badge/Stellar-Soroban-blue?style=flat-square)

Smart contracts and deploy tooling for **XOXNO Lending**: a non-custodial, over-collateralized money market on [Stellar](https://stellar.org) (Soroban).

## Protocol overview

Suppliers deposit Stellar Asset Contracts (SACs) and earn interest. Borrowers take loans against collateral under LTV and health-factor limits. Liquidators close underwater positions. Governance changes markets, oracles, risk parameters, and upgrades through a timelock.

Liquidity sits in one central pool. Markets and risk are configured separately so an asset can be listed in isolation (separate indexes, cash, revenue, and bad debt) without splitting the pool implementation.

| Role | What they do |
|------|----------------|
| **Suppliers** | Deposit supported assets; earn interest; withdraw when utilization and account health allow |
| **Borrowers** | Borrow against collateral within LTV and health-factor limits |
| **Liquidators** | Close undercollateralized positions |
| **Governance** | Schedule and execute config for markets, oracles, risk, and upgrades |

Mechanics that enforce that model:

- **Hubs** — market keys that isolate the same underlying into separate markets when required  
- **Spokes** — risk profiles (LTV, liquidation thresholds, caps, pause/freeze) bound to account groups  
- **Oracles** — Reflector, RedStone, and the Xoxno adapter; dual-source tolerance; fail closed on bad or stale prices  
- **Governance** — propose → wait delay → execute; GUARDIAN can halt immediately; resume is timelocked  
- **Ops** — multisig-compatible owner, canceller roles, recovery path for canceller deadlock; see [DEPLOYMENT.md](./DEPLOYMENT.md)

Rules and accounting: [architecture/INVARIANTS.md](./architecture/INVARIANTS.md). Contract map: [SCF_BUILD_ARCHITECTURE.md](./SCF_BUILD_ARCHITECTURE.md).

## Documentation

| Document | Audience |
|----------|----------|
| [DEPLOYMENT.md](./DEPLOYMENT.md) | Operators (deploy, configure, day-2) |
| [SCF_BUILD_ARCHITECTURE.md](./SCF_BUILD_ARCHITECTURE.md) | Engineers and auditors (topology, storage) |
| [architecture/INVARIANTS.md](./architecture/INVARIANTS.md) | Anyone changing accounting, risk, oracle, or auth |
| [architecture/decisions/](./architecture/decisions/README.md) | Design decisions and trade-offs |
| [CONTRIBUTING.md](./CONTRIBUTING.md) | Contributors |
| [architecture/DOC_STYLE.md](./architecture/DOC_STYLE.md) | Rustdoc / public ABI comment style |
| [certora/README.md](./certora/README.md) | Formal verification |
| [STRIDE.md](./STRIDE.md) | Threat model |
| [SECURITY.md](./SECURITY.md) | Vulnerability disclosure |

## Security

| Layer | What exists in this repo |
|-------|--------------------------|
| **Design** | [INVARIANTS](./architecture/INVARIANTS.md), [ADRs](./architecture/decisions/README.md), [STRIDE](./STRIDE.md) |
| **Testing** | Crate tests, Soroban harness (`make test`), live testnet scripts, fuzz |
| **Formal** | [Certora](./certora/README.md) specs on critical properties |
| **Static** | Clippy, Scout, CI on pull requests |
| **Report** | **security@xoxno.com** only — [SECURITY.md](./SECURITY.md) |

Do **not** open public issues or PRs for vulnerabilities.

## Repository layout

```text
contracts/          Soroban contracts
  controller/       Accounts, risk, oracle, liquidation, strategies
  pool/             Liquidity and flash loans (controller-owned)
  governance/       Timelock and roles
  swap-aggregator/  DEX routing for strategies
  price-aggregator/ Oracle authority (hard/soft reads)
  xoxno-oracle/     Multi-signer RedStone / SEP-40 feed
  defindex-strategy/
common/             Shared math, types, errors
interfaces/         Client ABIs
configs/            Network and market deploy inputs (`networks.json`)
tests/              Harness, fuzz, live scenarios
services/           Keeper (TTL), metrics exporter
certora/            Formal verification
architecture/       Invariants, ADRs, DOC_STYLE
```

Resolve contract addresses from `configs/networks.json`. Do not hardcode them in integrators.

## Development

**Needs:** Rust from [rust-toolchain.toml](./rust-toolchain.toml) (`wasm32v1-none`), Stellar CLI with Soroban support.

```bash
git clone https://github.com/XOXNO/rs-lending-xlm.git
cd rs-lending-xlm
cargo test --workspace
make build
make help
```

| Task | Command |
|------|---------|
| Compile contracts | `make build` |
| Optimized WASM | `make optimize` |
| Crate tests | `cargo test --workspace` |
| Integration harness | `make test` |
| Lint / format | `make clippy`, `make fmt` |
| Deploy / ops | `make testnet setup` — [DEPLOYMENT.md](./DEPLOYMENT.md) |

Keeper and exporter are separate Cargo workspaces under `services/`.

## License

[PolyForm Noncommercial 1.0.0](./LICENSE). Commercial use needs a written agreement with XOXNO.

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md). Protocol changes must keep [INVARIANTS.md](./architecture/INVARIANTS.md) and ship verification that matches the risk of the change.
