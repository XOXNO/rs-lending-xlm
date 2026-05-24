// Rejects positions below USD dust floor.

use common::errors::CollateralError;
use common::math::fp::{Ray, Wad};
use common::types::Account;
use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::cache::ControllerCache;
use crate::helpers;

// Dust check side.
#[derive(Clone, Copy)]
enum Side {
    Supply,
    Borrow,
}

// Asserts dust floor only on the supply positions for the listed assets.
// Callers that mutate supply pass the assets they touched; pre-existing
// positions on other assets (which the user did not touch) are out of
// scope and must not be allowed to block an unrelated action. Missing
// entries (position closed mid-tx) are skipped.
pub fn require_no_supply_dust_for_assets(
    env: &Env,
    cache: &mut ControllerCache,
    account: &Account,
    assets: &Vec<Address>,
) {
    check_assets_side(
        env,
        cache,
        assets,
        Side::Supply,
        |asset| {
            account
                .supply_positions
                .get(asset.clone())
                .map(|raw| Ray::from_raw(raw.scaled_amount_ray))
        },
        |cfg| cfg.min_collat_floor_usd.raw(),
    );
}

// Asserts dust floor only on the borrow positions for the listed assets.
// Mirrors [`require_no_supply_dust_for_assets`] for paths that mutate
// borrow positions (borrow, repay, liquidation repay leg, strategy debt
// leg). Pre-existing positions on other debt assets — including any that
// drifted sub-floor from price moves or interest accrual — are not the
// current action's concern and must not block it.
pub fn require_no_borrow_dust_for_assets(
    env: &Env,
    cache: &mut ControllerCache,
    account: &Account,
    assets: &Vec<Address>,
) {
    check_assets_side(
        env,
        cache,
        assets,
        Side::Borrow,
        |asset| {
            account
                .borrow_positions
                .get(asset.clone())
                .map(|raw| Ray::from_raw(raw.scaled_amount_ray))
        },
        |cfg| cfg.min_debt_floor_usd.raw(),
    );
}

fn check_assets_side(
    env: &Env,
    cache: &mut ControllerCache,
    assets: &Vec<Address>,
    side: Side,
    scaled_for: impl Fn(&Address) -> Option<Ray>,
    floor_for: impl Fn(&common::types::AssetConfig) -> i128,
) {
    for asset in assets.iter() {
        let Some(scaled) = scaled_for(&asset) else {
            continue;
        };
        let cfg = cache.cached_asset_config(&asset);
        check_position(env, cache, &asset, scaled, floor_for(&cfg), side);
    }
}

fn check_position(
    env: &Env,
    cache: &mut ControllerCache,
    asset: &Address,
    scaled: Ray,
    floor_wad: i128,
    side: Side,
) {
    if scaled == Ray::ZERO {
        return; // Position is closed; no dust possible.
    }
    if floor_wad == 0 {
        return; // Floor disabled (sentinel — admin-time test setups).
    }
    let feed = cache.cached_price(asset);
    let market_index = cache.cached_market_index(asset);
    let index = match side {
        Side::Supply => market_index.supply_index,
        Side::Borrow => market_index.borrow_index,
    };
    let value_wad = helpers::position_value(env, scaled, index, feed.price);
    if value_wad > Wad::ZERO && value_wad.raw() < floor_wad {
        panic_with_error!(env, CollateralError::DustResidueNotAllowed);
    }
}
