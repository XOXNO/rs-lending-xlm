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

Use `make help` for the full local command surface.

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

For protocol-sensitive changes, also include the relevant verification output:

```bash
./certora/compile_all.sh
./certora/scripts/run_profile.py fast
make fuzz FUZZ_TIME=30
make proptest PROPTEST_CASES=256
```

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
- Which invariants or architecture decisions are affected.
- Which local checks, fuzzing, or formal-verification profiles were run.
- Any deployment, migration, governance, oracle, or operational follow-up.

## Issues

Use public issues for bugs, documentation gaps, feature requests, and
non-sensitive design discussion. Include reproducible steps, expected behavior,
actual behavior, environment details, and relevant logs.

Use **security@xoxno.com** for vulnerabilities.
