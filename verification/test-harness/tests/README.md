# Integration tests

Contract-level scenarios for the lending protocol, executed in-process via the `test-harness` library. Each domain is one Cargo test binary (`tests/<domain>/main.rs`).

**Prerequisite:** `make build` (pool and controller WASM must exist).

## Binaries

| `--test` | Directory | Coverage |
|----------|-----------|----------|
| `smoke_test` | `smoke_test.rs` | Supply, borrow, liquidate, interest, e-mode, revenue |
| `controller` | `controller/` | Positions, supply/borrow/repay/withdraw, liquidation, admin, e-mode, isolation, flash loan, keeper, views |
| `oracle` | `oracle/` | Tolerance bands, staleness, dual-source, TWAP, Redstone, DEX USD repricing |
| `pool` | `pool/` | Interest curves, rewards, revenue, pool math |
| `strategy` | `strategy/` | Multiply, swap collateral/debt, router guards, happy paths, edge cases |
| `fuzz` | `fuzz/` | Proptest properties — see [`fuzz/README.md`](fuzz/README.md) |
| `meta` | `meta/` | Footprint, budget breakdown, chaos/stress sims, invariants, reentrancy, TTL |

## Module inventory

### `controller/`

`account`, `admin`, `admin_config`, `bad_debt_index`, `borrow`, `decimal_diversity`, `dust_floor`, `emode`, `emode_liquidation_combo`, `events`, `flash_loan`, `isolation`, `keeper`, `liquidation`, `liquidation_boundary`, `liquidation_coverage`, `liquidation_math`, `liquidation_mixed_decimal`, `max_utilization`, `ownership`, `repay`, `siloed_borrow`, `supply`, `validation_admin`, `views`, `withdraw`

### `oracle/`

- `tolerance/` — `bands`, `staleness`, `edge`, `config`, `dual_source`
- `twap`, `redstone`, `dex_usd_repricing`

### `pool/`

`interest`, `interest_rigorous`, `math_rates`, `pool_coverage`, `pool_revenue_edge`, `revenue`, `rewards`

### `strategy/`

`core`, `happy`, `router`, `edge/` (`multiply`, `rejections`, `swap`)

### `meta/`

`account_ttl_regression`, `bench_liquidate_max_positions`, `budget_breakdown`, `chaos_simulation`, `economic_attacks`, `footprint_test`, `invariant`, `lifecycle_regression`, `reentrancy_matrix`, `stress_simulation`, `utils`

## Test naming

```text
test_<entry>_<condition>_<expected>
```

| Segment | Meaning | Examples |
|---------|---------|----------|
| **entry** | API or subsystem | `supply`, `borrow`, `liquidate`, `multiply` |
| **condition** | Setup or input | `zero_amount`, `exceeding_ltv`, `stale_twap_history` |
| **expected** | Outcome | `rejects`, `allows`, `creates_position` |

Use `try_*` helpers plus `assert_contract_error` for expected failures.

## Running

```bash
# All integration + property tests
cargo test -p test-harness -- --test-threads=1

# One binary
cargo test -p test-harness --test controller -- --test-threads=1
cargo test -p test-harness --test oracle   -- --test-threads=1
cargo test -p test-harness --test pool     -- --test-threads=1
cargo test -p test-harness --test strategy -- --test-threads=1
cargo test -p test-harness --test fuzz     -- --test-threads=1
cargo test -p test-harness --test meta     -- --test-threads=1

# Fast gate
cargo test -p test-harness --test smoke_test -- --test-threads=1

# Filter by test name (works across binaries when unscoped)
cargo test -p test-harness smoke -- --test-threads=1
cargo test -p test-harness --test controller test_supply_rejects_zero -- --test-threads=1
cargo test -p test-harness --test fuzz prop_accounting_conservation -- --test-threads=1

# Makefile
make test-one FILE=controller
make test-match PATTERN=liquidation
```

### Proptest

Properties live in `fuzz/`. Case count is controlled by `PROPTEST_CASES` (Makefile default: `256`). Use release builds for long runs.

```bash
make proptest PROPTEST_CASES=256
make proptest-one TEST=prop_accounting_conservation PROPTEST_CASES=1000
PROPTEST_CASES=10000 cargo test --release -p test-harness --test fuzz -- --test-threads=1
```

Minimized failure seeds are committed as `fuzz/*.proptest-regressions`.

## Fixtures

Shared builders and seeds live in `src/fixtures.rs` and are re-exported from `tests/fixtures/mod.rs` in each binary.

```rust
mod fixtures;
use fixtures::{seed_liquidatable_usdc_eth, LendingTest, ALICE};
```

| API | Description |
|-----|-------------|
| `LendingTest::new().standard_two_asset()` | USDC + ETH markets, default reflector oracle |
| `LendingTest::new().standard_two_asset_dust_disabled().build()` | Two-asset book with dust floors off |
| `LendingTest::new().dual_source_two_asset()` | Built book with dual-source safe prices on USDC/ETH |
| `LendingTest::new().three_asset_usdc_eth_wbtc()` | USDC + ETH + WBTC |
| `LendingTest::new().three_asset_usdc_eth_wbtc_with_budget()` | Three-asset book with Soroban budget limits on |
| `liquidatable_usdc_eth()` | Built USDC/ETH market with liquidatable Alice position |
| `seed_liquidatable_usdc_eth(t)` | Alice: 10k USDC, 3 ETH debt, USDC at $0.50 |
| `seed_fuzz_conservation_book(t)` | Two-user seed for accounting conservation properties |
| `seed_standard_liquidity(t)` | Alice USDC supply, Bob ETH supply |
| `seed_liquidator_usdc(t, amount)` | Fund liquidator wallet |

Builder knobs: `with_dust_disabled_all_markets()`, `with_max_utilization_disabled_all_markets()`, `without_auto_auth()`, `with_budget_enabled()`, `with_market(preset)`, `with_market_config`, `with_position_limits`.

Example:

```rust
let mut t = LendingTest::new().standard_two_asset().build();
t.supply(ALICE, "USDC", 10_000.0);

let mut t = LendingTest::new().dual_source_two_asset();
t.supply(ALICE, "USDC", 10_000.0);

seed_liquidatable_usdc_eth(&mut t);
```

## Library reference

Crate root: [`../README.md`](../README.md). Public API surface: `test_harness::prelude::*` or granular imports from `test_harness::{LendingTest, …}`.