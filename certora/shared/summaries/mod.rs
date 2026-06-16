//! Certora summaries for expensive production functions.

use cvlr::cvlr_assume;
use cvlr::nondet::nondet;
use soroban_sdk::{Address, Env};

use crate::types::PriceFeedRaw;
use common::math::fp::{Bps, Wad};
use common::types::MarketIndex;

use crate::cache::Cache;

pub mod pool;
pub mod reflector;
pub mod sac;

/// Token price: positive WAD price, decimals <= 27, timestamp within skew.
pub fn token_price_summary(cache: &mut Cache, _asset: &Address) -> PriceFeedRaw {
    let price_wad: i128 = nondet();
    let asset_decimals: u32 = nondet();
    let timestamp: u64 = nondet();
    cvlr_assume!(price_wad > 0);
    cvlr_assume!(asset_decimals <= 27);
    cvlr_assume!(timestamp <= cache.current_timestamp_ms / 1000 + 60);
    PriceFeedRaw {
        price_wad,
        asset_decimals,
        timestamp,
    }
}

/// Anchor check: nondet bool (either outcome valid).
pub fn is_within_anchor_summary(
    _env: &Env,
    _aggregator: i128,
    _safe: i128,
    _upper_bound_ratio: u32,
    _lower_bound_ratio: u32,
) -> bool {
    nondet()
}

/// Asset index update: indexes >= production floors.
pub fn update_asset_index_summary(_cache: &mut Cache, _asset: &Address) -> MarketIndex {
    use common::math::fp::Ray;
    let supply_index_ray: i128 = nondet();
    let borrow_index_ray: i128 = nondet();
    cvlr_assume!(supply_index_ray >= common::constants::SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(borrow_index_ray >= common::constants::RAY);
    MarketIndex {
        supply_index: Ray::from(supply_index_ray),
        borrow_index: Ray::from(borrow_index_ray),
    }
}

/// Account totals: non-negative collateral/debt, weighted collateral <= collateral.
pub fn calculate_account_totals_summary(
    _env: &Env,
    _cache: &mut Cache,
    _supply_positions: &soroban_sdk::Map<Address, common::types::AccountPositionRaw>,
    _borrow_positions: &soroban_sdk::Map<Address, common::types::DebtPositionRaw>,
) -> (Wad, Wad, Wad) {
    let total_collateral_raw: i128 = nondet();
    let total_debt_raw: i128 = nondet();
    let weighted_coll_raw: i128 = nondet();
    cvlr_assume!(total_collateral_raw >= 0);
    cvlr_assume!(total_debt_raw >= 0);
    cvlr_assume!(weighted_coll_raw >= 0);
    cvlr_assume!(weighted_coll_raw <= total_collateral_raw);
    (
        Wad::from(total_collateral_raw),
        Wad::from(total_debt_raw),
        Wad::from(weighted_coll_raw),
    )
}

/// Linear bonus: in `[base_bonus, max_bonus]`; equals `base_bonus` when HF >= 1.02 WAD.
pub fn calculate_linear_bonus_summary(_env: &Env, hf: Wad, base_bonus: Bps, max_bonus: Bps) -> Bps {
    let bonus_raw: i128 = nondet();
    cvlr_assume!(bonus_raw >= base_bonus.raw());
    cvlr_assume!(bonus_raw <= max_bonus.raw());
    let target_hf_wad: i128 = 102 * common::constants::WAD / 100;
    if hf.raw() >= target_hf_wad {
        cvlr_assume!(bonus_raw == base_bonus.raw());
    }
    Bps::from(bonus_raw)
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