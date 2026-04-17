# Configuration Invariants

Every operator-set field, its valid range, cross-field rules, and on-chain enforcement site. Gaps marked **MISSING** are self-defense holes the protocol should close.

## `MarketParams` (interest rate model)

Set at `create_liquidity_pool`; mutable via `upgrade_pool_params`. `validation::validate_interest_rate_model` (`controller/src/validation.rs:90`) validates.

| Field | Type | Valid range | Cross-field rule | Enforcement |
|---|---|---|---|---|
| `max_borrow_rate_ray` | i128 (RAY) | `>= slope3_ray` | top of monotone slope chain | `validate_interest_rate_model` |
| `base_borrow_rate_ray` | i128 (RAY) | `>= 0` | `<= slope1_ray` | `validate_interest_rate_model` |
| `slope1_ray` | i128 (RAY) | `>= base_borrow_rate_ray` | `<= slope2_ray` | `validate_interest_rate_model` |
| `slope2_ray` | i128 (RAY) | `>= slope1_ray` | `<= slope3_ray` | `validate_interest_rate_model` |
| `slope3_ray` | i128 (RAY) | `>= slope2_ray` | `<= max_borrow_rate_ray` | `validate_interest_rate_model` |
| `mid_utilization_ray` | i128 (RAY) | `> 0` | `< optimal_utilization_ray` | `validate_interest_rate_model` |
| `optimal_utilization_ray` | i128 (RAY) | `> mid_utilization_ray` | `< RAY` (i.e., < 100%) | `validate_interest_rate_model` |
| `reserve_factor_bps` | i128 (BPS) | `[0, 10_000)` | — | `validate_interest_rate_model` uses `< BPS` not `< BPS+1`, rejecting 100% RF — intentional, prevents zero supplier rewards |
| `asset_id` | Address | contract address | matches market key | implicit (router checks asset key on storage write) |
| `asset_decimals` | u32 | matches token | **read on-chain at `create_liquidity_pool`**: Makefile/script reads decimals, but no on-chain check confirms operator-passed `params.asset_decimals` matches `token.decimals()` | **GAP**: an operator who builds `MarketParams` off-chain with a wrong `asset_decimals` causes the pool to store the wrong value. Suggested: `assert_eq!(params.asset_decimals, token::Client::new(env, &asset).decimals())` in `__constructor` or router. |

## `AssetConfig` (per-market risk parameters)

Set at `create_liquidity_pool` and mutable via `edit_asset_config`. `validation::validate_asset_config` (`controller/src/validation.rs:114`) validates, plus `config::edit_asset_config` bounds the flashloan_fee.

| Field | Type | Valid range | Cross-field rule | Enforcement |
|---|---|---|---|---|
| `loan_to_value_bps` | i128 (BPS) | implicit `[0, liquidation_threshold_bps)` | LT > LTV (else liquidatable on first borrow) | `validate_asset_config` |
| `liquidation_threshold_bps` | i128 (BPS) | `> loan_to_value_bps` | implicit `<= 10_000` (no explicit upper bound — **MINOR GAP**: LT > 10_000 leaves HF math undefined; should panic with `InvalidLiqThreshold`) | partial (`validate_asset_config`) |
| `liquidation_bonus_bps` | i128 (BPS) | `<= MAX_LIQUIDATION_BONUS` (= 1_500 = 15%) | — | `validate_asset_config` |
| `liquidation_fees_bps` | i128 (BPS) | `<= 10_000` | — | `validate_asset_config` |
| `is_collateralizable` | bool | — | — | — |
| `is_borrowable` | bool | — | — | — |
| `e_mode_enabled` | bool | — | **preserved across edits** by `edit_asset_config`; operator cannot directly toggle — only `add_asset_to_e_mode_category` flips it | `config::edit_asset_config:75` |
| `is_isolated_asset` | bool | — | When true, accounts holding this asset cannot supply other collateral. Mutual exclusivity with `e_mode_enabled` runs at borrow/supply time, not config time. | runtime in `borrow` / `supply` |
| `is_siloed_borrowing` | bool | — | When true, the account can borrow only this asset. | runtime in `borrow` |
| `is_flashloanable` | bool | — | gates `flash_loan` per asset | `flash_loan::process_flash_loan:32` |
| `isolation_borrow_enabled` | bool | — | gates whether isolated-asset accounts can take new debt | runtime |
| `isolation_debt_ceiling_usd_wad` | i128 (WAD) | should be `>= 0` | **MISSING**: `validate_asset_config` skips the `>= 0` check. A negative ceiling makes `current_isolated_debt > ceiling` impossible to fail, effectively unlimited. **Suggested**: add `if config.isolation_debt_ceiling_usd_wad < 0 panic!`. |
| `flashloan_fee_bps` | i128 (BPS) | `<= MAX_FLASHLOAN_FEE_BPS` (= 500 = 5%) | **MISSING**: no `>= 0` check. A negative fee underflows the receiver-pays calculation, paying the receiver to flash-loan. **Suggested**: require `>= 0` explicitly. | upper-bound only at `config::edit_asset_config:69` |
| `borrow_cap` | i128 (asset units) | `>= 0` | 0 = unlimited | `validate_asset_config` |
| `supply_cap` | i128 (asset units) | `>= 0` | 0 = unlimited | `validate_asset_config` |

## `OracleProviderConfig` / `MarketOracleConfigInput`

Set at `configure_market_oracle`; tolerances mutable via `edit_oracle_tolerance`. `controller/src/oracle/mod.rs` validates piecewise.

| Field | Type | Valid range | Cross-field rule | Enforcement |
|---|---|---|---|---|
| `exchange_source` | enum | `SpotOnly` / `SpotVsTwap` / `DualOracle` | `DualOracle` requires `dex_oracle.is_some()` | runtime panic on missing dex |
| `max_price_stale_seconds` | u64 | `> 0` (else every price is stale) | **MISSING**: no `> 0` check; a 0 staleness window rejects every price. (May be an intentional kill-switch — document if so.) | — |
| `first_tolerance_bps` | i128 (BPS) | `[MIN_FIRST_TOLERANCE, MAX_FIRST_TOLERANCE]` = `[50, 5000]` | `< last_tolerance_bps` | `validate_oracle_bounds`, `validate_and_calculate_tolerances` |
| `last_tolerance_bps` | i128 (BPS) | `[MIN_LAST_TOLERANCE, MAX_LAST_TOLERANCE]` = `[150, 5000]` | `> first_tolerance_bps` | same |
| `cex_oracle` | Address | contract address | required (always non-`None`) | implicit |
| `cex_asset_kind` | enum | `Stellar` / `Other` | matches what the Reflector contract expects for `cex_symbol` | **runtime trust**: operator must match these correctly; mismatch returns wrong price |
| `cex_symbol` | Symbol | non-empty for `Other`; ignored for `Stellar` | **MISSING**: no on-chain check that the symbol resolves on the CEX oracle. Suggested: probe `oracle.lastprice(...)` during `configure_market_oracle` and reject `None`. |
| `cex_decimals` | u32 | matches CEX oracle | **read on-chain** during `configure_market_oracle` | enforced |
| `dex_oracle` | Option<Address> | required iff `DualOracle` | implicit | runtime |
| `dex_asset_kind` | enum | as above | as above | trust |
| `dex_decimals` | u32 | matches DEX oracle | **read on-chain** when DEX configured | enforced |
| `twap_records` | u32 | `> 0` for any non-`SpotOnly` source | **MISSING**: no explicit lower-bound check; 0 records makes TWAP return `None` and panic with `TwapInsufficientObservations` at first price call. Reject at config time. |

## `EModeCategory`

Set via `add_e_mode_category` / `edit_e_mode_category`. `config.rs:111-149` validates.

| Field | Type | Valid range | Cross-field rule | Enforcement |
|---|---|---|---|---|
| `category_id` | u32 | auto-incremented | unique | `storage::increment_emode_category_id` |
| `loan_to_value_bps` | i128 (BPS) | implicit | `< liquidation_threshold_bps` | `config.rs:112` |
| `liquidation_threshold_bps` | i128 (BPS) | implicit | `> loan_to_value_bps`; **MISSING upper bound** (`<= 10_000`) | partial |
| `liquidation_bonus_bps` | i128 (BPS) | `<= MAX_LIQUIDATION_BONUS` | — | `config.rs:115` |
| `is_deprecated` | bool | initialized false | only `remove_e_mode_category` flips it | — |

## Operator gotchas

- **`borrow_cap = 0` and `supply_cap = 0` mean UNLIMITED, not "disabled".** A common operator-error class. To throttle a market, use a small positive value. To disable borrowing entirely, set `is_borrowable = false`. Finding L-04.

## `PositionLimits`

Set via `set_position_limits`. `config::set_position_limits:97-103` validates.

| Field | Type | Valid range | Enforcement |
|---|---|---|---|
| `max_supply_positions` | u32 | `[1, 32]` | `config::set_position_limits` |
| `max_borrow_positions` | u32 | `[1, 32]` | same |

## Address allowlists

| Setting | Validation |
|---|---|
| `set_aggregator(addr)` | `addr` exists and has a Wasm executable (`require_contract_address`) |
| `set_accumulator(addr)` | same |
| `set_liquidity_pool_template(hash)` | hash != all-zeros |
| `approve_token_wasm(token)` | sets `TokenApproved(token) = true`; `create_liquidity_pool` checks (else `TokenNotApproved`) |

## Summary of Self-Defense Gaps

Status legend: ✅ already enforced  ·  🔧 fixed during audit prep  ·  📝 intentional/documented  ·  ⚠️ open

| # | Field / Rule | Status | Notes |
|---|---|---|---|
| 1 | `MarketParams.asset_decimals` cross-checked against `token.decimals()` | ✅ | Enforced in `router::validate_market_creation:22-25` (`#[cfg(not(feature = "testing"))]`). The test-feature bypass is intentional for the SAC test harness. |
| 2 | `AssetConfig.liquidation_threshold_bps <= 10_000` upper bound | 🔧 | **Fixed in audit prep** (`validation.rs::validate_asset_config`). Also rejects negative LTV, negative `liquidation_bonus_bps`, and negative `liquidation_fees_bps`. |
| 3 | `AssetConfig.isolation_debt_ceiling_usd_wad >= 0` | 🔧 | **Fixed in audit prep** (`validation.rs::validate_asset_config`). |
| 4 | `AssetConfig.flashloan_fee_bps >= 0` | 🔧 | **Fixed in audit prep** (`config.rs::edit_asset_config`); panics with `FlashLoanError::NegativeFlashLoanFee`. |
| 5 | `MarketOracleConfigInput.max_price_stale_seconds` bounded | ✅ | Enforced in `config.rs::configure_market_oracle:371`: `[60, 86_400]` seconds, else `OracleError::InvalidStalenessConfig`. |
| 6 | `MarketOracleConfigInput.cex_symbol` resolution probed at config time | ✅ | Enforced in `config.rs::resolve_oracle_decimals:345`: `cex_client.lastprice(...).is_none()` panics with `GenericError::InvalidTicker`. |
| 7 | `MarketOracleConfigInput.twap_records` bounds | 📝 | Upper bound `<= 12` enforced (`resolve_oracle_decimals:326`). Lower bound 0 is an intentional fallback: the pricing path returns spot when `twap_records == 0` (see `oracle/mod.rs:225-226, 263-264`). Document as intentional. |
| 8 | `EModeCategory.liquidation_threshold_bps <= 10_000` upper bound | 🔧 | **Fixed in audit prep** (`config.rs::add_e_mode_category` and `edit_e_mode_category`). Also rejects negative LTV / threshold / bonus. |
| 9 | `is_isolated_asset` + `e_mode_enabled` mutual exclusivity at config time | ⚠️ | Open. Only the runtime supply/borrow paths enforce it. Pre-audit decision: leave as a runtime check (accommodates `add_asset_to_e_mode_category` toggling e-mode rather than `edit_asset_config`) or add a hard config-time reject. **Recommend leaving as-is**: `edit_asset_config` preserves `e_mode_enabled`, and only `add_asset_to_e_mode_category` flips it. |

Commit (post-prep) enforces all ✅ and 🔧 items. Auditors should still confirm semantic correctness of each check.
