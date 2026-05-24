// Rejects positions below USD dust floor.

use common::errors::CollateralError;
use common::math::fp::{Ray, Wad};
use common::types::{Account, AccountPosition, AccountPositionRaw};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

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
        &account.supply_positions,
        assets,
        Side::Supply,
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
        &account.borrow_positions,
        assets,
        Side::Borrow,
        |cfg| cfg.min_debt_floor_usd.raw(),
    );
}

fn check_assets_side(
    env: &Env,
    cache: &mut ControllerCache,
    positions: &Map<Address, AccountPositionRaw>,
    assets: &Vec<Address>,
    side: Side,
    floor_for: impl Fn(&common::types::AssetConfig) -> i128,
) {
    for asset in assets.iter() {
        let Some(raw) = positions.get(asset.clone()) else {
            continue;
        };
        let position = AccountPosition::from(&raw);
        let cfg = cache.cached_asset_config(&asset);
        check_position(
            env,
            cache,
            &asset,
            position.scaled_amount,
            floor_for(&cfg),
            side,
        );
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
