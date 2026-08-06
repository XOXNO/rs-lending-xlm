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

| Document | Audience |
|----------|----------|
| [docs/README.md](./docs/README.md) | Docs map |
| [Formulas](./docs/reference/formulas.md) | Risk, HF, liquidation math (code-matched) |
| [CONTRIBUTING.md](./CONTRIBUTING.md) | Contributors |
| [skills/](./skills/README.md) | Integrator agent recipes |
| [certora/](./certora/README.md) | Formal verification |
| [SECURITY.md](./SECURITY.md) | Vulnerability disclosure |
| Contract READMEs under `contracts/*/` | Per-crate entrypoints and layout |

Protocol behavior is defined by the contracts, interfaces, and tests. Start
with [skills/lending-protocol-fundamentals](./skills/lending-protocol-fundamentals/SKILL.md)
for the shared model (hubs, spokes, units, HF, pause matrix), and
[docs/reference/formulas.md](./docs/reference/formulas.md) for equations.

## Security

| Layer | In this repo |
|-------|----------------|
| **Design** | [Formulas](./docs/reference/formulas.md), contract rustdoc, skills fundamentals |
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
mock/               Test-only contracts (oracles, flash-loan receiver)
common/             Shared math, types, errors
interfaces/         Client ABIs
configs/            Network and market deploy inputs (`networks.json`)
docs/               Formula reference and docs map
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
| Deploy / ops | `make testnet setup` — see `make help` and `configs/` |

Keeper and exporter are separate Cargo workspaces under `services/`.

## License

[PolyForm Noncommercial 1.0.0](./LICENSE). Commercial use needs a written
agreement with XOXNO.

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md). Protocol changes must preserve
on-chain invariants (accounting, risk, oracle, auth)—see
[formulas](./docs/reference/formulas.md)—and ship verification that matches
the risk of the change.
