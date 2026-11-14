# XOXNO Lending

[![CI](https://img.shields.io/github/actions/workflow/status/XOXNO/rs-lending-xlm/tests.yml?label=CI&style=flat-square)](https://github.com/XOXNO/rs-lending-xlm/actions/workflows/tests.yml)

XOXNO Lending is a collateralized lending protocol on Stellar Soroban.
Suppliers earn interest on the liquidity that borrowers use. Borrowers hold
debt against collateral. A controller enforces account risk, and one pool
holds every token.

## Features

| Feature | Summary |
|---|---|
| Hubs and spokes | A hub groups markets that share one book per asset. A spoke lists hub assets with its own risk parameters, caps and halt flags. |
| Accounts as NFTs | Each account is a position NFT. Owners can add delegates. |
| Account strategies | `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`, `flash_position` and `migrate_from_blend` run in one transaction, through the swap aggregator. |
| Flash loans | `flash_loan` lends pool cash to a receiver contract for one call. |
| Composable prices | The price aggregator combines Chainlink, RedStone, Reflector and XOXNO feeds. It also prices Aquarius and Curve LP shares. Dual-source keys must agree within a tolerance. |
| Liquidation and bad debt | Liquidation pays a bonus from a curve. Bad debt reduces the supply index of the affected market. |
| Governance | Typed proposals run after a tiered timelock. A guardian can only reduce risk: pause, tighten flags, or suspend a registrar. |
| Delegated curation | Registrars run their own hubs and spokes through the curator contract, under the same timelock. |

## Audits

| Auditor | Scope | Commit | Report |
|---|---|---|---|
| Runtime Verification, November 2026 | Core contracts and `common/` | `a2486b25` (`v1.1.0`) | [PDF](audits/2026-11-runtime-verification.pdf) |

[audits/README.md](audits/README.md) lists each finding and its status.

## Deployments

| Network | Addresses |
|---|---|
| Mainnet and testnet | [Contract addresses and ids](skills/xoxno-lending/addresses.md), generated from [`configs/networks.json`](configs/networks.json) |

These documents describe this source tree. Before you deposit funds, compare
the deployed WASM hashes in `configs/networks.json` with your own build.

## Documentation

Start with [Architecture](docs/reference/architecture.md). Auditors can then
read the threat model, invariants, formulas and design decisions. Integrators
can use the endpoint, event and error references.

| Question | Reference |
|---|---|
| Components, authority, custody | [Architecture](docs/reference/architecture.md) |
| Attacks, trust assumptions, residual risks | [Threat model](docs/explanation/threat-model.md) |
| Safety properties | [Invariants](docs/reference/invariants.md) |
| Units, rounding, interest, risk, numeric limits | [Formulas](docs/reference/formulas.md) |
| Design rationale | [Design decisions](docs/explanation/decisions.md) |
| Callable surface and authorization | [Endpoints](docs/reference/endpoints.md) |
| Event payloads and indexing | [Events](docs/reference/events.md) |
| Failure codes and causes | [Errors](docs/reference/errors.md) |
| Operator procedures | [Freeze a listing](docs/reference/runbooks/freeze-a-listing.md), [force-socialize bad debt](docs/reference/runbooks/force-socialize-bad-debt.md) |
| Formal verification | [Certora](certora/README.md), [Aeneas proofs](verification/aeneas/README.md) |
| Integration guides for agents and SDK users | [Skills](skills/README.md) |
| Per-contract detail | `contracts/<name>/README.md` |

## Development

Install the Rust toolchain and targets in [rust-toolchain.toml](rust-toolchain.toml).
Install the Stellar CLI. Then build the WASM contracts before you run tests.

```sh
git clone https://github.com/XOXNO/rs-lending-xlm.git
cd rs-lending-xlm
make build
make test
```

| Task | Command |
|---|---|
| Build the contract WASM | `make build` |
| Run the workspace tests | `make test` |
| Run the integration harness only | `make test-harness` |
| Check formatting / lint | `make fmt-check` / `make clippy` |
| Check documentation links and symbols | `make docs-check` |
| List every target | `make help` |

| Path | Contents |
|---|---|
| `contracts/` | Deployable contracts |
| `common/`, `interfaces/` | Shared math, types and client interfaces |
| `tests/`, `certora/`, `verification/` | Tests, fuzzers and formal proofs |
| `services/` | Keeper and metrics exporter, in separate workspaces |

Read [CONTRIBUTING.md](CONTRIBUTING.md) before you open a pull request.

## Security and license

Report vulnerabilities through [SECURITY.md](SECURITY.md). Do not open a public
issue. Rewards are paid through the
[XOXNO bug bounty on Immunefi](https://immunefi.com/bug-bounty/xoxno/information/).

Licensed under [PolyForm Noncommercial 1.0.0](LICENSE). Commercial use requires
a written agreement with XOXNO.
