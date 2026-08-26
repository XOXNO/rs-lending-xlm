# test-harness

In-process Soroban integration layer for the rs-lending-xlm protocol. Inside a `soroban-sdk` test `Env` it registers the controller natively (`env.register(controller::Controller, …)`) and deploys the pool and position-nft from built WASM, wires mock oracles and aggregators, and exposes a fluent API for supply/borrow/liquidation/strategy flows used by integration and property tests. Because the controller is native, these tests do not exercise its compiled artifact or its WASM-level ABI.

Build contract WASM first: `make build` from the repo root — the pool and position-nft `.wasm` files must exist.

## What it provides

| Piece | Location | Role |
|-------|----------|------|
| **Library** | `src/` | `LendingTest`, builders, user ops, mocks, assertions, optional `BigRational` liquidation reference |
| **Integration tests** | `tests/` | Domain-grouped scenario binaries (controller, governance, oracle, pool, strategy, fuzz, meta) |
| **Smoke gate** | `tests/smoke_test.rs` | Fast end-to-end sanity check |

Default runs disable Soroban budget metering. Opt in with `LendingTest::new().with_budget_enabled()` when testing resource limits.

## Library layout

Main entry points. `src/` also holds smaller helper modules (`admin.rs`,
`assert.rs`, `errors.rs`, `flash_loan.rs`, `multi_hub.rs`, `presets.rs`,
`revenue.rs`, `time.rs`, `view.rs`, `helpers/`, `receivers/`).

```text
src/
  setup/builder.rs     LendingTestBuilder — markets, spoke, budget, auth mode
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
    .standard_two_asset()
    .build();

t.supply(ALICE, "USDC", 10_000.0);
t.borrow(ALICE, "ETH", 1.0);
```

`LendingTest::new()` returns a `LendingTestBuilder`. Chain `with_market(preset)`, fixture helpers (`standard_two_asset`, `three_asset_usdc_eth_wbtc`, …), then `.build()`.

### Features

| Feature | Default | Purpose |
|---------|---------|---------|
| `reference-math` | on | `test_harness::reference` for liquidation differential tests |
| `testing` | off | Controller and governance `testing` features (enabled by fuzz / libFuzzer consumers) |

## Running tests

Each test builds its own `Env` and writes its own `test_snapshots` file, so the
suite runs in parallel at libtest's default of one thread per core.

```bash
cargo test -p test-harness
cargo test -p test-harness --test smoke_test
cargo test -p test-harness --test controller
```

Pass `-- --test-threads=1` (or `make test TEST_THREADS=1`) to serialise while
bisecting a suspected cross-test interaction, and for readable `--nocapture`
output.

Makefile shortcuts: `make test`, `make test-one FILE=controller`, `make test-match PATTERN=liquidation`, `make proptest`.

Integration test layout, module inventory, naming rules, and fixtures: [`tests/README.md`](tests/README.md).

Proptest properties: [`tests/fuzz/README.md`](tests/fuzz/README.md) (this crate's `tests/fuzz/`, not the repo-root libFuzzer crate).

## Related verification

| Path | Role |
|------|------|
| [`../fuzz/`](../fuzz/README.md) (repo root `tests/fuzz/`) | libFuzzer targets (math + protocol byte-mutation campaigns) |
| [`../../certora/`](../../certora/README.md) | Formal verification specs |