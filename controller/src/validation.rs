use common::constants::{
    BPS, MAX_BORROW_RATE_RAY, MAX_FLASHLOAN_FEE_BPS, MAX_LIQUIDATION_BONUS, RAY, WAD,
};
use common::errors::{CollateralError, FlashLoanError, GenericError, OracleError};
use common::types::{
    Account, AssetConfig, MarketParams, MarketStatus, Payment, POSITION_TYPE_BORROW,
    POSITION_TYPE_DEPOSIT,
};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::{helpers, storage};

/// Panics with `AssetNotSupported` when `asset` has no market config.
pub fn require_asset_supported(env: &Env, asset: &Address) {
    if !storage::has_market_config(env, asset) {
        panic_with_error!(env, GenericError::AssetNotSupported);
    }
}

/// Panics with `PairNotActive` when the market status is not `Active`.
pub fn require_market_active(env: &Env, asset: &Address) {
    let market = storage::get_market_config(env, asset);
    if market.status != MarketStatus::Active {
        panic_with_error!(env, GenericError::PairNotActive);
    }
}

/// Panics with `AccountNotInMarket` when `caller` is not the account owner.
/// Does not call `require_auth`; use this when the caller was authenticated at
/// the endpoint boundary.
pub fn require_account_owner_match(env: &Env, account: &Account, caller: &Address) {
    if account.owner != *caller {
        panic_with_error!(env, GenericError::AccountNotInMarket);
    }
}

/// Panics with `FlashLoanOngoing` when a flash loan is already in progress.
pub fn require_not_flash_loaning(env: &Env) {
    if storage::is_flash_loan_ongoing(env) {
        panic_with_error!(env, FlashLoanError::FlashLoanOngoing);
    }
}

/// Panics with `AmountMustBePositive` when `amount ≤ 0`.
pub fn require_amount_positive(env: &Env, amount: i128) {
    if amount <= 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }
}

/// Panics with `InvalidPayments` when a payment-like batch is empty.
pub fn require_non_empty_payments<T>(env: &Env, payments: &Vec<T>) {
    if payments.is_empty() {
        panic_with_error!(env, GenericError::InvalidPayments);
    }
}

/// Panics with `InvalidPayments` when credited balance exceeds the sent amount.
pub fn require_credit_not_above_sent(env: &Env, sent: i128, received: i128) {
    if received > sent {
        panic_with_error!(env, GenericError::InvalidPayments);
    }
}

/// Panics with `InsufficientCollateral` when an account with debt has HF < 1.
pub fn require_healthy_account(
    env: &Env,
    cache: &mut crate::cache::ControllerCache,
    account: &Account,
) {
    if account.borrow_positions.is_empty() {
        return;
    }

    let hf = helpers::calculate_health_factor(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    if hf < WAD {
        panic_with_error!(env, CollateralError::InsufficientCollateral);
    }
}

/// Pre-flight bulk-isolation guard for supply batches.
///
/// An isolated account, or a batch whose first asset is isolated, must carry
/// exactly one collateral. Catching this up-front avoids running any
/// `token.transfer` or pool call before reverting on iteration 2 (Soroban
/// would still revert atomically, but the work is wasted).
///
/// Symmetric with [`validate_bulk_position_limits`] in placement and naming;
/// the cache is threaded in because the first asset's `AssetConfig` is
/// fetched here AND reused inside the per-asset loop, so reading once and
/// memoizing is cheaper than two storage reads.
pub fn validate_bulk_isolation(
    env: &Env,
    account: &Account,
    assets: &Vec<Payment>,
    cache: &mut crate::cache::ControllerCache,
) {
    if assets.len() <= 1 {
        return;
    }
    let (first_asset, _) = assets.get(0).unwrap();
    let first_config = cache.cached_asset_config(&first_asset);
    if account.is_isolated || first_config.is_isolated_asset {
        panic_with_error!(env, FlashLoanError::BulkSupplyNoIso);
    }
}

/// Panics with `PositionLimitExceeded` when the batch would push the account over its
/// supply or borrow position cap. Deduplicates assets before comparing against the limit.
pub fn validate_bulk_position_limits(
    env: &Env,
    account: &Account,
    position_type: u32,
    assets: &Vec<Payment>,
) {
    let limits = storage::get_position_limits(env);

    let (current_count, max_allowed) = if position_type == POSITION_TYPE_DEPOSIT {
        (account.supply_positions.len(), limits.max_supply_positions)
    } else if position_type == POSITION_TYPE_BORROW {
        (account.borrow_positions.len(), limits.max_borrow_positions)
    } else {
        // Panic rather than silently skipping the limit check if a future
        // caller passes an unrecognized position type.
        panic_with_error!(env, GenericError::InvalidPositionType);
    };

    // Repeated assets in one batch resolve to the same position; dedupe
    // before comparing against the position cap.
    let mut seen: Map<Address, bool> = Map::new(env);
    let mut new_positions_count: u32 = 0;
    for i in 0..assets.len() {
        let (asset, _) = assets.get(i).unwrap();
        if seen.contains_key(asset.clone()) {
            continue;
        }
        seen.set(asset.clone(), true);

        let already_present = if position_type == POSITION_TYPE_DEPOSIT {
            account.supply_positions.contains_key(asset)
        } else {
            account.borrow_positions.contains_key(asset)
        };
        if !already_present {
            new_positions_count = new_positions_count
                .checked_add(1)
                .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
        }
    }

    let total_positions = current_count
        .checked_add(new_positions_count)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    if total_positions > max_allowed {
        panic_with_error!(env, CollateralError::PositionLimitExceeded);
    }
}

/// Validates interest-rate model parameters: monotone slopes, utilization ordering,
/// reserve factor bounds, and the Taylor-envelope cap on `max_borrow_rate`.
pub fn validate_interest_rate_model(env: &Env, params: &MarketParams) {
    if params.base_borrow_rate_ray < 0
        || params.slope1_ray < params.base_borrow_rate_ray
        || params.slope2_ray < params.slope1_ray
        || params.slope3_ray < params.slope2_ray
        || params.max_borrow_rate_ray < params.slope3_ray
    {
        panic_with_error!(env, CollateralError::InvalidBorrowParams);
    }

    // Keep `max_borrow_rate_ray` inside the compound-interest Taylor envelope
    // (per-chunk `x <= 2 RAY`). At 100 % utilization across a full
    // `MAX_COMPOUND_DELTA_MS` chunk, a higher cap drifts above the documented
    // `< 0.01 %` accuracy bound.
    if params.max_borrow_rate_ray > MAX_BORROW_RATE_RAY {
        panic_with_error!(env, CollateralError::InvalidBorrowParams);
    }

    if params.mid_utilization_ray <= 0 {
        panic_with_error!(env, CollateralError::InvalidUtilRange);
    }
    if params.optimal_utilization_ray <= params.mid_utilization_ray {
        panic_with_error!(env, CollateralError::InvalidUtilRange);
    }
    if params.optimal_utilization_ray >= RAY {
        panic_with_error!(env, CollateralError::OptUtilTooHigh);
    }
    if i128::from(params.reserve_factor_bps) >= BPS {
        panic_with_error!(env, CollateralError::InvalidReserveFactor);
    }
}

/// Validates asset risk parameters: LTV ordering, liquidation bounds, cap sentinels,
/// isolation ceiling sign, and flash-loan fee bounds.
pub fn validate_asset_config(env: &Env, config: &AssetConfig) {
    // Liquidation threshold must sit strictly above LTV and at or below
    // 100% so new debt cannot open in liquidatable territory and HF math
    // stays bounded.
    if i128::from(config.liquidation_threshold_bps) <= i128::from(config.loan_to_value_bps)
        || i128::from(config.liquidation_threshold_bps) > BPS
    {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }

    if i128::from(config.liquidation_bonus_bps) > MAX_LIQUIDATION_BONUS {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }

    if i128::from(config.liquidation_fees_bps) > BPS {
        panic_with_error!(env, CollateralError::InvalidLiqThreshold);
    }

    // Cap sentinels: 0 = unlimited, >0 = enforced.
    if config.supply_cap < 0 || config.borrow_cap < 0 {
        panic_with_error!(env, CollateralError::InvalidBorrowParams);
    }

    // A negative isolation ceiling would make the `isolated_debt > ceiling`
    // check vacuously true and permit unlimited isolated borrowing.
    if config.isolation_debt_ceiling_usd_wad < 0 {
        panic_with_error!(env, CollateralError::InvalidBorrowParams);
    }

    // Shared validation for both `create_liquidity_pool` and
    // `edit_asset_config`. A fee above `MAX_FLASHLOAN_FEE_BPS` exceeds
    // the cap.
    if i128::from(config.flashloan_fee_bps) > MAX_FLASHLOAN_FEE_BPS {
        panic_with_error!(env, FlashLoanError::StrategyFeeExceeds);
    }
}

/// Panics with `BadAnchorTolerances` when `last ≤ first`.
pub fn validate_oracle_bounds(env: &Env, first: i128, last: i128) {
    if last <= first {
        panic_with_error!(env, OracleError::BadAnchorTolerances);
    }
    // Upper bound on `last` is enforced by the caller's range check via
    // `MAX_LAST_TOLERANCE`.
}
