//! Health-factor, LTV, dust-floor, and position-map helpers.
//!
//! These helpers compute over data supplied by callers. Price and index reads
//! still go through `ControllerCache`, so the active `OraclePolicy` remains the
//! caller's responsibility.

use common::math::fp::{Bps, Ray, Wad};
use common::types::{AccountPositionRaw, DebtPositionRaw};
use soroban_sdk::{Address, Env, Map};

use crate::cache::ControllerCache;
use crate::storage::{iter_debt_positions, iter_typed_positions};

/// USD WAD value of a scaled position at the supplied index and price.
pub fn position_value(env: &Env, scaled: Ray, index: Ray, price: Wad) -> Wad {
    let actual = scaled.mul(env, index);
    let actual_wad = actual.to_wad();
    actual_wad.mul(env, price)
}

/// Collateral value weighted by liquidation threshold in BPS.
pub fn weighted_collateral(env: &Env, value: Wad, threshold: Bps) -> Wad {
    threshold.apply_to_wad(env, value)
}

// LTV-weighted USD value sum of all supply positions.
pub fn calculate_ltv_collateral_wad(
    env: &Env,
    cache: &mut ControllerCache,
    supply_positions: &Map<Address, AccountPositionRaw>,
) -> Wad {
    let mut ltv = Wad::ZERO;
    for (asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = position_value(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );

        ltv += position.loan_to_value.apply_to_wad(env, value);
    }
    ltv
}

pub fn calculate_health_factor(
    env: &Env,
    cache: &mut ControllerCache,
    supply_positions: &Map<Address, AccountPositionRaw>,
    borrow_positions: &Map<Address, DebtPositionRaw>,
) -> Wad {
    if borrow_positions.is_empty() {
        return Wad::from_raw(i128::MAX); // No debt means infinite HF.
    }

    let (_, total_borrow, weighted_collateral_total) =
        calculate_account_totals(env, cache, supply_positions, borrow_positions);

    if total_borrow == Wad::ZERO {
        return Wad::from_raw(i128::MAX);
    }

    weighted_collateral_total.div_floor(env, total_borrow)
}

pub fn calculate_account_totals(
    env: &Env,
    cache: &mut ControllerCache,
    supply_positions: &Map<Address, AccountPositionRaw>,
    borrow_positions: &Map<Address, DebtPositionRaw>,
) -> (Wad, Wad, Wad) {
    // Private implementation can be summarized for Certora without changing callers.
    _calculate_account_totals_impl(env, cache, supply_positions, borrow_positions)
}

/// Internal implementation of the account total aggregation logic.
fn _calculate_account_totals_impl(
    env: &Env,
    cache: &mut ControllerCache,
    supply_positions: &Map<Address, AccountPositionRaw>,
    borrow_positions: &Map<Address, DebtPositionRaw>,
) -> (Wad, Wad, Wad) {
    let mut total_collateral = Wad::ZERO;
    let mut weighted_coll = Wad::ZERO;

    for (asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = position_value(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );

        total_collateral += value;
        weighted_coll += weighted_collateral(env, value, position.liquidation_threshold);
    }

    let total_debt = calculate_total_debt_wad(env, cache, borrow_positions);

    (total_collateral, total_debt, weighted_coll)
}

pub fn calculate_total_debt_wad(
    env: &Env,
    cache: &mut ControllerCache,
    borrow_positions: &Map<Address, DebtPositionRaw>,
) -> Wad {
    let mut total_debt = Wad::ZERO;
    for (asset, position) in iter_debt_positions(borrow_positions) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = position_value(
            env,
            position.scaled_amount,
            market_index.borrow_index,
            feed.price,
        );

        total_debt += value;
    }
    total_debt
}

use common::errors::CollateralError;
use common::types::{Account, AccountMeta, AccountPosition, DebtPosition, PositionMode};
use soroban_sdk::{panic_with_error, Vec};

use crate::emode;
use crate::storage;

#[derive(Clone, Copy)]
enum Side {
    Supply,
    Borrow,
}

/// Rejects sub-floor supply residue only for assets mutated by this action.
///
/// Closed positions are skipped, and unrelated positions cannot block a call
/// because their USD value drifted after an earlier transaction.
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

/// Rejects sub-floor debt residue only for assets mutated by this action.
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
        return;
    }
    if floor_wad == 0 {
        return;
    }
    let feed = cache.cached_price(asset);
    let market_index = cache.cached_market_index(asset);
    let index = match side {
        Side::Supply => market_index.supply_index,
        Side::Borrow => market_index.borrow_index,
    };
    let value_wad = position_value(env, scaled, index, feed.price);
    if value_wad > Wad::ZERO && value_wad.raw() < floor_wad {
        panic_with_error!(env, CollateralError::DustResidueNotAllowed);
    }
}

/// Creates account metadata and returns an empty in-memory account snapshot.
pub fn create_account(
    env: &Env,
    owner: &Address,
    e_mode_category: u32,
    mode: PositionMode,
    is_isolated: bool,
    isolated_asset: Option<Address>,
) -> (u64, Account) {
    emode::validate_e_mode_isolation_exclusion(env, e_mode_category, is_isolated);
    emode::active_e_mode_category(env, e_mode_category);

    let account_id = storage::increment_account_nonce(env);
    let account = Account {
        owner: owner.clone(),
        is_isolated,
        e_mode_category_id: e_mode_category,
        mode,
        isolated_asset,
        supply_positions: Map::new(env),
        borrow_positions: Map::new(env),
    };
    storage::set_account_meta(
        env,
        account_id,
        &AccountMeta {
            owner: owner.clone(),
            is_isolated,
            e_mode_category_id: e_mode_category,
            mode,
            isolated_asset: account.isolated_asset.clone(),
        },
    );

    (account_id, account)
}

pub fn remove_account(env: &Env, account_id: u64) {
    storage::remove_account_entry(env, account_id);
}

pub fn cleanup_account_if_empty(env: &Env, account: &Account, account_id: u64) {
    if account.is_empty() {
        remove_account(env, account_id);
    }
}

/// Upserts collateral position or removes it when the scaled supply share is zero.
pub fn update_or_remove_supply_position(
    account: &mut Account,
    asset: &Address,
    position: &AccountPosition,
) -> bool {
    if position.scaled_amount == Ray::ZERO {
        account.supply_positions.remove(asset.clone());
        true
    } else {
        account.supply_positions.set(asset.clone(), position.into());
        false
    }
}

/// Upserts debt position or removes it when the scaled debt share is zero.
pub fn update_or_remove_debt_position(
    account: &mut Account,
    asset: &Address,
    position: &DebtPosition,
) -> bool {
    if position.scaled_amount == Ray::ZERO {
        account.borrow_positions.remove(asset.clone());
        true
    } else {
        account.borrow_positions.set(asset.clone(), position.into());
        false
    }
}
