# AGENTS.md

## Cursor Cloud specific instructions

This is a Rust / Stellar **Soroban** smart-contract workspace (XOXNO Lending) plus
two off-chain service workspaces. See `README.md`, `CONTRIBUTING.md`, and
`docs/tutorials/01-build-and-test.md` for the canonical commands; notes below are
only the non-obvious environment caveats.

### Toolchain / tools (already provided by the update script + snapshot)
- Rust `1.95` with targets `wasm32v1-none` and `wasm32-unknown-unknown` is pinned
  by `rust-toolchain.toml` and auto-selected in this repo.
- `stellar` CLI (v27.0.0) is installed to `~/.local/bin` (via
  `.github/scripts/install-stellar-cli.sh`). `~/.profile` puts `~/.local/bin` on
  PATH for **login** shells. In a non-login shell where `stellar` is missing,
  run `export PATH="$HOME/.local/bin:$PATH"` first — `make build` calls `stellar`.
- System packages `pkg-config` and `libssl-dev` are installed (persisted in the
  snapshot); the `services/` binaries link OpenSSL and fail to build without them.

### Critical ordering gotcha
- The integration harness loads `target/wasm32v1-none/release/pool.wasm` at
  runtime and panics with "Run 'make build' first" if it is absent. **Always run
  `make build` before `cargo test --workspace`, `make test`, or `make test-pool`.**

### Build / test / lint (contracts workspace)
- Build WASM: `make build` (wraps `stellar contract build`).
- Unit tests: `cargo test --workspace`. Integration harness: `make test`
  (single-threaded, ~5 min). Pool-only: `make test-pool`.
- Lint gate (matches CI `tests.yml`): `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo fmt --all -- --check` is documented in `CONTRIBUTING.md` but is **not**
  run by the `tests.yml` CI job and may report diffs depending on the rustfmt
  build; clippy is the enforced gate.

### Off-chain services (separate Cargo workspaces, not part of the root workspace)
- Keeper (TTL): `cargo test --manifest-path services/keeper/Cargo.toml`
  (binaries: `keeper-bot`, `inspect_ttls`, `prepay_rent`, `prove_permissionless`).
- Exporter (Prometheus): `cargo test --manifest-path services/lending-exporter/Cargo.toml`.
- Both need a live Soroban RPC endpoint + YAML config to actually run against a
  network; `--help` and their test suites work offline.
