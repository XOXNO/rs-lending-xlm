use cvlr::cvlr_assume;
use soroban_sdk::{contract, contractimpl, Address, Env};

use crate::constants::{BPS, MAX_BORROW_RATE_RAY, RAY, RAY_DECIMALS};
use crate::types::MarketParamsRaw;

#[contract]
pub struct CommonCertoraHarness;

#[contractimpl]
impl CommonCertoraHarness {
    pub fn ping(_env: Env) {}
}

/// A rate model satisfying every constraint `MarketParamsRaw::verify` enforces
/// on a listed market, with all curve parameters left symbolic.
pub fn nondet_market_params(asset: &Address) -> MarketParamsRaw {
    let base_borrow_rate: i128 = cvlr::nondet::nondet();
    let slope1: i128 = cvlr::nondet::nondet();
    let slope2: i128 = cvlr::nondet::nondet();
    let slope3: i128 = cvlr::nondet::nondet();
    let mid_utilization: i128 = cvlr::nondet::nondet();
    let optimal_utilization: i128 = cvlr::nondet::nondet();
    let max_utilization: i128 = cvlr::nondet::nondet();
    let max_borrow_rate: i128 = cvlr::nondet::nondet();
    let reserve_factor: u32 = cvlr::nondet::nondet();
    let asset_decimals: u32 = cvlr::nondet::nondet();

    cvlr_assume!((0..=MAX_BORROW_RATE_RAY).contains(&base_borrow_rate));
    cvlr_assume!(base_borrow_rate <= slope1);
    cvlr_assume!(slope1 <= slope2);
    cvlr_assume!(slope2 <= slope3);
    cvlr_assume!(slope3 <= MAX_BORROW_RATE_RAY);

    cvlr_assume!(mid_utilization > 0 && mid_utilization < optimal_utilization);
    cvlr_assume!(optimal_utilization < RAY);
    cvlr_assume!(max_utilization >= optimal_utilization && max_utilization <= RAY);

    cvlr_assume!(max_borrow_rate > 0 && max_borrow_rate <= MAX_BORROW_RATE_RAY);
    cvlr_assume!((0..BPS).contains(&i128::from(reserve_factor)));
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);

    MarketParamsRaw {
        max_borrow_rate,
        base_borrow_rate,
        slope1,
        slope2,
        slope3,
        mid_utilization,
        optimal_utilization,
        max_utilization,
        reserve_factor,
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: asset.clone(),
        asset_decimals,
    }
}
