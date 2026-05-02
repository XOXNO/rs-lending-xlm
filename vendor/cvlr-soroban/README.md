# CVLR for Soroban

Soroban-specific components of [CVLR](https://github.com/Certora/cvlr) (Certora Verification Language for Rust), used for writing and verifying Soroban smart contracts with the Certora's Sunbeam verifier.

## Overview

This repo provides Soroban-related utilities for writing specs for Certora's Sunbeam verifier.

## Getting Started

Add the crates you need to your `Cargo.toml`:

```toml
[dependencies]
cvlr-soroban = "0.4.0"
cvlr-soroban-derive = "0.4.0"
cvlr-soroban-macros = "0.4.0"
soroban-sdk = "=26.0.0-rc.1"
```

## Using Unreleased Versions

If you want to consume the latest unreleased version of a crate from this workspace, you can depend on it directly from GitHub instead of crates.io:

```toml
[dependencies]
cvlr-soroban = { git = "https://github.com/Certora/cvlr-soroban", branch = "main" }
cvlr-soroban-derive = { git = "https://github.com/Certora/cvlr-soroban", branch = "main" }
cvlr-soroban-macros = { git = "https://github.com/Certora/cvlr-soroban", branch = "main" }
```

## Building and Testing

Build the workspace:

```bash
cargo build --release
```

Run all tests:

```bash
cargo test
```

The proc-macro expansion tests in `cvlr-soroban-derive` use `macrotest` and `trybuild`. They require `cargo-expand`:

```bash
cargo install cargo-expand
```

Run the proc-macro test harness:

```bash
cargo test -p cvlr-soroban-derive --test test_contractevent
```

If you intentionally change macro expansion and want to refresh the snapshot files:

```bash
MACROTEST=overwrite cargo test -p cvlr-soroban-derive --test test_contractevent test_contractevent_macro_expansion
```

## Documentation and Related Repositories

- [CVLR](https://github.com/Certora/cvlr)
- [Soroban verification documentation](https://docs.certora.com/en/latest/docs/sunbeam/index.html) for how to write specs and run Sunbeam
- [Sunbeam tutorials](https://github.com/Certora/sunbeam-tutorials)

## License

This project is licensed under the MIT License.

## Release

Current release: `0.4.0`
