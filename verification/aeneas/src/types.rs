//! Stand-in for `common::types::MarketParams`, limited to the fields the rate
//! code reads. The production struct also carries `is_flashloanable`,
//! `flashloan_fee`, `asset_id` and `asset_decimals`; `curve.rs` and
//! `index.rs` read none of them.

use crate::math::fp::{Bps, Ray};

#[derive(Clone, Debug)]
pub struct MarketParams {
    pub max_borrow_rate: Ray,
    pub base_borrow_rate: Ray,
    pub slope1: Ray,
    pub slope2: Ray,
    pub slope3: Ray,
    pub mid_utilization: Ray,
    pub optimal_utilization: Ray,
    pub max_utilization: Ray,
    pub reserve_factor: Bps,
}
