# Integration tests

Contract-level scenarios for the lending protocol, executed in-process via the `test-harness` library. Each domain directory (`tests/<domain>/main.rs`) and each top-level `tests/*.rs` file is one Cargo test binary.

**Prerequisite:** `make build` (the pool and position-nft WASM must exist).

## Binaries

| `--test` | Directory | Coverage |
|----------|-----------|----------|
| `smoke_test` | `smoke_test.rs` | Supply, borrow, withdraw, repay, liquidate, interest, spoke, revenue, multiply |
| `controller` | `controller/` | Positions, supply/borrow/repay/withdraw, liquidation, admin, spoke, flash loan, keeper, views |
| `governance` | `governance/` | Timelock operation states, immediate-path roles, permissionless execution, and admin-input validation on the governance forwarders: market creation, asset config, IRM, position limits, oracle config/tolerance probing |
| `oracle` | `oracle/` | Tolerance bands, staleness, future timestamps, dual-source, TWAP, Redstone, XOXNO adapter, DEX USD repricing |
| `pool` | `pool/` | Interest curves, revenue, pool math |
| `strategy` | `strategy/` | Multiply, flash position, swap collateral/debt, Blend migration, router guards, happy paths, edge cases |
| `composition` | `composition/` | One contract, as the top-level caller, chains controller verbs in a single invocation |
| `fuzz` | `fuzz/` | Proptest properties — see [`fuzz/README.md`](fuzz/README.md) |
| `meta` | `meta/` | Footprint, budget breakdown, chaos/stress sims, invariants, reentrancy, TTL |
| `astra_audit` | `astra_audit.rs` | Exact scaled-share reconciliation across every position NFT, including credit-mode receivers |
| `pool_money_flow_audit` | `pool_money_flow_audit.rs` | Pool-layer money paths under a mocked owner: books, shared token custody, recapitalization refunds |
| `poc_multiply_reentrancy` | `poc_multiply_reentrancy.rs` | An initial-payment token that re-enters the controller during `multiply` aborts the call; no collateral or USDC moves |
| `strategy_origination_fee_parity` | `strategy_origination_fee_parity.rs` | `flash_position` opens the same position as `multiply` without the strategy origination fee |
| `strategy_solvency_gate_rollback` | `strategy_solvency_gate_rollback.rs` | A solvency-gate rejection in `flash_position` reverts every earlier token transfer |
| `zz_storage_sizing` | `zz_storage_sizing.rs` | Prints the XDR size of the per-account storage entries (`print_storage_sizes`) |

`tests/test-harness/Cargo.toml` declares no `[[test]]` sections, so cargo
discovers every `tests/*.rs` and `tests/<domain>/main.rs`. All fifteen binaries
run under `cargo test -p test-harness`.

## Module inventory

### `controller/`

`account`, `admin`, `admin_config`, `audit_borrow_withdraw_liquidate_stale_anchor_blend`, `audit_liquidate_and_clean_stale_leg`, `audit_liquidate_dust_fee_dos`, `audit_supply_stale_shield`, `bad_debt_index`, `bad_debt_netting_and_exit_timing`, `borrow`, `bulk_indexes`, `decimal_diversity`, `deprecated_spoke_liquidation_liveness`, `dust_threshold_and_decimal_floor`, `events`, `extreme_amount_inputs`, `flash_loan`, `flash_loan_adversarial`, `force_socialize_refuses_solvent_accounts`, `governance_change_between_legs`, `keeper`, `large_positions_and_long_horizons`, `liquidation`, `liquidation_accrual_timing`, `liquidation_and_borrow_exact_boundaries`, `liquidation_band_full_close`, `liquidation_band_signed_auth`, `liquidation_boundary`, `liquidation_coverage`, `liquidation_extreme`, `liquidation_math`, `liquidation_mixed_decimal`, `liquidation_ratchet`, `liquidation_seize_modes`, `liquidation_under_delivering_debt_token`, `max_utilization`, `min_borrow_collateral`, `multi_hub`, `outbound_transfer_measurement`, `ownership`, `position_limit_lowering_keeps_topups`, `position_nft`, `position_nft_ttl_and_ownership_reads`, `recipient_is_protocol_contract`, `repay`, `round_trip_exactness_and_loop_drift`, `same_market_bad_debt_cleanup_arithmetic`, `security_audit`, `security_audit_extended`, `shared`, `spoke`, `spoke_caps`, `spoke_liquidation_combo`, `spoke_usage_tracks_positions`, `supply`, `third_party_supply_and_risk_restamp`, `validation_admin`, `views`, `withdraw`

#### Liquidation modules (roles)

| Module | Role |
|--------|------|
| `liquidation.rs` | Happy-path smoke: proportional/targeted seize, bonus tiers, bad-debt socialization, rejections (healthy account, active flash loan, zero amount), allowed cases (paused controller, self-liquidation) |
| `liquidation_coverage.rs` | Input validation and edge shapes: duplicate payments, empty/zero/unsupported assets, subunit collateral, multi-debt caps |
| `liquidation_math.rs` | Quantitative invariants: bonus formula, protocol fee on bonus only, bad-debt index delta, bounded seizure |
| `liquidation_boundary.rs` | Threshold behavior: HF exactly 1 vs just below, monotone bonus band, bad-debt trigger at collateral floor |
| `liquidation_mixed_decimal.rs` | Decimal heterogeneity across collateral/debt pairs |
| `liquidation_and_borrow_exact_boundaries.rs` | HF exactly 1, borrow exactly at the LTV limit, LTV collateral exactly at the floor |
| `liquidation_accrual_timing.rs` | Index accrual during the call with fixed prices gives the liquidator nothing |
| `liquidation_band_full_close.rs` | Full closes in the band `D <= C < D * (1 + base)`: merged offers, fee-on-transfer debt, over-offers |
| `liquidation_band_signed_auth.rs` | Band liquidations under enforced auth, with interest accrued between simulation and execution |
| `liquidation_extreme.rs` | Extreme liquidation curves: high target HF, flat bonus, narrow curve, high-LTV stablecoin |
| `liquidation_ratchet.rs` | A chain of partial liquidations never extracts more than a single liquidation |
| `liquidation_seize_modes.rs` | `no_seize` pause split and share-credit liquidation (`SeizeMode::Credit`) |
| `liquidation_under_delivering_debt_token.rs` | A fee-on-transfer debt token scales both the debt retired and the collateral seized |
| `spoke_liquidation_combo.rs` | Spoke category liquidation with category-specific LTV/threshold |

### `governance/`

`admin`, `admin_config`, `dex_usd_repricing`, `immediate`, `permissionless_execution`, `redstone`, `spoke`, `stale_edit_and_sensitive_floor`, `timelock`, `tolerance`, `validation_admin`

### `oracle/`

- `tolerance/` — `bands`, `config`, `dual_source`, `edge`, `staleness`
- `dex_usd_repricing`, `future_skew_live_path`, `redstone`, `redstone_bulk`, `twap`, `xoxno`

### `pool/`

`accrual_partition_bound`, `interest`, `interest_rigorous`, `math_rates`, `pool_coverage`, `pool_revenue_edge`, `revenue`

### `strategy/`

`adversarial`, `core`, `cross_hub_same_asset_loop`, `extreme_amount_inputs`, `flash_position`, `flash_position_adversarial`, `flash_position_callback_ownership_transfer`, `flash_position_mode_and_asset_edges`, `happy`, `helpers`, `migrate_blend`, `migrate_blend_account_ownership`, `rogue_hop_pool_transfer_joins_caller_auth_tree`, `router`, `strategy_solvency_gate_on_low_value_router_output`, `edge/` (`multiply`, `pause_bypass`, `rejections`, `swap`)

### `composition/`

`atomic_revert_all_legs`, `contract_caller_runs_every_verb`, `delegate_revocation_between_legs`, `helpers`, `nft_transfer_between_legs`, `repeated_loops_never_extract_value`, `supplier_exit_before_socialization_is_bounded_by_utilization`

### `meta/`

`account_ttl_regression`, `admin_instance_ttl_regression`, `bench_liquidate_max_positions`, `budget_breakdown`, `chaos_simulation`, `economic_attacks`, `footprint_test`, `invariant`, `lifecycle_regression`, `mem_attribution`, `reentrancy_matrix`, `repro_live_supply`, `stress_simulation`, `utils`

### `fuzz/`

`accounting_conservation`, `config`, `liquidation_vs_reference`, `migrate_blend`, `ops`, `privileged_auth_rejects`, `strategy_helpers`, `strategy_multiply_budget`, `strategy_router_invariants`

These inventories are generated from the `mod` declarations in each
`tests/<domain>/main.rs`. Regenerate them after adding or removing a module:

```bash
for m in tests/test-harness/tests/*/main.rs; do
  echo "--- $m"
  grep -oE '^ *(pub )?mod \w+;' "$m" | sed 's/.*mod //;s/;//' | paste -sd' ' -
done
```

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
cargo test -p test-harness

# One binary
cargo test -p test-harness --test controller
cargo test -p test-harness --test oracle
cargo test -p test-harness --test pool
cargo test -p test-harness --test strategy
cargo test -p test-harness --test fuzz
cargo test -p test-harness --test meta

# Fast gate
cargo test -p test-harness --test smoke_test

# Filter by test name (works across binaries when unscoped)
cargo test -p test-harness smoke
cargo test -p test-harness --test controller test_supply_rejects_zero
cargo test -p test-harness --test fuzz prop_accounting_conservation

# Serialise while bisecting a suspected cross-test interaction
cargo test -p test-harness -- --test-threads=1

# Makefile
make test-one FILE=controller
make test-match PATTERN=liquidation
```

### Proptest

Properties live in `fuzz/`. `make proptest` uses tuned per-property defaults;
`PROPTEST_CASES` overrides the defaults of every randomized property (see
`fuzz/config.rs`). The plain deterministic `#[test]` functions, such as the two
auth matrices, ignore the variable. Use release builds for long runs.

```bash
make proptest
make proptest PROPTEST_CASES=256
make proptest-one TEST=prop_accounting_conservation PROPTEST_CASES=1000
PROPTEST_CASES=10000 cargo test --release -p test-harness --test fuzz -- --test-threads=1
```

Minimized failure seeds are committed as `fuzz/*.proptest-regressions`.

## Fixtures

Shared builders and seeds live in `src/fixtures.rs`. The crate root re-exports
them (`src/prelude.rs`), so tests import them directly:

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

Example:

```rust
let mut t = LendingTest::new().standard_two_asset().build();
t.supply(ALICE, "USDC", 10_000.0);

let mut t = LendingTest::new().dual_source_two_asset();
t.supply(ALICE, "USDC", 10_000.0);

seed_liquidatable_usdc_eth(&mut t);
```

## Library reference

Crate root: [`../README.md`](../README.md). Public API surface: imports from the crate root, `test_harness::{LendingTest, …}`; `src/prelude.rs` lists the re-exports.

The harness also pins live contract facts: the ownership chain
(`controller/ownership.rs`), the pause matrix
(`controller/security_audit_extended.rs`), multi-hub and spoke wiring
(`controller/multi_hub.rs`, `controller/spoke.rs`), the bad-debt floor
(`controller/bad_debt_index.rs`), and oracle call-site policy (`oracle/`).
