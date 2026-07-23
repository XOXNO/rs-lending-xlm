# Tutorial: build and test

Goal: clone the repo, compile contracts, and run the default test surface.

## Prerequisites

- Rust matching [rust-toolchain.toml](../../rust-toolchain.toml)
- Target `wasm32v1-none` (`rustup target add wasm32v1-none`)
- [Stellar CLI](https://developers.stellar.org/docs/tools/cli) with Soroban support

## Steps

```bash
git clone https://github.com/XOXNO/rs-lending-xlm.git
cd rs-lending-xlm
cargo test --workspace
make build
make test
```

`cargo test --workspace` runs crate unit tests. `make build` produces WASM under
`target/wasm32v1-none/release/`. `make test` runs the Soroban integration
harness (needs a built `pool.wasm`).

## Check your outcome

- Workspace tests exit 0.
- `ls target/wasm32v1-none/release/pool.wasm` exists.
- Harness finishes without failure.

## Next

- Day-2 / deploy: [deploy and operate](../how-to/deploy-and-operate.md)
- Rules before changing accounting or risk: [invariants](../reference/invariants.md)
- Contribution checks: [CONTRIBUTING.md](../../CONTRIBUTING.md)
