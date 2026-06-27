//! Integrator preview views: `max_supply`, `max_borrow`, and `max_withdraw`.
//!
//! `max_withdraw` first tries a full close, then caps a partial by closed-form
//! pool solvency headroom and settles with a short stroop walk
//! against the exact `partial_ok` replica of the mutating path. Indexes keep
//! accruing after the read, so callers acting later should leave a margin.

use common::constants::RAY;
use common::math::fp::{Ray, Wad};
use common::math::fp_core;
use common::rates::{scaled_to_original, utilization};
use controller_interface::types::Account;
use soroban_sdk::{Address, Env};

use crate::cache::Cache;
use crate::{helpers, storage};

mod borrow;
mod supply;
mod withdraw;

pub use borrow::max_borrow;
pub use supply::max_supply;
pub use withdraw::max_withdraw;

/// Pool-side market state at view-simulated indexes.
struct MarketLimitCtx {
    // dimensional: pool totals are scaled shares; indexes convert to Token(asset).
    supplied: Ray,
    borrowed: Ray,
    // dimensional: cash is Token(asset) in asset-native decimals.
    cash: i128,
    max_utilization: Ray,
    // dimensional: supply/borrow indexes are Ray<Index(asset, side)>.
    supply_index: Ray,
    // dimensional: asset decimals for Token(asset) <-> Ray rescale.
    decimals: u32,
    borrow_index: Ray,
}

impl MarketLimitCtx {
    fn load(cache: &mut Cache, asset: &Address) -> Self {
        let index = cache.cached_market_index(asset);
        let sync = cache.cached_pool_sync_data(asset);
        Self {
            supplied: Ray::from(sync.state.supplied_ray),
            borrowed: Ray::from(sync.state.borrowed_ray),
            cash: sync.state.cash,
            max_utilization: Ray::from(sync.params.max_utilization_ray),
            supply_index: index.supply_index,
            decimals: sync.params.asset_decimals,
            borrow_index: index.borrow_index,
        }
    }

    /// Largest partial bounded by cash and post-withdraw utilization.
    fn pool_partial_cap(&self, env: &Env, full_request: i128) -> i128 {
        let cap = self.cash.min(full_request);
        if self.max_utilization >= Ray::ONE || self.borrowed == Ray::ZERO {
            return cap;
        }
        let borrowed_orig = scaled_to_original(env, self.borrowed, self.borrow_index);
        if borrowed_orig == Ray::ZERO {
            return cap;
        }
        let min_supplied = ray_div_ceil(env, borrowed_orig, self.max_utilization);
        if self.supplied <= min_supplied {
            return 0;
        }
        let max_scaled_out = self.supplied - min_supplied;
        let util_cap =
            scaled_to_original(env, max_scaled_out, self.supply_index).to_asset(self.decimals);
        cap.min(util_cap)
    }

    /// Mirrors the pool's post-withdraw reserve, utilization, and solvency
    /// guards for an outflow of `transfer_out` units burning `scaled_out`.
    fn pool_state_ok(&self, env: &Env, scaled_out: Ray, transfer_out: i128) -> bool {
        if transfer_out > self.cash || scaled_out > self.supplied {
            return false;
        }
        let post_supplied = self.supplied - scaled_out;
        if post_supplied == Ray::ZERO {
            return self.borrowed == Ray::ZERO;
        }
        if self.max_utilization >= Ray::ONE {
            return true;
        }
        let util = utilization(
            env,
            scaled_to_original(env, self.borrowed, self.borrow_index),
            scaled_to_original(env, post_supplied, self.supply_index),
        );
        util <= self.max_utilization
    }
}

fn ray_div_ceil(env: &Env, num: Ray, den: Ray) -> Ray {
    Ray::from(fp_core::mul_div_ceil(env, num.raw(), RAY, den.raw()))
}

/// Replica of `require_post_pool_risk_gates` LTV/HF legs; HF >= 1 in
/// floor division is equivalent to `weighted >= debt`.
fn account_gates_ok(env: &Env, cache: &mut Cache, account: &Account) -> bool {
    if account.borrow_positions.is_empty() {
        return true;
    }
    let totals = helpers::calculate_account_risk_totals(
        env,
        cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    if totals.total_debt == Wad::ZERO {
        return true;
    }
    let debt = totals.total_debt.raw();
    if totals.weighted_collateral.raw() < debt {
        return false;
    }
    if totals.ltv_collateral.raw() < debt {
        return false;
    }
    let floor = storage::get_min_borrow_collateral_usd_wad(env);
    // dimensional: floor and ltv_collateral are Wad<USD>.
    floor == 0 || totals.ltv_collateral.raw() >= floor
}
