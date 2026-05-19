//! Per-position dust floor. Rejects any account state where a
//! position's USD value falls in the open interval `(0, floor)`:
//!
//!   * `value == 0`        — position closed, allowed.
//!   * `value >= floor`    — meaningful position, allowed.
//!   * `0 < value < floor` — sub-threshold residue, reverts with
//!                            `DustResidueNotAllowed`.
//!
//! The floor is per-market (`AssetConfig::min_collat_floor_usd_wad`,
//! `AssetConfig::min_debt_floor_usd_wad`) and bounded at admin time to
//! be `>= MIN_DUST_FLOOR_WAD`. Liquidation has an escape hatch in
//! [`crate::helpers::estimate_liquidation_amount`] that expands to a
//! full close when the partial would otherwise leave dust.
//!
//! # Per-operation scope
//!
//! The gate enforces "no NEW dust from THIS operation": it iterates
//! only the positions present on the `Account` value passed in. Supply
//! and repay load just the side they mutate and leave the other side
//! empty so a price / index drift on an untouched side cannot block a
//! legitimate user action.
//!
//! Drift-caused dust is cleared by:
//!   1. The position owner (`repay`, `withdraw_all`,
//!      `repay_debt_with_collateral`).
//!   2. Bad-debt socialization (`check_bad_debt_after_liquidation`)
//!      when the account is genuinely under water.

use common::errors::CollateralError;
use common::fp::{Ray, Wad};
use common::types::Account;
use soroban_sdk::{panic_with_error, Address, Env};

use crate::cache::ControllerCache;
use crate::helpers;

/// Direction tag for diagnostics-friendly error context. Both sides use the
/// same error code today; if future work splits supply/borrow dust errors,
/// they branch here.
#[derive(Clone, Copy)]
enum Side {
    Supply,
    Borrow,
}

/// Asserts that every position on the account has either zero USD value
/// or a value at or above the per-asset floor. Called from every
/// state-changing entry on the controller post-mutation.
///
/// Argument is `&Account` (not `&mut`) — the dust gate is a read-only
/// invariant check that runs *after* the entry has staged its mutations in
/// memory. Single call site per entry is sufficient because the helper
/// iterates both sides of the account.
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
