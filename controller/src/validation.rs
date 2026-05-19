use common::constants::{BPS, MAX_FLASHLOAN_FEE_BPS, MAX_LIQUIDATION_BONUS, MIN_DUST_FLOOR_WAD, WAD};
use common::errors::{CollateralError, FlashLoanError, GenericError, OracleError};
use common::types::{
    Account, AssetConfig, MarketStatus, Payment, POSITION_TYPE_BORROW, POSITION_TYPE_DEPOSIT,
};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::cache::ControllerCache;
use crate::{helpers, storage};

// Invariant-driven `Option` unwrap. Use at every `Vec::get(i)` inside
// a `0..vec.len()` loop, every `Map::get(k)` for a key that was just
// inserted or iterated, and any other site where `None` is impossible
// under the surrounding invariant. Surfaces an explicit `InternalError`
// instead of an opaque host panic, so fuzzers can categorize misses
// rather than treating every host panic as the same failure mode.
// Use instead of raw `.unwrap()` in production paths.
#[inline]
pub fn expect_invariant<T>(env: &Env, opt: Option<T>) -> T {
    opt.unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError))
}

// Panics with `AssetNotSupported` when `asset` has no market config.
// The cache fetch is shared with every subsequent
// `cached_market_config` / `cached_asset_config` / `cached_pool_address`
// for the same asset in this transaction.
pub fn require_asset_supported(env: &Env, cache: &mut ControllerCache, asset: &Address) {
    let _ = env;
    // `cached_market_config` panics with `AssetNotSupported` on a
    // missing market — that panic IS the check.
    let _ = cache.cached_market_config(asset);
}

// Panics with `AssetNotSupported` when the market is missing or
// `PairNotActive` when its status is not `Active`. Shares the same
// cache fetch as `require_asset_supported` and any later read.
pub fn require_market_active(env: &Env, cache: &mut ControllerCache, asset: &Address) {
    let market = cache.cached_market_config(asset);
    if market.status != MarketStatus::Active {
        panic_with_error!(env, GenericError::PairNotActive);
    }
}

// Panics with `AccountNotInMarket` when `caller` is not the account owner.
// Does not call `require_auth`; use this when the caller was authenticated at
// the endpoint boundary.
pub fn require_account_owner_match(env: &Env, account: &Account, caller: &Address) {
    if account.owner != *caller {
        panic_with_error!(env, GenericError::AccountNotInMarket);
    }
}

// Panics with `FlashLoanOngoing` when a flash loan is in progress.
//
// Soroban does not enforce non-reentrancy automatically, so every
// public state-changing controller entry must call this before
// mutating storage. The list of call sites (supply, withdraw,
// borrow_batch, repay, liquidate, clean_bad_debt, flash_loan,
// multiply, swap_debt, swap_collateral, repay_debt_with_collateral,
// keeper entries) is exercised by the reentrancy matrix integration
// test; adding a new public entry that doesn't call this here will
// fail that test.
pub fn require_not_flash_loaning(env: &Env) {
    if storage::is_flash_loan_ongoing(env) {
        panic_with_error!(env, FlashLoanError::FlashLoanOngoing);
    }
}

// Panics with `AmountMustBePositive` when `amount ≤ 0`.
pub fn require_amount_positive(env: &Env, amount: i128) {
    if amount <= 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }
}

// Panics with `InvalidPayments` when a payment-like batch is empty.
pub fn require_non_empty_payments<T>(env: &Env, payments: &Vec<T>) {
    if payments.is_empty() {
        panic_with_error!(env, GenericError::InvalidPayments);
    }
}

// Panics with `InvalidPayments` when credited balance exceeds the sent amount.
pub fn require_credit_not_above_sent(env: &Env, sent: i128, received: i128) {
    if received > sent {
        panic_with_error!(env, GenericError::InvalidPayments);
    }
}

// Panics with `InsufficientCollateral` when an account with debt has HF < 1.
pub fn require_healthy_account(
    env: &Env,
    cache: &mut ControllerCache,
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

// Panics with `InsufficientCollateral` when total debt USD exceeds
// the LTV-weighted collateral USD. Mirrors the borrow-time invariant
// on every collateral-reducing path so withdraws and strategy exits
// cannot leave a position above the configured LTV ceiling — HF alone
// uses the looser liquidation threshold and would admit a state that
// is technically healthy but already inside the liquidation band.
pub fn require_within_ltv(
    env: &Env,
    cache: &mut ControllerCache,
    account: &Account,
) {
    if account.borrow_positions.is_empty() {
        return;
    }

    let ltv_collateral_wad =
        helpers::calculate_ltv_collateral_wad(env, cache, &account.supply_positions).raw();

    let mut total_borrow_wad: i128 = 0;
    for (asset, position) in account.borrow_positions.iter() {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);
        let value = helpers::position_value(
            env,
            common::fp::Ray::from_raw(position.scaled_amount_ray),
            common::fp::Ray::from_raw(market_index.borrow_index_ray),
            common::fp::Wad::from_raw(feed.price_wad),
        )
        .raw();
        total_borrow_wad = total_borrow_wad
            .checked_add(value)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    }

    if ltv_collateral_wad < total_borrow_wad {
        panic_with_error!(env, CollateralError::InsufficientCollateral);
    }
}

// Pre-flight bulk-isolation guard for supply batches.
//
// An isolated account, or a batch whose first asset is isolated, must carry
// exactly one collateral. Catching this up-front avoids running any
// `token.transfer` or pool call before reverting on iteration 2 (Soroban
// would still revert atomically, but the work is wasted).
//
// Symmetric with [`validate_bulk_position_limits`] in placement and naming;
// the cache is threaded in because the first asset's `AssetConfig` is
// fetched here AND reused inside the per-asset loop, so reading once and
// memoizing is cheaper than two storage reads.
pub fn validate_bulk_isolation(
    env: &Env,
    account: &Account,
    assets: &Vec<Payment>,
    cache: &mut ControllerCache,
) {
    if assets.len() <= 1 {
        return;
    }
    let (first_asset, _) = expect_invariant(env, assets.get(0));
    let first_config = cache.cached_asset_config(&first_asset);
    if account.is_isolated || first_config.is_isolated_asset {
        panic_with_error!(env, FlashLoanError::BulkSupplyNoIso);
    }
}

// Panics with `PositionLimitExceeded` when the batch would push the account over its
// supply or borrow position cap. Deduplicates assets before comparing against the limit.
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
        let (asset, _) = expect_invariant(env, assets.get(i));
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

// Validates asset risk parameters: LTV ordering, liquidation bounds, cap sentinels,
// isolation ceiling sign, and flash-loan fee bounds.
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

    // Dust floors are USD-WAD per-position minimums. Hard floor of $10
    // (`MIN_DUST_FLOOR_WAD`) — below this, dust positions become
    // un-liquidatable bad debt. Each market may opt for a higher floor.
    //
    // Sentinel: both fields == 0 disables the dust gate entirely (test
    // setups that need tiny amounts to exercise precision paths). The
    // sentinel must set BOTH fields — a half-disabled config (one side
    // 0, the other non-zero) is a bug.
    let dust_disabled = config.min_collat_floor_usd_wad == 0
        && config.min_debt_floor_usd_wad == 0;
    if !dust_disabled
        && (config.min_collat_floor_usd_wad < MIN_DUST_FLOOR_WAD
            || config.min_debt_floor_usd_wad < MIN_DUST_FLOOR_WAD)
    {
        panic_with_error!(env, CollateralError::DustFloorTooLow);
    }
}

// Panics with `BadAnchorTolerances` when `last ≤ first`.
pub fn validate_oracle_bounds(env: &Env, first: i128, last: i128) {
    if last <= first {
        panic_with_error!(env, OracleError::BadAnchorTolerances);
    }
    // Upper bound on `last` is enforced by the caller's range check via
    // `MAX_LAST_TOLERANCE`.
}
