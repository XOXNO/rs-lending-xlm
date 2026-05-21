// Rejects positions below USD dust floor.

use common::errors::CollateralError;
use common::math::fp::{Ray, Wad};
use common::types::Account;
use soroban_sdk::{panic_with_error, Address, Env};

use crate::cache::ControllerCache;
use crate::helpers;

// Dust check side.
#[derive(Clone, Copy)]
enum Side {
    Supply,
    Borrow,
}

// Asserts positions are zero or above floor.
pub fn require_no_dust_after(env: &Env, cache: &mut ControllerCache, account: &Account) {
    for (asset, position) in account.supply_positions.iter() {
        let cfg = cache.cached_asset_config(&asset);
        check_position(
            env,
            cache,
            &asset,
            position.scaled_amount_ray,
            cfg.min_collat_floor_usd_wad,
            Side::Supply,
        );
    }
    for (asset, position) in account.borrow_positions.iter() {
        let cfg = cache.cached_asset_config(&asset);
        check_position(
            env,
            cache,
            &asset,
            position.scaled_amount_ray,
            cfg.min_debt_floor_usd_wad,
            Side::Borrow,
        );
    }
}

fn check_position(
    env: &Env,
    cache: &mut ControllerCache,
    asset: &Address,
    scaled_ray: i128,
    floor_wad: i128,
    side: Side,
) {
    if scaled_ray == 0 {
        return; // Position is closed; no dust possible.
    }
    if floor_wad == 0 {
        return; // Floor disabled (sentinel — admin-time test setups).
    }
    let feed = cache.cached_price(asset);
    let market_index = cache.cached_market_index(asset);
    let index_ray = match side {
        Side::Supply => market_index.supply_index_ray,
        Side::Borrow => market_index.borrow_index_ray,
    };
    let value_wad = helpers::position_value(
        env,
        Ray::from_raw(scaled_ray),
        Ray::from_raw(index_ray),
        Wad::from_raw(feed.price_wad),
    );
    if value_wad > Wad::ZERO && value_wad.raw() < floor_wad {
        panic_with_error!(env, CollateralError::DustResidueNotAllowed);
    }
}
