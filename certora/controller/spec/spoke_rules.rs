//! Spoke constraint rules: listing, deprecation, and parameter resolution.
//!
//! The refactor renamed spoke categories to spokes. A category maps to a spoke
//! id; "asset registered in the category" maps to `SpokeAsset(spoke_id, hub_asset)`
//! existing. The spec models hub 0, so each asset address resolves to
//! `HubAssetKey { hub_id: 0, asset }`.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Vec};

use crate::types::{AccountPositionType, HubAssetKey, MarketOracleConfigOption, SpokeAssetArgs};

/// Hub-0 coordinate for `asset`; the spec models the single default hub.
fn hub0(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    }
}

/// Supply of an asset not listed on the account's spoke must revert.
#[rule]
fn spoke_only_registered_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let hub_asset = hub0(&asset);
    cvlr_assume!(crate::storage::get_spoke_asset(&e, attrs.spoke_id, &hub_asset).is_none());

    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    assets.push_back((hub_asset, amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &assets);

    cvlr_satisfy!(false);
}

/// Borrow of an asset not listed on the account's spoke must revert.
#[rule]
fn spoke_borrow_only_registered_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let hub_asset = hub0(&asset);
    cvlr_assume!(crate::storage::get_spoke_asset(&e, attrs.spoke_id, &hub_asset).is_none());

    let mut borrows: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    borrows.push_back((hub_asset, amount));
    crate::positions::borrow::process_borrow(&e, &caller, account_id, &borrows, None);

    cvlr_satisfy!(false);
}

/// Borrow of a listed asset with `is_borrowable = false` must revert.
#[rule]
fn spoke_only_borrowable_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let spoke_asset = crate::storage::get_spoke_asset(&e, attrs.spoke_id, &hub0(&asset));
    cvlr_assume!(spoke_asset.is_some());
    let cfg = spoke_asset.unwrap();
    cvlr_assume!(!cfg.is_borrowable);

    let mut borrows: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    borrows.push_back((hub0(&asset), amount));
    crate::positions::borrow::process_borrow(&e, &caller, account_id, &borrows, None);

    cvlr_satisfy!(false);
}

/// Supply of a listed asset with `is_collateralizable = false` must revert.
#[rule]
fn spoke_only_collateralizable_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let spoke_asset = crate::storage::get_spoke_asset(&e, attrs.spoke_id, &hub0(&asset));
    cvlr_assume!(spoke_asset.is_some());
    let cfg = spoke_asset.unwrap();
    cvlr_assume!(!cfg.is_collateralizable);

    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    assets.push_back((hub0(&asset), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &assets);

    cvlr_satisfy!(false);
}

/// New supply into a deprecated spoke must revert.
#[rule]
fn deprecated_spoke_blocks_new_supply(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let spoke = crate::storage::get_spoke(&e, attrs.spoke_id);
    cvlr_assume!(spoke.is_deprecated);

    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    assets.push_back((hub0(&asset), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &assets);

    cvlr_satisfy!(false);
}

/// New borrow from a deprecated spoke must revert.
#[rule]
fn deprecated_spoke_blocks_new_borrow(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let spoke = crate::storage::get_spoke(&e, attrs.spoke_id);
    cvlr_assume!(spoke.is_deprecated);

    let mut borrows: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    borrows.push_back((hub0(&asset), amount));
    crate::positions::borrow::process_borrow(&e, &caller, account_id, &borrows, None);

    cvlr_satisfy!(false);
}

/// Withdrawals remain allowed on deprecated spokes; scaled deposit decreases or position closes.
#[rule]
fn deprecated_spoke_allows_withdraw(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let spoke = crate::storage::get_spoke(&e, attrs.spoke_id);
    cvlr_assume!(spoke.is_deprecated);

    let position =
        crate::storage::get_position(&e, account_id, AccountPositionType::Deposit, &asset);
    cvlr_assume!(position.is_some());
    let pos_before = position.unwrap();
    cvlr_assume!(pos_before.scaled_amount > 0);
    let scaled_before = pos_before.scaled_amount;

    let mut withdrawals: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    withdrawals.push_back((hub0(&asset), amount));
    crate::positions::withdraw::process_withdraw(&e, &caller, account_id, &withdrawals, None);

    let position_after =
        crate::storage::get_position(&e, account_id, AccountPositionType::Deposit, &asset);
    match position_after {
        None => {
            cvlr_assert!(true);
        }
        Some(pos_after) => {
            cvlr_assert!(pos_after.scaled_amount < scaled_before);
        }
    }
}

/// On an active spoke, the effective risk config projects the asset's own
/// per-spoke `SpokeAssetConfig` (LTV, threshold, bonus, collateral/borrow flags).
#[rule]
fn spoke_overrides_asset_params(e: Env, asset: Address, category_id: u32) {
    cvlr_assume!(category_id > 0);

    let spoke = crate::storage::get_spoke(&e, category_id);
    // Deprecated spokes are not the active-override case; pin to the active branch.
    cvlr_assume!(!spoke.is_deprecated);

    let hub_asset = hub0(&asset);
    let spoke_asset = crate::storage::get_spoke_asset(&e, category_id, &hub_asset);
    cvlr_assume!(spoke_asset.is_some());
    let cfg = spoke_asset.unwrap();

    // Self-contained per-spoke resolution (no base+overlay): the effective
    // config is the spoke's `SpokeAssetConfig` projected to `AssetConfig`,
    // served from the per-tx cache memo (one `SpokeAsset` read per asset).
    let mut cache = crate::context::Cache::new(&e);
    let asset_config = crate::spoke::effective_asset_config(&mut cache, category_id, &hub_asset);

    cvlr_assert!(asset_config.loan_to_value.raw() == i128::from(cfg.loan_to_value));
    cvlr_assert!(asset_config.liquidation_threshold.raw() == i128::from(cfg.liquidation_threshold));
    cvlr_assert!(asset_config.liquidation_bonus.raw() == i128::from(cfg.liquidation_bonus));

    cvlr_assert!(asset_config.is_collateralizable == cfg.is_collateralizable);
    cvlr_assert!(asset_config.is_borrowable == cfg.is_borrowable);
}

/// Listed spoke assets satisfy LTV < liquidation threshold.
#[rule]
fn spoke_asset_has_valid_params(e: Env, asset: Address, category_id: u32) {
    cvlr_assume!(category_id > 0);

    let spoke_asset = crate::storage::get_spoke_asset(&e, category_id, &hub0(&asset));
    cvlr_assume!(spoke_asset.is_some());
    let cfg = spoke_asset.unwrap();

    cvlr_assert!(cfg.liquidation_threshold > cfg.loan_to_value);
}

/// `add_asset_to_spoke` persists only assets with threshold > LTV.
#[rule]
fn add_asset_enforces_valid_bounds(
    e: Env,
    asset: Address,
    category_id: u32,
    ltv: u32,
    threshold: u32,
    bonus: u32,
) {
    cvlr_assume!(category_id > 0);

    crate::config::add_asset_to_spoke(
        &e,
        &SpokeAssetArgs {
            hub_id: 0,
            asset: asset.clone(),
            spoke_id: category_id,
            can_collateral: true,
            can_borrow: true,
            paused: false,
            frozen: false,
            ltv,
            threshold,
            bonus,
            liquidation_fees: 0,
            supply_cap: 0,
            borrow_cap: 0,
            oracle_override: MarketOracleConfigOption::None,
        },
    );

    let cfg = crate::storage::get_spoke_asset(&e, category_id, &hub0(&asset)).unwrap();
    cvlr_assert!(cfg.liquidation_threshold > cfg.loan_to_value);
}

/// `edit_asset_in_spoke` leaves threshold > LTV in storage.
#[rule]
fn edit_asset_enforces_valid_bounds(
    e: Env,
    asset: Address,
    category_id: u32,
    ltv: u32,
    threshold: u32,
    bonus: u32,
) {
    cvlr_assume!(category_id > 0);

    crate::config::edit_asset_in_spoke(
        &e,
        &SpokeAssetArgs {
            hub_id: 0,
            asset: asset.clone(),
            spoke_id: category_id,
            can_collateral: true,
            can_borrow: true,
            paused: false,
            frozen: false,
            ltv,
            threshold,
            bonus,
            liquidation_fees: 0,
            supply_cap: 0,
            borrow_cap: 0,
            oracle_override: MarketOracleConfigOption::None,
        },
    );

    let cfg = crate::storage::get_spoke_asset(&e, category_id, &hub0(&asset)).unwrap();
    cvlr_assert!(cfg.liquidation_threshold > cfg.loan_to_value);
}

/// `remove_spoke` deprecates the spoke.
///
/// TODO(multi-hub): re-verify that removal severs asset membership. Under the
/// spoke model the per-asset `SpokeAsset(spoke_id, hub_asset)` keys are not
/// enumerable, so `remove_spoke` only flips the deprecation flag (member assets
/// and their backlinks are left in place, kept unreachable by deprecation). The
/// old "asset map cleared / reverse index updated" coverage no longer applies
/// and needs a fresh property over the deprecation-gated reads.
#[rule]
fn spoke_remove_category(e: Env, category_id: u32) {
    cvlr_assume!(category_id > 0);

    // The spoke must exist and be active so `remove_spoke` reaches its body.
    let before = crate::storage::try_get_spoke(&e, category_id);
    cvlr_assume!(matches!(&before, Some(spoke) if !spoke.is_deprecated));

    crate::config::remove_spoke(&e, category_id);

    let spoke = crate::storage::get_spoke(&e, category_id);
    cvlr_assert!(spoke.is_deprecated);
}

/// Adding an asset to a deprecated spoke must revert.
#[rule]
fn spoke_add_asset_to_deprecated_category(e: Env, asset: Address, category_id: u32) {
    cvlr_assume!(category_id > 0);

    let spoke = crate::storage::try_get_spoke(&e, category_id);
    cvlr_assume!(spoke.is_some());
    cvlr_assume!(spoke.unwrap().is_deprecated);

    crate::config::add_asset_to_spoke(
        &e,
        &SpokeAssetArgs {
            hub_id: 0,
            asset,
            spoke_id: category_id,
            can_collateral: true,
            can_borrow: true,
            paused: false,
            frozen: false,
            ltv: 9_000,
            threshold: 9_300,
            bonus: 300,
            liquidation_fees: 0,
            supply_cap: 0,
            borrow_cap: 0,
            oracle_override: MarketOracleConfigOption::None,
        },
    );

    cvlr_satisfy!(false);
}

#[rule]
fn spoke_supply_sanity(
    e: Env,
    caller: Address,
    account_id: u64,
    spoke_id: u32,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);
    cvlr_assume!(spoke_id > 0);

    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    assets.push_back((hub0(&asset), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, spoke_id, &assets);
    cvlr_satisfy!(true);
}

#[rule]
fn spoke_borrow_sanity(e: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let mut borrows: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    borrows.push_back((hub0(&asset), amount));
    crate::positions::borrow::process_borrow(&e, &caller, account_id, &borrows, None);
    cvlr_satisfy!(true);
}
