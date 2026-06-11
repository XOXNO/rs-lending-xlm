use common::constants::WAD;
use common::errors::{EModeError, GenericError};
use common::math::fp::Ray;
use common::types::{Account, DebtPosition, PriceFeed};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::cache::Cache;

/// Clears isolated-debt accounting for a debt position that is being removed.
pub(crate) fn clear_position_isolated_debt(
    env: &Env,
    asset: &Address,
    position: &DebtPosition,
    account: &Account,
    cache: &mut Cache,
) {
    if !account.is_isolated {
        return;
    }

    let market_index = cache.cached_market_index(asset);
    let feed = cache.cached_price(asset);
    let actual_amount = actual_borrow_amount(
        env,
        position,
        market_index.borrow_index,
        feed.asset_decimals,
    );
    adjust_isolated_debt_for_repay(env, account, cache, actual_amount, &feed);
}

pub(crate) fn actual_borrow_amount(
    env: &Env,
    position: &DebtPosition,
    borrow_index: Ray,
    asset_decimals: u32,
) -> i128 {
    position
        .scaled_amount
        .mul(env, borrow_index)
        .to_asset(asset_decimals)
}

pub(crate) fn adjust_isolated_debt_for_repay(
    env: &Env,
    account: &Account,
    cache: &mut Cache,
    actual_amount: i128,
    feed: &PriceFeed,
) {
    if actual_amount > 0 {
        adjust_isolated_debt_usd(env, account, actual_amount, feed, cache);
    }
}

pub(crate) fn adjust_isolated_debt_usd(
    env: &Env,
    account: &Account,
    token_amount: i128,
    feed: &PriceFeed,
    cache: &mut Cache,
) {
    let Some(isolated_asset) = account.try_isolated_token() else {
        return;
    };

    let usd_wad = feed.usd_value_wad(env, token_amount).raw();

    let current = cache.get_isolated_debt(&isolated_asset);
    let mut new_debt = if usd_wad >= current {
        0
    } else {
        current - usd_wad
    };

    if new_debt > 0 && new_debt < WAD {
        new_debt = 0;
    }

    cache.set_isolated_debt(&isolated_asset, new_debt);
}

/// Adds USD WAD debt to the isolated collateral tracker and checks its ceiling.
///
/// Prices only isolated accounts: the oracle read happens after the guard so
/// non-isolated borrows never pay for it here.
pub(crate) fn add_isolated_debt(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    asset: &Address,
    amount: i128,
) {
    if !account.is_isolated {
        return;
    }

    let feed = cache.cached_price(asset);
    let amount_in_usd_wad = feed.usd_value_wad(env, amount).raw();

    let isolated_token = account
        .try_isolated_token()
        .unwrap_or_else(|| panic_with_error!(env, GenericError::InternalError));
    let collateral_config = cache.cached_asset_config(&isolated_token);

    // Read from the cache to stay consistent with pending in-batch deltas and
    // with the adjust_isolated_debt_usd decrement path above.
    let current_debt = cache.get_isolated_debt(&isolated_token);
    let new_debt = current_debt
        .checked_add(amount_in_usd_wad)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));

    assert_with_error!(
        env,
        new_debt <= collateral_config.isolation_debt_ceiling_usd.raw(),
        EModeError::DebtCeilingReached
    );

    // Write back through the cache; flush_isolated_debts defers the storage
    // write and emits `UpdateDebtCeilingBatchEvent` at end-of-batch.
    // Emitting here instead would fire one event per in-batch borrow.
    cache.set_isolated_debt(&isolated_token, new_debt);
}
