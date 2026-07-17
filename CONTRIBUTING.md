# Contributing

Thanks for helping improve XOXNO Lending. This repo holds **invariant-critical**
Soroban contracts. Changes must preserve protocol rules and leave enough
evidence for reviewers.

## Before you start

- [README.md](./README.md) — layout and commands
- [architecture/INVARIANTS.md](./architecture/INVARIANTS.md) — before touching accounting, auth, oracle, liquidation, flash loans, or risk
- [SECURITY.md](./SECURITY.md) — vulnerabilities go to **security@xoxno.com**, never public issues/PRs
- Large protocol changes: open an issue first for scope and verification expectations

## Setup

- Rust from [rust-toolchain.toml](./rust-toolchain.toml)
- Target `wasm32v1-none`
- Stellar CLI (Soroban)

```bash
cargo test --workspace
make build
make test
make test-pool
make help
```

Separate workspaces:

```bash
cargo test --manifest-path services/keeper/Cargo.toml
# lending-exporter: see services/lending-exporter/README.md
```

## Change guidelines

- Keep PRs focused (no drive-by format or unrelated refactors).
- Preserve INVARIANTS; update architecture notes when behavior changes.
- Fixed-point: token-native at transfers, WAD for USD/HF, RAY for rates/indexes.
- Call out auth/role changes in the PR description.
- Add or update tests / fuzz / Certora / docs when you change a verified surface.
- Never commit secrets, keys, `.env`, or local deploy state.

## Verification tiers

### Always (every PR)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Run the harness layers that match what you touched (`make test`, `make test-pool`).

### If you touch money, risk, oracle, gov, storage, or strategies

Add the relevant deeper checks (pick by surface; see [SCF §14](./SCF_BUILD_ARCHITECTURE.md#14-verification-surface)):

```bash
make certora-wasm
# then the Certora profile that covers your domain
make fuzz FUZZ_TIME=30          # or a single target
make proptest PROPTEST_CASES=256
make miri-common                # pure math changes
```

### Release / protocol-wide

Full matrix in SCF §14 (`mutants`, full Certora profiles, coverage, Scout, etc.).

### Mirror CI locally (optional)

```bash
# Docker + act; see .actrc
.github/scripts/act-local.sh list
make act-ci-dryrun
make act-ci
```

Certora under act needs `.github/act/.secrets` (see `.secrets.example`).

## Pull request body

State:

1. What changed and why  
2. Which invariants / ADRs are affected  
3. Which checks you ran (tier above)  
4. Any deploy, migration, oracle, or ops follow-up  

All changes must preserve [INVARIANTS.md](./architecture/INVARIANTS.md). Live facts
(controller/pool boundary, pause matrix, fail-closed oracle, scaled balances) are
in the contracts and SCF — not in outdated prose.

## CI security (self-hosted PR jobs)

Some jobs run PR-controlled code on a persistent self-hosted runner. **Required
repo setting (admin UI, not YAML):**

**Settings → Actions → General → Fork pull request workflows from outside
collaborators** → *Require approval for all outside collaborators* (or first-time
contributors).

Also in place: third-party actions pinned by SHA, least-privilege
`permissions:`, and `wasm-testing-abi-check` so deployable governance WASM does
not export test-only ABIs.

## Issues

Public issues: bugs, docs gaps, features, non-sensitive design. Include repro
steps, expected vs actual, environment.

Vulnerabilities: **security@xoxno.com** only.
