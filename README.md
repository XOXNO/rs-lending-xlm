# XOXNO Lending

[![CI](https://img.shields.io/github/actions/workflow/status/XOXNO/rs-lending-xlm/tests.yml?label=CI&style=flat-square)](https://github.com/XOXNO/rs-lending-xlm/actions/workflows/tests.yml)

A collateralized lending protocol on Stellar Soroban. Suppliers hold indexed
claims on market liquidity; borrowers hold debt against collateral. A
controller applies account and risk rules, a central pool holds tokens, and
configured governance controls administration.

## Documentation

For auditors, read architecture, threat model, invariants, formulas, then decisions.
These documents describe this source tree, not verified deployment state.

| Question | Reference |
|---|---|
| Components, authority, custody | [Architecture](docs/reference/architecture.md) |
| Attacks, trust assumptions, residual risks | [Threat model](docs/explanation/threat-model.md) |
| Safety properties and their limits | [Invariants](docs/reference/invariants.md) |
| Units, rounding, interest, risk, numeric limits | [Formulas](docs/reference/formulas.md) |
| Design rationale | [Decisions](docs/explanation/decisions.md) |
| Callable surface and authorization | [Endpoints](docs/reference/endpoints.md) |
| Event payloads and indexing | [Events](docs/reference/events.md) |
| Failure codes and causes | [Errors](docs/reference/errors.md) |
| Governance bad-debt cleanup | [Force-socialize runbook](docs/reference/runbooks/force-socialize-bad-debt.md) |
| Historical audit results and limitations | [Audit history](docs/audit/README.md) |
| Formal-model boundaries and prover diagnosis | [Certora tuning](docs/explanation/certora-sunbeam-prover-tuning.md) |

## Development

Install the Rust toolchain and targets in [rust-toolchain.toml](rust-toolchain.toml)
and Stellar CLI. See [Contributing](CONTRIBUTING.md) for the development workflow.

```sh
git clone https://github.com/XOXNO/rs-lending-xlm.git
cd rs-lending-xlm
make build
make test
make help
```

| Task | Command |
|---|---|
| Workspace tests, including required testing features | `make build`, then `make test` |
| Integration harness alone, after WASM build | `make test-harness` |
| Contract WASM build | `make build` |
| Formatting check / lint | `make fmt-check` / `make clippy` |
| Documentation links and symbol names | `make docs-check` |

Contracts, shared arithmetic, and client interfaces live in `contracts/`,
`common/`, and `interfaces/`; verification lives in `tests/` and `certora/`.
The keeper and exporter are separate service workspaces. Network configuration
must be matched to deployed addresses and artifacts before operational use.

## Security and license

Report vulnerabilities through [SECURITY.md](SECURITY.md), not public issues.
Money movement, authorization, risk, and storage changes require verification
of their affected invariants.

Licensed under [PolyForm Noncommercial 1.0.0](LICENSE). Commercial use requires
a written agreement with XOXNO.
