# Integration tests

Contract-level scenarios run in-process via the `test-harness` library ([crate README](../README.md)). Each domain directory (`tests/<domain>/main.rs`) and each top-level `tests/*.rs` file is one Cargo test binary. Run `make build` first: the pool and position-nft WASM must exist.

## Binaries

Domain binaries live in `<name>/main.rs`; the rest are `<name>.rs`.

| `--test` | Coverage |
|----------|----------|
| `smoke_test` | Supply, borrow, withdraw, repay, liquidate, interest, spoke, revenue, multiply |
| `controller` | Positions, supply/borrow/repay/withdraw, liquidation, admin, spoke, flash loan, keeper, views |
| `governance` | Timelock operation states, immediate-path roles, permissionless execution, spoke, Redstone, DEX USD repricing, stale edits; forwarder input validation (market creation, asset config, IRM, position limits, oracle config/tolerance probing) |
| `oracle` | `tolerance/` (bands, config, dual source, edges, staleness), future timestamps and future-skew live path, TWAP, Redstone, XOXNO adapter, DEX USD repricing |
| `pool` | Interest curves, rate math, revenue, accrual partition bound, pool coverage |
| `strategy` | Multiply, flash position, swap collateral/debt, Blend migration, router guards, cross-hub loops, `edge/` cases, adversarial and happy paths |
| `composition` | One caller contract chains controller verbs in one invocation: atomic revert, delegate revocation, NFT transfer, repeated loops, supplier exit |
| `fuzz` | Proptest properties ([`fuzz/README.md`](fuzz/README.md)) |
| `meta` | TTL regressions, budget and memory attribution, footprint, chaos and stress sims, economic attacks, invariants, reentrancy matrix |
| `astra_audit` | Audit regression: exact scaled-share reconciliation across every position NFT, including credit-mode receivers |
| `pool_money_flow_audit` | Audit regression: pool money paths under a mocked owner (books, shared token custody, recapitalization refunds) |
| `curve_collateral` | Curve LP collateral against production price-aggregator and router WASMs and real Curve pool executables, under mainnet resource limits |
| `custom_account_auth` | Contract account authorizes the controller directly through the host |
| `poc_multiply_reentrancy` | Initial-payment token re-entering the controller during `multiply` aborts the call; no collateral or USDC moves |
| `strategy_origination_fee_parity` | `flash_position` opens the same position as `multiply`, without the strategy origination fee |
| `strategy_solvency_gate_rollback` | Solvency-gate rejection in `flash_position` reverts every earlier transfer |
| `zz_storage_sizing` | Prints per-account storage entry XDR sizes (`print_storage_sizes`) |

`tests/test-harness/Cargo.toml` declares no `[[test]]` sections, so cargo
discovers every binary.

## Module inventory

Each domain's `main.rs` declares its modules. List them with (swap `controller` for any domain):

```bash
grep -E '^ *(pub )?mod ' tests/test-harness/tests/controller/main.rs
```

### `controller/`

| Area | Modules |
|------|---------|
| Core verbs and reads | `supply`, `borrow`, `repay`, `withdraw`, `account`, `views`, `events`, `position_nft*` |
| Liquidation | `liquidation*`, `spoke_liquidation_combo`, `deprecated_spoke_liquidation_liveness` (roles below) |
| Bad debt | `bad_debt_*`, `force_socialize_refuses_solvent_accounts`, `same_market_bad_debt_cleanup_arithmetic` |
| Admin, ownership, keeper | `admin*`, `validation_admin`, `ownership`, `keeper`, `bulk_indexes`, risk restamps and governance edits between legs |
| Spokes and hubs | `spoke*`, `multi_hub` |
| Flash loans | `flash_loan*` |
| Amounts, decimals, limits | Dust floors, decimal mixes, extreme amounts, long horizons, utilization and borrow-collateral floors, round trips, transfer measurement |
| LIQVID listings | `liqvid_*` |
| External audit finding regressions | `audit_*`, `security_audit*` |
| Helpers | `shared` |

### Liquidation modules (roles)

| Module | Role |
|--------|------|
| `liquidation.rs` | Smoke: proportional/targeted seize, bonus tiers, bad-debt socialization; rejects healthy account, active flash loan, zero amount; allows paused controller, self-liquidation |
| `liquidation_coverage.rs` | Input validation: duplicate payments, empty/zero/unsupported assets, subunit collateral, multi-debt caps |
| `liquidation_math.rs` | Bonus formula, protocol fee on bonus only, bad-debt index delta, bounded seizure |
| `liquidation_boundary.rs` | HF exactly 1 vs just below, monotone bonus band, bad-debt trigger at collateral floor |
| `liquidation_mixed_decimal.rs` | Mixed collateral/debt decimals |
| `liquidation_and_borrow_exact_boundaries.rs` | HF exactly 1, borrow at the LTV limit, LTV collateral at the floor |
| `liquidation_accrual_timing.rs` | In-call index accrual at fixed prices gives the liquidator nothing |
| `liquidation_band_full_close.rs` | Full closes in the band `D <= C < D * (1 + base)`: merged offers, fee-on-transfer debt, over-offers |
| `liquidation_band_signed_auth.rs` | Band liquidations under enforced auth, with interest accrued between simulation and execution |
| `liquidation_extreme.rs` | Extreme curves: high target HF, flat bonus, narrow curve, high-LTV stablecoin |
| `liquidation_ratchet.rs` | Chained partial liquidations never extract more than one liquidation |
| `liquidation_seize_modes.rs` | `no_seize` pause split and share-credit liquidation (`SeizeMode::Credit`) |
| `liquidation_under_delivering_debt_token.rs` | Fee-on-transfer debt token scales both debt retired and collateral seized |
| `spoke_liquidation_combo.rs` | Spoke liquidation with category-specific LTV/threshold |

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

Each test builds its own `Env` and writes its own `test_snapshots` file, so the
suite runs in parallel at libtest's default of one thread per core.

```bash
cargo test -p test-harness                       # every binary
cargo test -p test-harness --test controller     # one binary (any `--test` above)
cargo test -p test-harness --test smoke_test     # fast gate

# Filter by test name (works across binaries when unscoped)
cargo test -p test-harness smoke
cargo test -p test-harness --test controller test_supply_rejects_zero

# Serialise while bisecting a suspected cross-test interaction, or for readable --nocapture output
cargo test -p test-harness -- --test-threads=1   # or: make test TEST_THREADS=1

make test-harness                                # this crate only
make test                                        # whole workspace
make test-one FILE=controller
make test-match PATTERN=liquidation
```

Proptest runs, `PROPTEST_CASES` and committed regression seeds: [`fuzz/README.md`](fuzz/README.md).

## Fixtures

Shared builders and seeds live in `src/fixtures.rs`; the crate root re-exports them (`src/prelude.rs`):

```rust
use test_harness::{seed_liquidatable_usdc_eth, LendingTest, ALICE};
```

| API | Description |
|-----|-------------|
| `LendingTest::new().standard_two_asset()` | USDC + ETH markets, default reflector oracle |
| `LendingTest::new().standard_two_asset_dust_disabled()` | Built two-asset book with dust floors off |
| `LendingTest::new().dual_source_two_asset()` | Built book with dual-source safe prices on USDC/ETH |
| `LendingTest::new().three_asset_usdc_eth_wbtc()` | USDC + ETH + WBTC |
| `LendingTest::new().stablecoin_spoke_two_asset()` | USDC + USDT, both listed on the stablecoin spoke (id 2) |
| `liquidatable_usdc_eth()` | Built USDC/ETH market with liquidatable Alice position |
| `seed_liquidatable_usdc_eth(t)` | Alice: 10k USDC, 3 ETH debt, USDC at $0.50 |
| `seed_band_usdc_eth(t)` | Alice: 10k USDC, 3 ETH debt, USDC at $0.62 (collateral covers the debt, not the base bonus) |
| `seed_fuzz_conservation_book(t)` | Two-user seed for accounting conservation properties |

Builder knobs: `with_min_borrow_collateral_disabled()` (LTV-weighted borrow collateral floor = 0), `with_max_utilization_disabled_all_markets()`, `with_budget_enabled()`, `with_market(preset)`, `with_market_config`, `with_position_limits`.

```rust
let mut t = LendingTest::new().dual_source_two_asset(); // already built
t.supply(ALICE, "USDC", 10_000.0);
seed_liquidatable_usdc_eth(&mut t);
```

## Library reference

The harness also pins live contract facts: the ownership chain
(`controller/ownership.rs`), the pause matrix
(`controller/security_audit_extended.rs`), multi-hub and spoke wiring
(`controller/multi_hub.rs`, `controller/spoke.rs`), the bad-debt floor
(`controller/bad_debt_index.rs`), and oracle call-site policy (`oracle/`).
