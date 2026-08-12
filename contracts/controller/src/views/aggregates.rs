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
    let borrow = storage::get_debt_positions(env, account_id);
    risk::calculate_account_risk_totals(env, &mut cache, &supply, &borrow)
        .total_collateral
        .raw()
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
    let supply = storage::get_supply_positions(env, account_id);
    risk::calculate_account_risk_totals(env, &mut cache, &supply, &borrow)
        .total_debt
        .raw()
}

pub(crate) fn ltv_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
    let Some(mut account) = storage::try_get_account(env, account_id) else {
        return 0;
    };
    let mut cache = Cache::new_view(env);
    let _ = risk::restamp_listed_supply_ltv(&mut cache, &mut account);
    risk::calculate_account_risk_totals(env, &mut cache, &account.supply_positions, &account.borrow_positions)
        .ltv_collateral
        .raw()
}
