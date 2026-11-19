pub(crate) mod units;

pub use crate::oracle::config::*;
pub use units::*;

use common::math::fp::Ray;
use common::types::{HubAssetKey, PoolStateRaw};
use soroban_sdk::{Address, Env};

pub const HARNESS_HUB: u32 = 1;

pub const HARNESS_SPOKE: u32 = 1;

pub fn hub_asset(asset: Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: HARNESS_HUB,
        asset,
    }
}

/// Books `amount` of seeded liquidity as cash plus the supply shares that own
/// it, floored at the current supply index as a supply would mint them.
///
/// The shares have no owner and no spoke usage. They keep the market from
/// having debt without supply (INV-ACCT-09) and never leave the pool.
pub fn seed_liquidity(env: &Env, state: &mut PoolStateRaw, amount: i128, decimals: u32) {
    let shares =
        Ray::from_asset(env, amount, decimals).div_floor(env, Ray::from(state.supply_index));
    state.cash += amount;
    state.supplied = Ray::from(state.supplied).checked_add(env, shares).raw();
}
