//! Integrator preview views for supply, borrow, and withdraw limits.

use crate::risk;
use crate::storage;
use common::constants::RAY;
use common::math::fp::{Ray, Wad};
use common::math::fp_core;
use common::rates::{scaled_to_original, utilization};
use common::types::{Account, HubAssetKey};
use soroban_sdk::Env;

use crate::context::Cache;

mod borrow;
mod supply;
mod withdraw;

pub use borrow::max_borrow;
pub use supply::max_supply;
pub use withdraw::max_withdraw;

/// Pool-side market state with simulated indexes.
struct MarketLimitCtx {
    // dimensional: pool totals are scaled shares; indexes convert to Token(asset).
    supplied: Ray,
    borrowed: Ray,
    cash: i128,
    max_utilization: Ray,
    supply_index: Ray,
    decimals: u32,
    borrow_index: Ray,
}

impl MarketLimitCtx {
    /// Loads pool state and simulated indexes for `hub_asset` into a limit context.
    fn load(cache: &mut Cache, hub_asset: &HubAssetKey) -> Self {
        let index = cache.cached_market_index(hub_asset);
        let sync = cache.cached_pool_sync_data(hub_asset);
        Self {
            supplied: Ray::from(sync.state.supplied),
            borrowed: Ray::from(sync.state.borrowed),
            cash: sync.state.cash,
            max_utilization: Ray::from(sync.params.max_utilization),
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
        let min_supplied = Ray::from(fp_core::mul_div_ceil(
            env,
            borrowed_orig.raw(),
            RAY,
            self.max_utilization.raw(),
        ));
        if self.supplied <= min_supplied {
            return 0;
        }
        let max_scaled_out = self.supplied - min_supplied;
        let util_cap =
            scaled_to_original(env, max_scaled_out, self.supply_index).to_asset(self.decimals);
        cap.min(util_cap)
    }

    /// Mirrors pool post-withdraw reserve, utilization, and solvency guards.
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

/// Replica of `require_post_pool_risk_gates` LTV/HF legs; HF >= 1 in
/// floor division is equivalent to `weighted >= debt`.
fn account_gates_ok(env: &Env, cache: &mut Cache, account: &Account) -> bool {
    if account.borrow_positions.is_empty() {
        return true;
    }
    let totals = risk::calculate_account_risk_totals(
        env,
        cache,
        account.spoke_id,
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
    floor == 0 || totals.ltv_collateral.raw() >= floor
}
