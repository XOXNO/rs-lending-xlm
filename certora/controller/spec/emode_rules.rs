//! E-Mode constraint rules: whitelist, deprecation, and parameter overrides.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Vec};

/// Supply of an asset not registered in the account's e-mode category must revert.
#[rule]
fn emode_only_registered_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);

    let asset_cats = crate::storage::get_asset_emodes(&e, &asset);
    cvlr_assume!(!asset_cats.contains(attrs.e_mode_category_id));

    let mut assets: Vec<(Address, i128)> = Vec::new(&e);
    assets.push_back((asset, amount));
    crate::positions::supply::process_supply(
        &e,
        &caller,
        account_id,
        attrs.e_mode_category_id,
        &assets,
    );

    cvlr_satisfy!(false);
}

/// Borrow of an asset not registered in the account's e-mode category must revert.
#[rule]
fn emode_borrow_only_registered_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);

    let asset_cats = crate::storage::get_asset_emodes(&e, &asset);
    cvlr_assume!(!asset_cats.contains(attrs.e_mode_category_id));

    let mut borrows: Vec<(Address, i128)> = Vec::new(&e);
    borrows.push_back((asset, amount));
    crate::positions::borrow::process_borrow(&e, &caller, account_id, &borrows);

    cvlr_satisfy!(false);
}

/// Borrow of a registered asset with `is_borrowable = false` must revert.
#[rule]
fn emode_only_borrowable_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);

    let emode_config = crate::storage::get_emode_asset(&e, attrs.e_mode_category_id, &asset);
    cvlr_assume!(emode_config.is_some());
    let cfg = emode_config.unwrap();
    cvlr_assume!(!cfg.is_borrowable);

    let mut borrows: Vec<(Address, i128)> = Vec::new(&e);
    borrows.push_back((asset, amount));
    crate::positions::borrow::process_borrow(&e, &caller, account_id, &borrows);

    cvlr_satisfy!(false);
}

/// Supply of a registered asset with `is_collateralizable = false` must revert.
#[rule]
fn emode_only_collateralizable_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);

    let emode_config = crate::storage::get_emode_asset(&e, attrs.e_mode_category_id, &asset);
    cvlr_assume!(emode_config.is_some());
    let cfg = emode_config.unwrap();
    cvlr_assume!(!cfg.is_collateralizable);

    let mut assets: Vec<(Address, i128)> = Vec::new(&e);
    assets.push_back((asset, amount));
    crate::positions::supply::process_supply(
        &e,
        &caller,
        account_id,
        attrs.e_mode_category_id,
        &assets,
    );

    cvlr_satisfy!(false);
}

/// New supply into a deprecated e-mode category must revert.
#[rule]
fn deprecated_emode_blocks_new_supply(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);

    let category = crate::storage::get_emode_category(&e, attrs.e_mode_category_id);
    cvlr_assume!(category.is_deprecated);

    let mut assets: Vec<(Address, i128)> = Vec::new(&e);
    assets.push_back((asset, amount));
    crate::positions::supply::process_supply(
        &e,
        &caller,
        account_id,
        attrs.e_mode_category_id,
        &assets,
    );

    cvlr_satisfy!(false);
}

/// New borrow from a deprecated e-mode category must revert.
#[rule]
fn deprecated_emode_blocks_new_borrow(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);

    let category = crate::storage::get_emode_category(&e, attrs.e_mode_category_id);
    cvlr_assume!(category.is_deprecated);

    let mut borrows: Vec<(Address, i128)> = Vec::new(&e);
    borrows.push_back((asset, amount));
    crate::positions::borrow::process_borrow(&e, &caller, account_id, &borrows);

    cvlr_satisfy!(false);
}

/// Withdrawals remain allowed in deprecated categories; scaled deposit decreases or position closes.
#[rule]
fn deprecated_emode_allows_withdraw(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);

    let category = crate::storage::get_emode_category(&e, attrs.e_mode_category_id);
    cvlr_assume!(category.is_deprecated);

    let position = crate::storage::get_position(
        &e,
        account_id,
        crate::types::AccountPositionType::Deposit,
        &asset,
    );
    cvlr_assume!(position.is_some());
    let pos_before = position.unwrap();
    cvlr_assume!(pos_before.scaled_amount_ray > 0);
    let scaled_before = pos_before.scaled_amount_ray;

    let mut withdrawals: Vec<(Address, i128)> = Vec::new(&e);
    withdrawals.push_back((asset.clone(), amount));
    crate::positions::withdraw::process_withdraw(&e, &caller, account_id, &withdrawals, None);

    let position_after = crate::storage::get_position(
        &e,
        account_id,
        crate::types::AccountPositionType::Deposit,
        &asset,
    );
    match position_after {
        None => {
            cvlr_assert!(true);
        }
        Some(pos_after) => {
            cvlr_assert!(pos_after.scaled_amount_ray < scaled_before);
        }
    }
}

/// Active e-mode overrides LTV, threshold, bonus, and collateral/borrow flags from the category.
#[rule]
fn emode_overrides_asset_params(e: Env, asset: Address, category_id: u32) {
    cvlr_assume!(category_id > 0);

    let category = crate::storage::get_emode_category(&e, category_id);
    // Deprecated categories skip override; pin to the active branch.
    cvlr_assume!(!category.is_deprecated);

    let emode_asset = crate::storage::get_emode_asset(&e, category_id, &asset);
    cvlr_assume!(emode_asset.is_some());

    let asset_cats = crate::storage::get_asset_emodes(&e, &asset);
    cvlr_assume!(asset_cats.contains(category_id));

    let mut asset_config = crate::types::AssetConfig::from(
        &crate::storage::get_market_config(&e, &asset).asset_config,
    );
    let emode_cat = crate::emode::e_mode_category(&e, category_id);
    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let emode_asset_cfg = cache.cached_emode_asset(category_id, &asset);
    crate::emode::apply_e_mode_to_asset_config(&e, &mut asset_config, &emode_cat, emode_asset_cfg);

    cvlr_assert!(asset_config.loan_to_value.raw() == i128::from(category.loan_to_value_bps));
    cvlr_assert!(
        asset_config.liquidation_threshold.raw() == i128::from(category.liquidation_threshold_bps)
    );
    cvlr_assert!(
        asset_config.liquidation_bonus.raw() == i128::from(category.liquidation_bonus_bps)
    );

    let cfg = emode_asset.unwrap();
    cvlr_assert!(asset_config.is_collateralizable == cfg.is_collateralizable);
    cvlr_assert!(asset_config.is_borrowable == cfg.is_borrowable);
}

/// Non-deprecated e-mode categories satisfy LTV < liquidation threshold.
#[rule]
fn emode_category_has_valid_params(e: Env, category_id: u32) {
    cvlr_assume!(category_id > 0);

    let category = crate::storage::get_emode_category(&e, category_id);
    cvlr_assume!(!category.is_deprecated);

    cvlr_assert!(category.liquidation_threshold_bps > category.loan_to_value_bps);
}

/// `add_e_mode_category` persists only categories with threshold > LTV.
#[rule]
fn add_emode_enforces_valid_bounds(e: Env, ltv: u32, threshold: u32, bonus: u32) {
    let id = crate::governance::config::add_e_mode_category(&e, ltv, threshold, bonus);

    let category = crate::storage::get_emode_category(&e, id);
    cvlr_assert!(category.liquidation_threshold_bps > category.loan_to_value_bps);
}

/// `edit_e_mode_category` leaves threshold > LTV in storage.
#[rule]
fn edit_emode_enforces_valid_bounds(
    e: Env,
    category_id: u32,
    ltv: u32,
    threshold: u32,
    bonus: u32,
) {
    cvlr_assume!(category_id > 0);

    crate::governance::config::edit_e_mode_category(&e, category_id, ltv, threshold, bonus);

    let category = crate::storage::get_emode_category(&e, category_id);
    cvlr_assert!(category.liquidation_threshold_bps > category.loan_to_value_bps);
}

/// `remove_e_mode_category` deprecates the category, clears its asset map, and updates reverse indexes.
#[rule]
fn emode_remove_category(e: Env, category_id: u32) {
    cvlr_assume!(category_id > 0);

    let members_before = crate::storage::get_emode_assets(&e, category_id);
    cvlr_assume!(!members_before.is_empty());
    cvlr_assume!(members_before.len() <= 5);
    let sample_asset = members_before.keys().get(0).unwrap();
    let cats_before = crate::storage::get_asset_emodes(&e, &sample_asset);
    cvlr_assume!(cats_before.contains(category_id));
    let cats_before_len = cats_before.len();

    crate::governance::config::remove_e_mode_category(&e, category_id);

    let category = crate::storage::get_emode_category(&e, category_id);
    cvlr_assert!(category.is_deprecated);

    let members_after = crate::storage::get_emode_assets(&e, category_id);
    cvlr_assert!(members_after.is_empty());

    let cats_after = crate::storage::get_asset_emodes(&e, &sample_asset);
    cvlr_assert!(!cats_after.contains(category_id));

    if cats_before_len == 1 {
        let market_after = crate::storage::get_market_config(&e, &sample_asset);
        cvlr_assert!(market_after.asset_config.e_mode_categories.is_empty());
    }
}

/// Adding an asset to a deprecated category must revert.
#[rule]
fn emode_add_asset_to_deprecated_category(e: Env, asset: Address, category_id: u32) {
    cvlr_assume!(category_id > 0);

    let category = crate::storage::try_get_emode_category(&e, category_id);
    cvlr_assume!(category.is_some());
    cvlr_assume!(category.unwrap().is_deprecated);

    crate::governance::config::add_asset_to_e_mode_category(&e, asset, category_id, true, true);

    cvlr_satisfy!(false);
}

#[rule]
fn emode_supply_sanity(
    e: Env,
    caller: Address,
    account_id: u64,
    e_mode_category: u32,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);
    cvlr_assume!(e_mode_category > 0);

    let mut assets: Vec<(Address, i128)> = Vec::new(&e);
    assets.push_back((asset, amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, e_mode_category, &assets);
    cvlr_satisfy!(true);
}

#[rule]
fn emode_borrow_sanity(e: Env, caller: Address, account_id: u64, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);

    let mut borrows: Vec<(Address, i128)> = Vec::new(&e);
    borrows.push_back((asset, amount));
    crate::positions::borrow::process_borrow(&e, &caller, account_id, &borrows);
    cvlr_satisfy!(true);
}
