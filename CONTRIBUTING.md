# Contributing

XOXNO Lending contains invariant-critical Soroban contracts. A good change is
small, reviewable, and accompanied by evidence that it preserves the relevant
protocol properties.

## Before you change code

Read the material that matches your change:

| Change | Read first |
|---|---|
| Protocol overview and local setup | [README.md](README.md) |
| Accounting, risk, and liquidation arithmetic | [Formulas](docs/reference/formulas.md) |
| Required protocol properties | [Runtime invariants](docs/reference/invariants.md) |
| Threats and trust boundaries | [Threat model](docs/explanation/threat-model.md) |
| Design rationale | [Decision records](docs/explanation/decisions/README.md) |
| Formal verification | [Certora guide](certora/README.md) |
| Vulnerability reporting | [SECURITY.md](SECURITY.md) |

For a large protocol change, open an issue before implementation to agree on
scope, migration implications, and verification expectations. Do not open a
public issue or pull request for a vulnerability.

## Set up

Install the Rust toolchain declared by the repository, the wasm32v1-none
target, and Stellar CLI with Soroban support.

    cargo test --workspace
    make build
    make test
    make help

The keeper and lending exporter have separate Cargo workspaces. Run their
checks from their own manifests and follow their local documentation.

## Working agreement

- Keep pull requests focused. Do not combine a protocol change with unrelated
  cleanup or formatting.
- Preserve the unit boundary: token amounts at transfers, WAD for USD and
  health factor, RAY for shares and rates, and BPS for ratios and fees.
- Identify changes to authorization, governance, price handling, storage,
  accounting, risk, liquidation, or external-call behavior explicitly.
- Update public documentation when behavior, an invariant, or a proof boundary
  changes.
- Never commit secrets, private keys, environment files, or local deployment
  state.

## Verification

Choose the smallest evidence set that covers the changed surface. More
risk-sensitive changes require more than a passing workspace build.

| Surface | Minimum evidence |
|---|---|
| Documentation or isolated non-protocol tooling | Relevant formatting or targeted test |
| Ordinary contract change | Format, lint, workspace tests, and matching harness tests |
| Arithmetic, accounting, risk, price, governance, storage, or strategy change | Above, plus focused regression or property test; fuzz, Miri, or Certora where applicable |
| Release-wide or cross-contract change | Full targeted matrix, artifact checks, and operational or migration review |

Every pull request should start with:

    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace

Then run the focused checks for the changed behavior:

    make test
    make test-pool
    make fuzz FUZZ_TIME=30
    make proptest PROPTEST_CASES=256
    make miri-common
    make certora-wasm

Not every command applies to every change. If a check is skipped, say why in
the pull request rather than implying it ran.

For repository-wide assurance, use the verification targets listed by
make help-verify. Scout, mutation testing, coverage, and broader Certora
profiles are intentionally heavier and should be selected by risk and scope.

## Pull request description

State:

1. What changed and why.
2. Which invariants, threat boundaries, or users are affected.
3. Tests, fuzzing, formal checks, and manual validation that ran.
4. Any configuration, deployment, migration, oracle, or operational follow-up.
5. Known limitations or work deliberately left out of scope.

A reviewer should be able to understand the safety argument without reconstructing
the entire change history.

## Self-hosted CI safety

Some workflows execute pull-request-controlled code on persistent self-hosted
runners. Repository administrators must require approval for workflow runs from
outside collaborators. This setting lives in the repository's GitHub Actions
configuration, not in workflow YAML.

Do not weaken pinned-action, least-privilege, or deployable-ABI safeguards to
make CI pass. Raise a maintainer discussion if a legitimate change needs a
different CI permission or execution model.

## Issues and reviews

Use public issues for reproducible bugs, documentation gaps, features, and
non-sensitive design discussion. Include the environment, expected result, and
observed result.

Review with evidence: explain the affected behavior, the adversarial case, and
the verification result. Be direct about uncertainty. Follow the
[Code of conduct](CODE_OF_CONDUCT.md) in every project space.
