//! Certora summaries for expensive production functions.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::{Address, Env};

use crate::types::PriceFeedRaw;
use common::math::fp::Wad;
use common::types::MarketIndexRaw;

use crate::context::Cache;

pub mod pool;
pub mod reflector;
pub mod sac;

/// Token price: positive WAD price, decimals <= 27, timestamp not materially
/// ahead of the current ledger. Shared by cache and external-call summaries.
pub(crate) fn price_feed_summary(env: &Env, _asset: &Address) -> PriceFeedRaw {
    let price_wad: i128 = nondet();
    let asset_decimals: u32 = nondet();
    let timestamp: u64 = nondet();
    cvlr_assume!(price_wad > 0);
    cvlr_assume!(asset_decimals <= 27);
    cvlr_assume!(timestamp <= env.ledger().timestamp().saturating_add(60));
    PriceFeedRaw {
        price_wad,
        asset_decimals,
        timestamp,
    }
}

pub(crate) fn token_price_summary(cache: &mut Cache, asset: &Address) -> PriceFeedRaw {
    price_feed_summary(cache.env(), asset)
}

/// Pool market index inside the production band: floor from bad-debt
/// write-down, caps from the `update_*_index` clamps (models `bulk_get_indexes`).
pub fn bulk_index_summary(_env: &Env, _asset: &Address) -> MarketIndexRaw {
    let supply_index: i128 = nondet();
    let borrow_index: i128 = nondet();
    cvlr_assume!(supply_index >= common::constants::SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(supply_index <= common::constants::MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(borrow_index >= common::constants::RAY);
    cvlr_assume!(borrow_index <= common::constants::MAX_BORROW_INDEX_RAY);
    MarketIndexRaw {
        supply_index,
        borrow_index,
    }
}

/// Account risk totals: non-neg aggregates; weighted/LTV coll <= neutral coll;
/// HF from abstracted weighted coll and debt (gate relation preserved).
pub(crate) fn calculate_account_risk_totals_summary(
    env: &Env,
    _cache: &mut Cache,
    _spoke_id: u32,
    supply_positions: &soroban_sdk::Map<
        common::types::HubAssetKey,
        common::types::AccountPositionRaw,
    >,
    borrow_positions: &soroban_sdk::Map<common::types::HubAssetKey, common::types::DebtPositionRaw>,
) -> crate::risk::AccountRiskTotals {
    let total_collateral_raw: i128 = nondet();
    let ltv_collateral_raw: i128 = nondet();
    let weighted_coll_raw: i128 = nondet();
    let total_debt_raw: i128 = nondet();
    cvlr_assume!(total_collateral_raw >= 0);
    cvlr_assume!(ltv_collateral_raw >= 0);
    cvlr_assume!(weighted_coll_raw >= 0);
    cvlr_assume!(total_debt_raw >= 0);
    cvlr_assume!(weighted_coll_raw <= total_collateral_raw);
    cvlr_assume!(ltv_collateral_raw <= total_collateral_raw);
    if supply_positions.is_empty() {
        cvlr_assume!(total_collateral_raw == 0);
        cvlr_assume!(ltv_collateral_raw == 0);
        cvlr_assume!(weighted_coll_raw == 0);
    }
    if borrow_positions.is_empty() {
        cvlr_assume!(total_debt_raw == 0);
    }

    let total_debt = Wad::from(total_debt_raw);
    let weighted_collateral = Wad::from(weighted_coll_raw);
    let health_factor = if total_debt == Wad::ZERO {
        Wad::from(i128::MAX)
    } else {
        weighted_collateral.div_floor(env, total_debt)
    };

    crate::risk::AccountRiskTotals {
        total_collateral: Wad::from(total_collateral_raw),
        ltv_collateral: Wad::from(ltv_collateral_raw),
        weighted_collateral,
        total_debt,
        health_factor,
    }
}

/// Total collateral in USD, non-negative.
pub fn total_collateral_in_usd_summary(_env: &Env, _account_id: u64) -> i128 {
    let total: i128 = nondet();
    cvlr_assume!(total >= 0);
    total
}

/// Total borrow in USD, non-negative.
pub fn total_borrow_in_usd_summary(_env: &Env, _account_id: u64) -> i128 {
    let total: i128 = nondet();
    cvlr_assume!(total >= 0);
    total
}

/// LTV-weighted collateral in USD, non-negative.
pub fn ltv_collateral_in_usd_summary(_env: &Env, _account_id: u64) -> i128 {
    let total: i128 = nondet();
    cvlr_assume!(total >= 0);
    total
}
