# XOXNO Lending

[![CI](https://img.shields.io/github/actions/workflow/status/XOXNO/rs-lending-xlm/tests.yml?label=CI&style=flat-square)](https://github.com/XOXNO/rs-lending-xlm/actions/workflows/tests.yml) ![Rust](https://img.shields.io/badge/Rust-1.95-orange?style=flat-square) ![Stellar Soroban](https://img.shields.io/badge/Stellar-Soroban-blue?style=flat-square)

Smart contracts and deploy tooling for **XOXNO Lending**: an over-collateralized
money market on [Stellar](https://stellar.org) (Soroban).

Suppliers deposit SAC assets and earn interest. Borrowers take loans against
collateral under LTV and health-factor limits. Liquidators close underwater
positions. Governance changes markets, oracles, risk, and upgrades through a
timelock.

One central **pool** holds liquidity. Markets use `HubAssetKey { hub_id, asset }`
for isolation; **spokes** hold risk (LTV, caps, pause/freeze) per account group.
Oracles: Reflector, RedStone, and `xoxno-oracle`, with dual-source tolerance and
fail-closed reads. GUARDIAN can pause immediately; **unpause is timelocked**.

## Documentation

Start at **[docs/README.md](./docs/README.md)** (Diátaxis map).

| Document | Audience |
|----------|----------|
| [Tutorial: build and test](./docs/tutorials/01-build-and-test.md) | New contributors |
| [Deploy and operate](./docs/how-to/deploy-and-operate.md) | Operators |
| [Architecture](./docs/reference/architecture.md) | Engineers / auditors |
| [Invariants](./docs/reference/invariants.md) | Anyone changing accounting, risk, oracle, or auth |
| [ADRs](./docs/explanation/decisions/README.md) | Design trade-offs |
| [Threat model](./docs/explanation/threat-model.md) | Security review |
| [CONTRIBUTING.md](./CONTRIBUTING.md) | Contributors |
| [Doc style](./docs/reference/doc-style.md) | Rustdoc / public ABI comments |
| [skills/](./skills/README.md) | Integrator agent recipes |
| [certora/](./certora/README.md) | Formal verification |
| [SECURITY.md](./SECURITY.md) | Vulnerability disclosure |

Root redirects: [DEPLOYMENT.md](./DEPLOYMENT.md),
[SCF_BUILD_ARCHITECTURE.md](./SCF_BUILD_ARCHITECTURE.md),
[STRIDE.md](./STRIDE.md).

## Security

| Layer | In this repo |
|-------|----------------|
| **Design** | [Invariants](./docs/reference/invariants.md), [ADRs](./docs/explanation/decisions/README.md), [threat model](./docs/explanation/threat-model.md) |
| **Testing** | Crate tests, Soroban harness (`make test`), live testnet scripts, fuzz |
| **Formal** | [Certora](./certora/README.md) |
| **Static** | Clippy, Scout, CI |
| **Report** | **security@xoxno.com** only — [SECURITY.md](./SECURITY.md) |

Do **not** open public issues or PRs for vulnerabilities.

## Repository layout

```text
contracts/          Soroban contracts
  controller/       Accounts, risk, oracle, liquidation, strategies
  pool/             Liquidity and flash loans (controller-owned)
  governance/       Timelock and roles
  swap-aggregator/  DEX routing for strategies
  price-aggregator/ Oracle authority
  xoxno-oracle/     Multi-signer RedStone / SEP-40 feed
  defindex-strategy/
common/             Shared math, types, errors
interfaces/         Client ABIs
configs/            Network and market deploy inputs (`networks.json`)
docs/               Tutorials, how-tos, reference, explanation
tests/              Harness, fuzz, live scenarios
services/           Keeper (TTL), metrics exporter
certora/            Formal verification
skills/             Agent integration skills
```

Resolve contract addresses from `configs/networks.json`. Do not hardcode them.

## Development

**Needs:** Rust from [rust-toolchain.toml](./rust-toolchain.toml) (`wasm32v1-none`),
Stellar CLI with Soroban support.

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
| Deploy / ops | `make testnet setup` — [deploy guide](./docs/how-to/deploy-and-operate.md) |

Keeper and exporter are separate Cargo workspaces under `services/`.

## License

[PolyForm Noncommercial 1.0.0](./LICENSE). Commercial use needs a written
agreement with XOXNO.

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md). Protocol changes must preserve
[invariants](./docs/reference/invariants.md) and ship verification that matches
the risk of the change.
