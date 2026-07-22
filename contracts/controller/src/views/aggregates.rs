//! USD-aggregate views.

use crate::risk;
use crate::storage;
use soroban_sdk::Env;

use crate::context::Cache;

pub(crate) fn total_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    if storage::try_get_account_meta(env, account_id).is_none() {
        return 0;
    }
    let supply = storage::get_supply_positions(env, account_id);
    if supply.is_empty() {
        return 0;
    }

    let mut cache = Cache::new_view(env);
    risk::sum_supply_usd(env, &mut cache, &supply, risk::PositionValueMode::Neutral).raw()
}

pub(crate) fn total_borrow_in_usd(env: &Env, account_id: u64) -> i128 {
    if storage::try_get_account_meta(env, account_id).is_none() {
        return 0;
    }
    let borrow = storage::get_debt_positions(env, account_id);
    if borrow.is_empty() {
        return 0;
    }

    let mut cache = Cache::new_view(env);
    risk::sum_debt_usd(env, &mut cache, &borrow, risk::PositionValueMode::Neutral).raw()
}

pub(crate) fn ltv_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    let Some(mut account) = storage::try_get_account(env, account_id) else {
        return 0;
    };
    let mut cache = Cache::new_view(env);
    let _ = risk::restamp_listed_supply_safe_params(&mut cache, &mut account);
    risk::calculate_ltv_collateral_wad(env, &mut cache, account.spoke_id, &account.supply_positions)
        .raw()
}
