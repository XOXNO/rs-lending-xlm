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
| Design rationale | [Decision records](docs/explanation/decisions.md) |
| Formal verification | [Certora guide](certora/README.md) |
| Vulnerability reporting | [SECURITY.md](SECURITY.md) |

For a large protocol change, open an issue before implementation to agree on
scope, migration implications, and verification expectations. Do not open a
public issue or pull request for a vulnerability.

## Set up

Install the Rust toolchain and targets from `rust-toolchain.toml`, and Stellar
CLI with `make install-stellar-cli`. Run `make build` before the tests: they
load contract WASM from `target/`.

    make build
    make test
    make help

The keeper (`services/keeper`) and the lending exporter
(`services/lending-exporter`) are separate Cargo workspaces. Run their checks
from their own manifests and follow their README files.

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

    make fmt-check
    make clippy
    make build
    make test
    make docs-check
    make access-control-check

Then run the focused checks for the changed behavior:

    make test-match PATTERN=<substring>
    make fuzz FUZZ_TIME=30
    make proptest PROPTEST_CASES=256
    make miri-common
    make certora-wasm

Not every command applies to every change. If a check is skipped, say why in
the pull request rather than implying it ran.

For repository-wide assurance, use the verification targets that
`make help-verify` lists. Scout, mutation testing, coverage, and the broader
Certora profiles are heavier; select them by risk and scope.

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
runners. The real boundary is the repository setting that requires approval
for workflow runs from outside collaborators. This setting lives in the
repository's GitHub Actions configuration, not in workflow YAML. Keep it on.

The workflow guard is a second layer. Every self-hosted job that can run on
`pull_request` carries
`github.event.pull_request.head.repo.full_name == github.repository` in its
`if`, and `.github/scripts/check_workflows.py` fails such a job without it. A
job whose `if` admits only push, schedule or `workflow_dispatch` events is
exempt. A `pull_request` run uses the workflow files of the pull request
itself, so the guard protects only a fork pull request that leaves
`.github/**` unchanged. A maintainer must never approve a fork run that
changes `.github/**`.

A fork pull request skips the self-hosted gates and gets a failing
`Fork pull request` check. That check runs on a GitHub-hosted runner and checks
out no code. To run the gates, a maintainer who has read the change pushes its
branch to this repository. GitHub reports a skipped job as successful, so
administrators should make both the `Static Gates` and the `Fork pull request`
checks required on `main`. They should also require actions pinned to a
full-length commit SHA.

The release build job runs on a GitHub-hosted runner, restores no cache, and
builds with `--locked`. Its attested WASM therefore comes only from the
checked-out commit (a tag, or a branch on a dry run) and the committed
`Cargo.lock`.

Pin every action to a full commit SHA with a trailing `# <ref>` comment. The
Static Gates job runs on every same-repository pull request and fails on an
unpinned action. The stellar-cli installer checks the release tarball against a
pinned SHA-256, so a `STELLAR_VERSION` bump needs the new digests in
`.github/scripts/install-stellar-cli.sh`.

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
