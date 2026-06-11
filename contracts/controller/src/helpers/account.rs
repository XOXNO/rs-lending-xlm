//! Per-asset dust gates and in-memory account lifecycle.
//!
//! Dust checks reuse `position_value` from the sibling `math` module and read
//! prices/indexes through `Cache`, so the active `OraclePolicy`
//! remains the caller's responsibility.

use common::errors::CollateralError;
use common::math::fp::{Ray, Wad};
use common::types::{Account, AccountMeta, AccountPosition, DebtPosition, PositionMode};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use super::math::position_value;
use crate::cache::Cache;
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
    cache: &mut Cache,
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
                .map(|raw| Ray::from(raw.scaled_amount_ray))
        },
        |cfg| cfg.min_collat_floor_usd.raw(),
    );
}

/// Rejects sub-floor debt residue only for assets mutated by this action.
pub fn require_no_borrow_dust_for_assets(
    env: &Env,
    cache: &mut Cache,
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
                .map(|raw| Ray::from(raw.scaled_amount_ray))
        },
        |cfg| cfg.min_debt_floor_usd.raw(),
    );
}

fn check_assets_side(
    env: &Env,
    cache: &mut Cache,
    assets: &Vec<Address>,
    side: Side,
    scaled_for: impl Fn(&Address) -> Option<Ray>,
    floor_for: impl Fn(&common::types::AssetConfig) -> i128,
) {
    // Single filter pass: only open positions on markets with a non-zero
    // floor are priced; the same set feeds the prefetch and the checks.
    let mut priceable: Vec<Address> = Vec::new(env);
    for asset in assets.iter() {
        let Some(scaled) = scaled_for(&asset) else {
            continue;
        };
        if scaled == Ray::ZERO {
            continue;
        }
        let floor = floor_for(&cache.cached_asset_config(&asset));
        if floor == 0 {
            continue;
        }
        priceable.push_back(asset);
    }
    // Idempotent with earlier gates: the prefetch skips assets already in
    // prices_cache and feeds already fetched, so flows whose LTV/HF gate
    // priced these assets make zero additional oracle calls here.
    crate::oracle::prefetch_redstone_feeds(cache, &priceable);

    for asset in priceable.iter() {
        let scaled = scaled_for(&asset).expect("priceable is a filtered subset of assets");
        let cfg = cache.cached_asset_config(&asset);
        check_position(env, cache, &asset, scaled, floor_for(&cfg), side);
    }
}

fn check_position(
    env: &Env,
    cache: &mut Cache,
    asset: &Address,
    scaled: Ray,
    floor_wad: i128,
    side: Side,
) {
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
