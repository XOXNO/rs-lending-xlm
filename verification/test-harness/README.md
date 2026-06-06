# test-harness

In-process Soroban integration layer for the rs-lending-xlm protocol. It deploys controller and pool WASM inside a `soroban-sdk` test `Env`, wires mock oracles and aggregators, and exposes a fluent API for supply/borrow/liquidation/strategy flows used by integration and property tests.

Build contract WASM first: `make build` from the repo root.

## What it provides

| Piece | Location | Role |
|-------|----------|------|
| **Library** | `src/` | `LendingTest`, builders, user ops, mocks, assertions, optional `BigRational` liquidation reference |
| **Integration tests** | `tests/` | Domain-grouped scenario binaries (controller, pool, oracle, strategy, fuzz, meta) |
| **Smoke gate** | `tests/smoke_test.rs` | Fast end-to-end sanity check |

Default runs disable Soroban budget metering. Opt in with `LendingTest::new().with_budget_enabled()` when testing resource limits.

## Library layout

```text
src/
  setup/builder.rs     LendingTestBuilder — markets, e-mode, budget, auth mode
  core/                LendingTest runtime, market/user state types
  ops/                 supply, borrow, withdraw, repay, account helpers
  oracle/              reflector config + runtime price/oracle helpers
  strategy/            swap payloads, multiply/swap strategy actions
  fixtures.rs          Canonical multi-market presets and seed helpers
  liquidation.rs       Liquidation helpers and health-factor views
  keeper.rs            Index sync, bad-debt cleanup
  reference/           Exact-rational liquidation reference (feature `reference-math`, default on)
  mock_*.rs            Reflector, Redstone, aggregator, SAC stand-ins
  prelude.rs           Convenient re-exports for test authors
```

### Entry point

```rust
use test_harness::prelude::*;

let mut t = LendingTest::new()
    .standard_two_asset()   // builder extension from fixtures
    .build();

t.supply(ALICE, "USDC", 10_000.0);
t.borrow(ALICE, "ETH", 1.0);
```

`LendingTest::new()` returns a `LendingTestBuilder`. Chain `with_market(preset)`, fixture helpers (`standard_two_asset`, `three_asset_usdc_eth_wbtc`, …), then `.build()`.

### Features

| Feature | Default | Purpose |
|---------|---------|---------|
| `reference-math` | on | `test_harness::reference` for liquidation differential tests |
| `testing` | off | Controller `testing` feature (enabled by fuzz / libFuzzer consumers) |

## Running tests

Soroban env state is not thread-safe — always pass `--test-threads=1`.

```bash
cargo test -p test-harness -- --test-threads=1
cargo test -p test-harness --test smoke_test -- --test-threads=1
cargo test -p test-harness --test controller -- --test-threads=1
```

Makefile shortcuts: `make test`, `make test-one FILE=controller`, `make test-match PATTERN=liquidation`, `make proptest`.

Integration test layout, module inventory, naming rules, and fixtures: [`tests/README.md`](tests/README.md).

Property-based fuzz properties: [`tests/fuzz/README.md`](tests/fuzz/README.md).

## Related verification

| Path | Role |
|------|------|
| `verification/fuzz/` | libFuzzer targets (math + protocol byte-mutation campaigns) |
| `verification/certora/` | Formal verification specs |