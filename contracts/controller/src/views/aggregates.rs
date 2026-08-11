//! Read-only USD-denominated aggregate views over a single account's supply
//! and borrow positions.

use crate::risk;
use crate::storage;
use soroban_sdk::Env;

use crate::context::Cache;

/// Returns the total USD value of the account's supply positions, in WAD scale.
/// Returns 0 if the account has no metadata or no supply positions.
pub(crate) fn total_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    if storage::try_get_account_meta(env, account_id).is_none() {
        return 0;
    }
    let supply = storage::get_supply_positions(env, account_id);
    if supply.is_empty() {
        return 0;
    }

    let mut cache = Cache::new_view(env);
    risk::sum_supply_usd(env, &mut cache, &supply).raw()
}

/// Returns the total USD value of the account's borrow positions, in WAD scale.
/// Returns 0 if the account has no metadata or no borrow positions.
pub(crate) fn total_borrow_in_usd(env: &Env, account_id: u64) -> i128 {
    if storage::try_get_account_meta(env, account_id).is_none() {
        return 0;
    }
    let borrow = storage::get_debt_positions(env, account_id);
    if borrow.is_empty() {
        return 0;
    }

    let mut cache = Cache::new_view(env);
    risk::sum_debt_usd(env, &mut cache, &borrow).raw()
}

/// Restamps loan-to-value ratios for the account's supply positions against
/// the current listed values, then returns the LTV-weighted USD value of
/// those positions, in WAD scale. Returns 0 if the account does not exist.
pub(crate) fn ltv_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    let Some(mut account) = storage::try_get_account(env, account_id) else {
        return 0;
    };
    let mut cache = Cache::new_view(env);
    let _ = risk::restamp_listed_supply_ltv(&mut cache, &mut account);
    risk::calculate_ltv_collateral_wad(env, &mut cache, &account.supply_positions).raw()
}
