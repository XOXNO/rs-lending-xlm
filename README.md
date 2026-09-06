# XOXNO Lending

[![CI](https://img.shields.io/github/actions/workflow/status/XOXNO/rs-lending-xlm/tests.yml?label=CI&style=flat-square)](https://github.com/XOXNO/rs-lending-xlm/actions/workflows/tests.yml)

XOXNO Lending is a collateralized lending protocol on Stellar Soroban.
Suppliers earn interest on borrowed liquidity. Borrowers hold debt against
collateral, with account risk enforced by a controller and tokens held in a
central pool.

## Documentation

Start with [Architecture](docs/reference/architecture.md) for markets, accounts,
and contract responsibilities. Auditors can continue through the threat model,
invariants, formulas, and design rationale in the order below. Integrators can
then use the endpoint, event, and error references.

These documents describe this source tree. Deployed code, owners, and
configuration require separate verification.

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
| Formal-model boundaries and prover diagnosis | [Certora tuning](docs/explanation/certora-sunbeam-prover-tuning.md) |

## Development

Install the Rust toolchain and targets in [rust-toolchain.toml](rust-toolchain.toml)
and Stellar CLI, then build the WASM contracts before running tests.

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
`common/`, and `interfaces/`. Verification lives in `tests/` and `certora/`.
The keeper and exporter use separate service workspaces. See
[Contributing](CONTRIBUTING.md) for change and review requirements.

## Security and license

Report vulnerabilities through [SECURITY.md](SECURITY.md), not public issues.
Money movement, authorization, risk, and storage changes require verification
of their affected invariants.

Licensed under [PolyForm Noncommercial 1.0.0](LICENSE). Commercial use requires
a written agreement with XOXNO.
