use common::constants::WAD;
use common::errors::GenericError;
use common::fp::Wad;
use common::types::{Account, Payment, PositionMode};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::cache::ControllerCache;
use crate::cross_contract::pool::pool_update_indexes_call;
use crate::cross_contract::sac::sac_transfer_call;
use crate::{storage, validation};

pub use crate::positions::account::{create_account, remove_account};

// Deduplicates and sums payments.
pub fn aggregate_positive_payments(env: &Env, payments: &Vec<Payment>) -> Vec<Payment> {
    aggregate_payments(env, payments, false)
}

// Deduplicates withdrawal requests.
pub fn aggregate_withdrawal_payments(env: &Env, payments: &Vec<Payment>) -> Vec<Payment> {
    aggregate_payments(env, payments, true)
}

// Transfers asset and returns credited amount.
pub fn transfer_and_measure_received(
    env: &Env,
    asset: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
    balance_decrease_error: GenericError,
) -> i128 {
    if amount <= 0 {
        panic_with_error!(env, balance_decrease_error);
    }
    sac_transfer_call(env, asset, from, to, &amount);
    amount
}

fn aggregate_payments(
    env: &Env,
    payments: &Vec<Payment>,
    zero_is_withdraw_all: bool,
) -> Vec<Payment> {
    let mut order: Vec<Address> = Vec::new(env);
    let mut totals: Map<Address, i128> = Map::new(env);

    for (asset, amount) in payments {
        let next =
            aggregate_payment_amount(env, totals.get(asset.clone()), amount, zero_is_withdraw_all);

        if !totals.contains_key(asset.clone()) {
            order.push_back(asset.clone());
        }
        totals.set(asset, next);
    }

    let mut result = Vec::new(env);
    for asset in order {
        let amount = validation::expect_invariant(env, totals.get(asset.clone()));
        result.push_back((asset, amount));
    }

    result
}

fn aggregate_payment_amount(
    env: &Env,
    previous: Option<i128>,
    amount: i128,
    zero_is_withdraw_all: bool,
) -> i128 {
    if amount < 0 || (!zero_is_withdraw_all && amount == 0) {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }

    if zero_is_withdraw_all && (amount == 0 || previous == Some(0)) {
        return 0;
    }

    previous
        .unwrap_or(0)
        .checked_add(amount)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

// Creates account for supply entry point.
pub fn create_account_for_first_asset(
    env: &Env,
    caller: &Address,
    e_mode_category: u32,
    assets: &Vec<Payment>,
) -> (u64, Account) {
    let (first_asset, _) = validation::expect_invariant(env, assets.get(0));
    let first_config = storage::get_market_config(env, &first_asset).asset_config;
    let is_isolated = first_config.is_isolated_asset;
    let isolated_asset = if is_isolated {
        Some(first_asset.clone())
    } else {
        None
    };
    create_account(
        env,
        caller,
        e_mode_category,
        PositionMode::Normal,
        is_isolated,
        isolated_asset,
    )
}

// Syncs market indexes and cache.
pub fn sync_market_indexes(env: &Env, cache: &mut ControllerCache, assets: &Vec<Address>) {
    for asset in assets {
        let pool_addr = cache.cached_pool_address(&asset);
        let state = pool_update_indexes_call(env, &pool_addr);
        // Refresh cache for subsequent reads.
        cache.record_market_update(&state);
    }
}

// Decrements isolated debt tracker.
pub fn adjust_isolated_debt_usd(
    env: &Env,
    account: &Account,
    token_amount: i128,
    price_wad: &i128,
    asset_decimals: u32,
    cache: &mut ControllerCache,
) {
    let Some(isolated_asset) = account.isolated_asset.clone() else {
        return;
    };

    let amount_wad = Wad::from_token(token_amount, asset_decimals);
    let usd_wad = amount_wad.mul(env, Wad::from_raw(*price_wad)).raw();

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
