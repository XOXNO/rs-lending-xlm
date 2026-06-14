use common::errors::{EModeError, GenericError};
use common::math::fp::Ray;
use common::math::fp_core::mul_div_floor;
use controller_interface::types::Account;
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env};

use crate::cache::Cache;
use crate::storage;

/// Clears isolated-debt accounting for a debt position being fully removed
/// (bad-debt seizure): decrements the global ceiling counter by the exact
/// principal basis this position contributed and drops the basis entry.
pub(crate) fn clear_position_isolated_debt(
    env: &Env,
    account: &Account,
    account_id: u64,
    asset: &Address,
    cache: &mut Cache,
) {
    if !account.is_isolated {
        return;
    }
    let Some(isolated_asset) = account.try_isolated_token() else {
        return;
    };
    let basis = storage::get_isolated_basis(env, account_id, asset);
    if basis <= 0 {
        return;
    }
    decrement_counter(cache, &isolated_asset, basis);
    storage::set_isolated_basis(env, account_id, asset, 0);
}

/// Decrements the isolated-debt ceiling counter and per-position basis when a
/// repay shrinks a debt position from `scaled_before` to `scaled_after`.
///
/// The decrement is the principal basis attributed to the repaid share —
/// proportional (floored, so the counter never under-counts) on a partial
/// repay and the full remaining basis on a full close — so the counter has no
/// interest- or price-drift asymmetry with the borrow-time increment.
pub(crate) fn adjust_isolated_debt_for_repay(
    env: &Env,
    account: &Account,
    account_id: u64,
    cache: &mut Cache,
    asset: &Address,
    scaled_before: Ray,
    scaled_after: Ray,
) {
    if !account.is_isolated {
        return;
    }
    let Some(isolated_asset) = account.try_isolated_token() else {
        return;
    };
    let basis = storage::get_isolated_basis(env, account_id, asset);
    if basis <= 0 || scaled_before <= Ray::ZERO || scaled_after >= scaled_before {
        return;
    }

    let decrement = if scaled_after == Ray::ZERO {
        basis
    } else {
        let repaid = scaled_before.raw() - scaled_after.raw();
        mul_div_floor(env, basis, repaid, scaled_before.raw())
    };

    decrement_counter(cache, &isolated_asset, decrement);
    storage::set_isolated_basis(env, account_id, asset, basis - decrement);
}

/// Adds USD WAD debt to the isolated collateral tracker, checks its ceiling,
/// and records the same principal basis on the debt position so repay and
/// liquidation remove exactly what was added.
///
/// Prices only isolated accounts: the oracle read happens after the guard so
/// non-isolated borrows never pay for it here.
pub(crate) fn add_isolated_debt(
    env: &Env,
    cache: &mut Cache,
    account: &Account,
    account_id: u64,
    asset: &Address,
    amount: i128,
) {
    if !account.is_isolated {
        return;
    }

    let feed = cache.cached_price(asset);
    let amount_in_usd_wad = feed.usd_value_wad(env, amount).raw();

    let isolated_token = crate::validation::expect_invariant(env, account.try_isolated_token());
    let collateral_config = cache.cached_asset_config(&isolated_token);

    // Read from the cache to stay consistent with pending in-batch deltas and
    // with the adjust_isolated_debt_for_repay decrement path above.
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

    // Record the principal basis on the debt position so repay/liquidation
    // decrement exactly this contribution, with no interest or price drift.
    let basis = storage::get_isolated_basis(env, account_id, asset)
        .checked_add(amount_in_usd_wad)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    storage::set_isolated_basis(env, account_id, asset, basis);
}

/// Decrements the cached global isolated-debt counter by `amount`, clamped at
/// zero. The counter aggregates across accounts, so it is not snapped to zero
/// on sub-unit residue — other accounts' balances may legitimately remain.
fn decrement_counter(cache: &mut Cache, isolated_asset: &Address, amount: i128) {
    if amount <= 0 {
        return;
    }
    let current = cache.get_isolated_debt(isolated_asset);
    let new_debt = (current - amount).max(0);
    cache.set_isolated_debt(isolated_asset, new_debt);
}
