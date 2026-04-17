# Configuration Invariants

Quick-reference for reviewing a proposed market config or a `set_position_limits` / `edit_asset_config` call. Each row: valid range, enforcement site, one-line rationale, one-line failure mode if violated.

## Risk parameters (`AssetConfig`)

Set at `create_liquidity_pool`, mutable via `edit_asset_config`. Validated by `validation::validate_asset_config` (`controller/src/validation.rs:114`).

| Parameter | Valid range | Enforced in | Rationale | Failure mode |
|---|---|---|---|---|
| `loan_to_value_bps` | `[0, liquidation_threshold_bps)` | `validation.rs::validate_asset_config` | LT must exceed LTV so a fresh borrow is not instantly liquidatable | Borrower liquidated immediately on first borrow |
| `liquidation_threshold_bps` | `> loan_to_value_bps`, `<= 10_000` | `validation.rs::validate_asset_config` | Upper bound keeps HF math well-defined | HF undefined, liquidation pathway breaks |
| `liquidation_bonus_bps` | `<= MAX_LIQUIDATION_BONUS` (1_500 = 15%) | `validation.rs::validate_asset_config` | Caps liquidator premium | Liquidator extracts excessive bonus from borrower |
| `liquidation_fees_bps` | `<= 10_000` | `validation.rs::validate_asset_config` | Fee cannot exceed 100% of seized collateral | Fee math overflows seized amount |
| `is_collateralizable` | bool | — | Gates collateral usage | — |
| `is_borrowable` | bool | — | Gates borrow path per asset | — |

## Isolation and siloed borrowing

| Parameter | Valid range | Enforced in | Rationale | Failure mode |
|---|---|---|---|---|
| `is_isolated_asset` | bool | runtime in `borrow` / `supply` | Isolated collateral cannot mix with other collateral | Mixed collateral bypasses debt ceiling |
| `is_siloed_borrowing` | bool | runtime in `borrow` | Account may borrow only this asset while the flag holds | Account accrues multi-asset debt against a siloed asset |
| `isolation_borrow_enabled` | bool | runtime | Gates whether isolated-asset accounts can take new debt | Isolated accounts borrow when disallowed |
| `isolation_debt_ceiling_usd_wad` | `>= 0` | `validation.rs::validate_asset_config` | Caps aggregate isolated debt in USD | Negative ceiling lets isolated debt grow unbounded |

## Flash loans

| Parameter | Valid range | Enforced in | Rationale | Failure mode |
|---|---|---|---|---|
| `is_flashloanable` | bool | `flash_loan::process_flash_loan:32` | Per-asset flash-loan switch | Flash loans disabled at config reject unrelated calls |
| `flashloan_fee_bps` | `[0, MAX_FLASHLOAN_FEE_BPS]` (500 = 5%) | `config.rs::edit_asset_config:69` (panics `FlashLoanError::NegativeFlashLoanFee` on negative) | Caps fee and blocks negative-fee underflow | Negative fee pays the receiver; over-cap extracts too much |

## Caps

| Parameter | Valid range | Enforced in | Rationale | Failure mode |
|---|---|---|---|---|
| `borrow_cap` | `>= 0` (0 = unlimited) | `validation.rs::validate_asset_config` | Throttles aggregate borrow | Unchecked borrow growth |
| `supply_cap` | `>= 0` (0 = unlimited) | `validation.rs::validate_asset_config` | Throttles aggregate supply | Unchecked supply growth |

Note: cap = 0 means UNLIMITED, not disabled. To disable borrowing entirely set `is_borrowable = false`.

## Interest-rate model (`MarketParams`)

Set at `create_liquidity_pool`; mutable via `upgrade_pool_params`. Validated by `validation::validate_interest_rate_model` (`controller/src/validation.rs:90`).

| Parameter | Valid range | Enforced in | Rationale | Failure mode |
|---|---|---|---|---|
| `base_borrow_rate_ray` | `[0, slope1_ray]` | `validate_interest_rate_model` | Floor of the rate curve | Curve non-monotone, rates decrease under load |
| `slope1_ray` | `[base_borrow_rate_ray, slope2_ray]` | `validate_interest_rate_model` | Monotone chain segment | Curve non-monotone |
| `slope2_ray` | `[slope1_ray, slope3_ray]` | `validate_interest_rate_model` | Monotone chain segment | Curve non-monotone |
| `slope3_ray` | `[slope2_ray, max_borrow_rate_ray]` | `validate_interest_rate_model` | Monotone chain segment | Curve non-monotone |
| `max_borrow_rate_ray` | `>= slope3_ray` | `validate_interest_rate_model` | Hard ceiling on borrow rate | Unbounded rate accrual |
| `mid_utilization_ray` | `(0, optimal_utilization_ray)` | `validate_interest_rate_model` | First kink below optimal utilization | Kinks collapse, rate jumps become discontinuous |
| `optimal_utilization_ray` | `(mid_utilization_ray, RAY)` | `validate_interest_rate_model` | Second kink strictly below 100% | Optimal at 100% removes the high-utilization slope |
| `reserve_factor_bps` | `[0, 10_000)` | `validate_interest_rate_model` (strict `< BPS`) | Rejects 100% RF so suppliers keep a share | Suppliers earn zero rewards |
| `asset_id` | market contract address | router checks asset key on storage write | Matches the market key | Config written under wrong key |
| `asset_decimals` | `== token.decimals()` | `router::validate_market_creation:22-25` (`#[cfg(not(feature = "testing"))]`) | Decimal mismatch corrupts pricing | Wrong decimals silently misprice the market |

## Oracle (`OracleProviderConfig` / `MarketOracleConfigInput`)

Set at `configure_market_oracle`; tolerances mutable via `edit_oracle_tolerance`. Validated in `controller/src/oracle/mod.rs` and `config.rs`.

| Parameter | Valid range | Enforced in | Rationale | Failure mode |
|---|---|---|---|---|
| `exchange_source` | `SpotOnly` / `SpotVsTwap` / `DualOracle` | runtime panic on missing dex | `DualOracle` requires `dex_oracle.is_some()` | Panics when a DEX call is made without a configured DEX |
| `max_price_stale_seconds` | `[60, 86_400]` | `config.rs::configure_market_oracle:371` (`OracleError::InvalidStalenessConfig`) | Bounded freshness window | Too-small rejects all prices; too-large accepts stale |
| `first_tolerance_bps` | `[MIN_FIRST_TOLERANCE, MAX_FIRST_TOLERANCE]` = `[50, 5000]`, `< last_tolerance_bps` | `validate_oracle_bounds`, `validate_and_calculate_tolerances` | First deviation band | Wrong band triggers false fallbacks |
| `last_tolerance_bps` | `[MIN_LAST_TOLERANCE, MAX_LAST_TOLERANCE]` = `[150, 5000]`, `> first_tolerance_bps` | same | Outer deviation band | Wrong band masks large deviations |
| `cex_oracle` | contract address, required | implicit | Primary price source | Missing CEX oracle breaks pricing |
| `cex_asset_kind` | `Stellar` / `Other` | runtime trust | Must match symbol the Reflector contract expects | Wrong kind returns wrong price |
| `cex_symbol` | resolvable via `cex_client.lastprice(...)` | `config.rs::resolve_oracle_decimals:345` (`GenericError::InvalidTicker`) | Rejects unresolvable symbols at config time | Symbol returns `None` later and breaks pricing |
| `cex_decimals` | `== cex oracle decimals` | read on-chain during `configure_market_oracle` | Matches oracle | Scaling error on price |
| `dex_oracle` | required iff `DualOracle` | runtime | Secondary source for dual-oracle mode | Dual-oracle mode panics without DEX |
| `dex_asset_kind` | `Stellar` / `Other` | runtime trust | Matches DEX oracle | Wrong kind returns wrong price |
| `dex_decimals` | `== dex oracle decimals` | read on-chain when DEX configured | Matches oracle | Scaling error on price |
| `twap_records` | `<= 12` (0 = spot fallback) | `resolve_oracle_decimals:326`; fallback at `oracle/mod.rs:225-226, 263-264` | Upper bound caps TWAP window; 0 is an intentional spot fallback | Over-cap reverts; misuse of `0` when caller expects TWAP |

## E-mode (`EModeCategory`)

Set via `add_e_mode_category` / `edit_e_mode_category`. Validated in `config.rs:111-149`.

| Parameter | Valid range | Enforced in | Rationale | Failure mode |
|---|---|---|---|---|
| `category_id` | auto-incremented, unique | `storage::increment_emode_category_id` | Stable identifier | Collision would overwrite category config |
| `loan_to_value_bps` | `< liquidation_threshold_bps`, non-negative | `config.rs:112` | LT must exceed LTV | Immediate liquidation on borrow |
| `liquidation_threshold_bps` | `> loan_to_value_bps`, `<= 10_000`, non-negative | `config.rs` (`add_e_mode_category` / `edit_e_mode_category`) | Upper bound keeps HF math well-defined | HF undefined |
| `liquidation_bonus_bps` | `<= MAX_LIQUIDATION_BONUS`, non-negative | `config.rs:115` | Caps liquidator premium | Excessive bonus extraction |
| `is_deprecated` | initialized false; flipped only by `remove_e_mode_category` | — | Soft-delete toggle | Accidental deprecation blocks new positions |
| `e_mode_enabled` on `AssetConfig` | preserved across `edit_asset_config`; only `add_asset_to_e_mode_category` flips it | `config::edit_asset_config:75` | Prevents operator from silently breaking e-mode assumptions | Toggling e-mode without running category registration |

Mutual exclusivity of `is_isolated_asset` and `e_mode_enabled` is enforced at runtime in supply/borrow paths (not config time).

## Position limits (`PositionLimits`)

Set via `set_position_limits`. Validated in `config::set_position_limits:97-103`.

| Parameter | Valid range | Enforced in | Rationale | Failure mode |
|---|---|---|---|---|
| `max_supply_positions` | `[1, 32]` | `config::set_position_limits` | Bounds per-account supply slots | 0 locks out supply; above 32 bloats storage and gas |
| `max_borrow_positions` | `[1, 32]` | `config::set_position_limits` | Bounds per-account borrow slots | 0 locks out borrow; above 32 bloats storage and gas |

## Address allowlists

| Setting | Valid range | Enforced in | Rationale | Failure mode |
|---|---|---|---|---|
| `set_aggregator(addr)` | existing contract with Wasm executable | `require_contract_address` | Reject EOAs and empty addresses | Calls to non-contract revert later |
| `set_accumulator(addr)` | existing contract with Wasm executable | `require_contract_address` | Same | Same |
| `set_liquidity_pool_template(hash)` | non-zero hash | hash check | Template must exist | Pool deploy fails on zero template |
| `approve_token_wasm(token)` | sets `TokenApproved(token) = true` | `create_liquidity_pool` (`TokenNotApproved`) | Gate on approved tokens only | Unapproved token pool creation |
