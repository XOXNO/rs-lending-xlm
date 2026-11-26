# test-harness

This crate is the in-process Soroban integration layer for the lending protocol. Inside a `soroban-sdk` test `Env`, the harness registers the controller natively (`env.register(controller::Controller, …)`), deploys the pool and position-nft from built WASM, wires mock oracles and aggregators, and exposes a fluent API for supply, borrow, liquidation, and strategy flows.

The controller is native, so these tests do not exercise its compiled artifact or its WASM-level ABI.

Run `make build` from the repo root first. The pool and position-nft `.wasm` files must exist.
The [Curve collateral tests](tests/curve_collateral.rs) also load the production price-aggregator and swap-aggregator WASM. `PRICE_AGGREGATOR_WASM_PATH` and `SWAP_AGGREGATOR_WASM_PATH` override their default build paths.

Default runs disable Soroban budget metering. Opt in with `LendingTest::new().with_budget_enabled()` when you test resource limits.

## Library layout

`src/` holds `LendingTest`, builders, user ops, mocks, assertions, and the optional `BigRational` liquidation reference. The smaller helper modules are `admin.rs`,
`assert.rs`, `context.rs`, `errors.rs`, `flash_loan.rs`, `multi_hub.rs`,
`presets.rs`, `revenue.rs`, `script_runner.rs`, `time.rs`, `view.rs`,
`freezable_token.rs`, `weird_token.rs`, `helpers/`, and `receivers/`.

```text
src/
  setup/builder.rs     LendingTestBuilder — markets, spokes, position limits, budget
  core/                LendingTest runtime, market/user state types
  ops/                 supply, borrow, withdraw, repay, account helpers
  oracle/              Oracle config builders, RedStone/xoxno adapters, runtime price helpers
  strategy/            swap payloads, strategy actions, Blend migration helpers
  fixtures.rs          Canonical multi-market presets and seed helpers
  liquidation.rs       Liquidation call helpers
  keeper.rs            Index sync, bad-debt cleanup
  reference/           Exact-rational liquidation reference (feature `reference-math`, default on)
  mock_*.rs            Reflector, RedStone, aggregator, Blend, SAC stand-ins
  prelude.rs           Re-exports, surfaced at the crate root
```

### Entry point

```rust
use test_harness::*;

let mut t = LendingTest::new()
    .standard_two_asset()
    .build();

t.supply(ALICE, "USDC", 10_000.0);
t.borrow(ALICE, "ETH", 1.0);
```

`LendingTest::new()` returns a `LendingTestBuilder`. Chain `with_market(preset)`, fixture helpers (`standard_two_asset`, `three_asset_usdc_eth_wbtc`, and others), then `.build()`.

`.build()` creates its `Env` with `new_env()`. Use `new_env()` and `env_from_ledger_snapshot()` for any other test `Env`. They write no `test_snapshots/` JSON file when the `Env` is dropped. The SDK default writes the whole ledger and auth tree of every test, and that made the suite about 3× slower. Set `HARNESS_TEST_SNAPSHOTS=1` to write the files again.

### Features

| Feature | Default | Purpose |
|---------|---------|---------|
| `reference-math` | on | `test_harness::reference` for liquidation differential tests |
| `testing` | off | Forwards controller and governance `testing`. The harness dependencies already enable both. Set by the libFuzzer crate (`tests/fuzz`) |

## Tests

`tests/` holds domain-grouped scenario binaries (controller, governance, oracle, pool, strategy, composition, fuzz, meta), single-file binaries, and the `tests/smoke_test.rs` fast gate. Binaries, run commands, naming rules, and fixtures are in [`tests/README.md`](tests/README.md). Proptest properties are in [`tests/fuzz/README.md`](tests/fuzz/README.md) (this crate's `tests/fuzz/`, not the repo-root libFuzzer crate).

See also [`../fuzz/`](../fuzz/README.md) for libFuzzer math and protocol campaigns and [`../../certora/`](../../certora/README.md) for formal verification specs.
