# Contributing

Thank you for helping improve XOXNO Lending. This repository contains
pre-audit Soroban smart contracts, so contributions need to preserve protocol
invariants and include enough verification evidence for reviewers to evaluate
the change.

## Before You Start

- Read [README.md](./README.md) for the repository layout and command surface.
- Read [architecture/INVARIANTS.md](./architecture/INVARIANTS.md) before
  changing accounting, authorization, oracle, liquidation, flash-loan, or risk
  logic.
- Read [SECURITY.md](./SECURITY.md) before reporting any vulnerability. Do not
  open public issues or pull requests for security problems.
- For large protocol changes, open an issue first so maintainers can confirm
  scope, design constraints, and verification expectations.

## Development Setup

Install:

- Rust from [rust-toolchain.toml](./rust-toolchain.toml).
- `wasm32v1-none` for the configured Rust toolchain.
- Stellar CLI with Soroban contract support.

Build and test locally:

```bash
cargo test --workspace
make build
make test
make test-pool
```

The `services/keeper` is a separate workspace:

```bash
cargo test --manifest-path services/keeper/Cargo.toml
```

Use `make help` for the full local command surface (build, optimize, test layers, certora, fuzz, proptest, mutants, miri, coverage, scout, etc.).

## Change Guidelines

- Keep changes focused. Avoid unrelated formatting, refactors, generated
  artifacts, or dependency movement in the same pull request.
- Preserve documented invariants and update the relevant architecture notes when
  behavior changes.
- Use explicit fixed-point domains: token-native amounts at token boundaries,
  WAD for USD values and health factor, and RAY for rates and indexes.
- Keep authorization and role changes visible in the pull request description.
- Update tests, fuzz targets, Certora specs, or architecture documentation when
  the change affects a verified behavior surface.
- Do not commit secrets, private keys, `.env` files, or local deployment state.

## Pull Requests

Before requesting review, run the checks that match your change:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
make test
make test-pool
```

For protocol-sensitive changes (accounting, risk, oracle, liquidation, flash loans, strategies, governance, storage), also run the relevant parts of the verification surface (see `SCF_BUILD_ARCHITECTURE.md §14` and `INVARIANTS.md` verification matrix):

```bash
make certora-wasm            # before Certora runs
./certora/scripts/run_profile.py fast
make fuzz FUZZ_TIME=30
make proptest PROPTEST_CASES=256
make mutants
make miri-common
```

See `make help` and the full test layers in `README.md` and `SCF_BUILD_ARCHITECTURE.md`.

To mirror CI locally, use [nektos/act](https://github.com/nektos/act) (runs
workflows in Docker, same step order as GitHub):

```bash
brew install act          # once; Docker must be running
.github/scripts/act-local.sh list
.github/scripts/act-local.sh -n ci       # dry-run
.github/scripts/act-local.sh ci          # ci.yml build-and-test job
.github/scripts/act-local.sh ci --full   # + security-scan (slow)
make act-ci-dryrun                       # Makefile shortcuts
make act-ci
```

Runner image mappings live in `.actrc` at the repo root (`self-hosted` →
`catthehacker/ubuntu:act-latest`). Certora workflows need
`.github/act/.secrets` (see `.github/act/.secrets.example`).

Each pull request should explain:

- What changed and why.
- Which invariants (see `INVARIANTS.md`) or architecture decisions (see `architecture/decisions/`) are affected.
- Which local checks, fuzzing, Certora profiles, or other verification were run (reference the matrix in `SCF_BUILD_ARCHITECTURE.md §14` when relevant).
- Any deployment, migration, governance, oracle, or operational follow-up.

All changes must preserve the rules in `INVARIANTS.md`. The live implementation facts (controller owns accounts/risk/oracle/strategies and is the sole caller of the pool; 3-layer pause/freeze matrix; fail-closed oracle with Xoxno as distinct provider; scaled balances + index monotonicity except bad-debt floor; etc.) are the ground truth.

## CI/CD Security

### Reviewer-approval gate for self-hosted PR jobs

Several jobs run PR-controlled code (build scripts, tests, Makefile targets, Scout,
fuzz smoke, Miri) on a persistent self-hosted runner. A malicious pull request
could otherwise execute arbitrary code on that runner and read caches, tools, or
runner-local state.

The gate is GitHub's native fork pull request approval setting, not a deployment
environment.

#### Required one-time repo setup (admin, GitHub Settings — cannot be done in YAML)

1. **Settings → Actions → General → Fork pull request workflows from outside
   collaborators** → set to *Require approval for all outside collaborators* (or
   *for first-time contributors*, if internal contributors should run without
   approval).

With this set, a PR from a fork/outside collaborator pauses in the Actions tab
until a maintainer approves the run.

### Other hardening in place

- Third-party actions are pinned to immutable commit SHAs (e.g. `scout-audit`), not
  mutable tags.
- Workflows declare least-privilege `permissions:` (`contents: read` for PR jobs;
  the release e2e job is scoped to `contents: write` only).
- `make wasm-size-check` runs `wasm-testing-abi-check`, which fails the build if the
  deployable `governance.wasm` ever exports the test-only `set_controller` ABI.

## Issues

Use public issues for bugs, documentation gaps, feature requests, and
non-sensitive design discussion. Include reproducible steps, expected behavior,
actual behavior, environment details, and relevant logs.

Use **security@xoxno.com** for vulnerabilities.
