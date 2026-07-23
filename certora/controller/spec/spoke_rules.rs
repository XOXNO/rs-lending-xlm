//! Spoke constraints: listing, deprecation, and effective risk-config resolution.
//! Rules use the production-valid primary hub.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Vec};

use crate::types::{AccountPositionType, HubAssetKey, SpokeAssetArgs};

/// Primary-hub coordinate for `asset`.
fn hub0(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: crate::spec::fixture::HUB_ID,
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
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &caller);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let hub_asset = hub0(&asset);
    cvlr_assume!(crate::storage::get_spoke_asset(&e, attrs.spoke_id, &hub_asset).is_none());

    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    assets.push_back((hub_asset, amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &assets);

    cvlr_assert!(false);
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
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &caller);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let hub_asset = hub0(&asset);
    cvlr_assume!(crate::storage::get_spoke_asset(&e, attrs.spoke_id, &hub_asset).is_none());

    let mut borrows: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    borrows.push_back((hub_asset, amount));
    crate::positions::borrow::process_borrow(&e, &caller, account_id, &borrows, None);

    cvlr_assert!(false);
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
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let mut stored =
        crate::storage::get_spoke_asset(&e, crate::spec::fixture::SPOKE_ID, &hub0(&asset)).unwrap();
    stored.is_borrowable = false;
    crate::storage::set_spoke_asset(&e, crate::spec::fixture::SPOKE_ID, &hub0(&asset), &stored);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let spoke_asset = crate::storage::get_spoke_asset(&e, attrs.spoke_id, &hub0(&asset));
    cvlr_assume!(spoke_asset.is_some());
    let cfg = spoke_asset.unwrap();
    cvlr_assume!(!cfg.is_borrowable);

    let mut borrows: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    borrows.push_back((hub0(&asset), amount));
    crate::positions::borrow::process_borrow(&e, &caller, account_id, &borrows, None);

    cvlr_assert!(false);
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
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let mut stored =
        crate::storage::get_spoke_asset(&e, crate::spec::fixture::SPOKE_ID, &hub0(&asset)).unwrap();
    stored.is_collateralizable = false;
    crate::storage::set_spoke_asset(&e, crate::spec::fixture::SPOKE_ID, &hub0(&asset), &stored);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let spoke_asset = crate::storage::get_spoke_asset(&e, attrs.spoke_id, &hub0(&asset));
    cvlr_assume!(spoke_asset.is_some());
    let cfg = spoke_asset.unwrap();
    cvlr_assume!(!cfg.is_collateralizable);

    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    assets.push_back((hub0(&asset), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &assets);

    cvlr_assert!(false);
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
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    let mut deprecated = crate::storage::get_spoke(&e, crate::spec::fixture::SPOKE_ID);
    deprecated.is_deprecated = true;
    crate::storage::set_spoke(&e, crate::spec::fixture::SPOKE_ID, &deprecated);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let spoke = crate::storage::get_spoke(&e, attrs.spoke_id);
    cvlr_assume!(spoke.is_deprecated);

    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    assets.push_back((hub0(&asset), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &assets);

    cvlr_assert!(false);
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
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    let mut deprecated = crate::storage::get_spoke(&e, crate::spec::fixture::SPOKE_ID);
    deprecated.is_deprecated = true;
    crate::storage::set_spoke(&e, crate::spec::fixture::SPOKE_ID, &deprecated);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let spoke = crate::storage::get_spoke(&e, attrs.spoke_id);
    cvlr_assume!(spoke.is_deprecated);

    let mut borrows: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    borrows.push_back((hub0(&asset), amount));
    crate::positions::borrow::process_borrow(&e, &caller, account_id, &borrows, None);

    cvlr_assert!(false);
}

#[rule]
fn deprecated_spoke_withdraw_does_not_increase_supply(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    let mut deprecated = crate::storage::get_spoke(&e, crate::spec::fixture::SPOKE_ID);
    deprecated.is_deprecated = true;
    crate::storage::set_spoke(&e, crate::spec::fixture::SPOKE_ID, &deprecated);

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
            cvlr_assert!(pos_after.scaled_amount <= scaled_before);
        }
    }
}

#[rule]
fn spoke_overrides_asset_params(e: Env, asset: Address) {
    let category_id = crate::spec::fixture::SPOKE_ID;
    crate::spec::fixture::seed_market(&e, &asset);

    let spoke = crate::storage::get_spoke(&e, category_id);
    // Active-spoke branch only.
    cvlr_assume!(!spoke.is_deprecated);

    let hub_asset = hub0(&asset);
    let spoke_asset = crate::storage::get_spoke_asset(&e, category_id, &hub_asset);
    cvlr_assume!(spoke_asset.is_some());
    let cfg = spoke_asset.unwrap();

    // Listed config projected to `AssetConfig`.
    let mut cache = crate::context::Cache::new(&e);
    let asset_config: common::types::AssetConfig =
        (&cache.require_spoke_asset(category_id, &hub_asset)).into();

    cvlr_assert!(asset_config.loan_to_value.raw() == i128::from(cfg.loan_to_value));
    cvlr_assert!(asset_config.liquidation_threshold.raw() == i128::from(cfg.liquidation_threshold));
    cvlr_assert!(asset_config.liquidation_bonus.raw() == i128::from(cfg.liquidation_bonus));

    cvlr_assert!(asset_config.is_collateralizable == cfg.is_collateralizable);
    cvlr_assert!(asset_config.is_borrowable == cfg.is_borrowable);
}

/// `add_asset_to_spoke` persists only assets with threshold > LTV.
#[rule]
fn add_asset_enforces_valid_bounds(e: Env, asset: Address, ltv: u32, threshold: u32, bonus: u32) {
    let category_id = crate::spec::fixture::SPOKE_ID;
    crate::spec::fixture::seed_protocol(&e);

    crate::config::add_asset_to_spoke(
        &e,
        &SpokeAssetArgs {
            hub_id: crate::spec::fixture::HUB_ID,
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
        },
    );

    let cfg = crate::storage::get_spoke_asset(&e, category_id, &hub0(&asset)).unwrap();
    cvlr_assert!(cfg.liquidation_threshold > cfg.loan_to_value);
}

#[rule]
fn edit_asset_enforces_valid_bounds(e: Env, asset: Address, ltv: u32, threshold: u32, bonus: u32) {
    let category_id = crate::spec::fixture::SPOKE_ID;
    crate::spec::fixture::seed_market(&e, &asset);

    crate::config::edit_asset_in_spoke(
        &e,
        &SpokeAssetArgs {
            hub_id: crate::spec::fixture::HUB_ID,
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
        },
    );

    let cfg = crate::storage::get_spoke_asset(&e, category_id, &hub0(&asset)).unwrap();
    cvlr_assert!(cfg.liquidation_threshold > cfg.loan_to_value);
}

#[rule]
fn spoke_remove_category(e: Env) {
    let category_id = crate::spec::fixture::SPOKE_ID;
    crate::spec::fixture::seed_protocol(&e);

    // Spoke must exist and be active for `remove_spoke` to run.
    let before = crate::storage::try_get_spoke(&e, category_id);
    cvlr_assume!(matches!(&before, Some(spoke) if !spoke.is_deprecated));

    crate::config::remove_spoke(&e, category_id);

    let spoke = crate::storage::get_spoke(&e, category_id);
    cvlr_assert!(spoke.is_deprecated);
}

/// Adding an asset to a deprecated spoke must revert.
#[rule]
fn spoke_add_asset_to_deprecated_category(e: Env, asset: Address) {
    let category_id = crate::spec::fixture::SPOKE_ID;
    crate::spec::fixture::seed_protocol(&e);
    let mut deprecated = crate::storage::get_spoke(&e, category_id);
    deprecated.is_deprecated = true;
    crate::storage::set_spoke(&e, category_id, &deprecated);

    let spoke = crate::storage::try_get_spoke(&e, category_id);
    cvlr_assume!(spoke.is_some());
    cvlr_assume!(spoke.unwrap().is_deprecated);

    crate::config::add_asset_to_spoke(
        &e,
        &SpokeAssetArgs {
            hub_id: crate::spec::fixture::HUB_ID,
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
        },
    );

    cvlr_assert!(false);
}

#[rule]
fn spoke_supply_sanity(e: Env, caller: Address, asset: Address) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let amount = crate::constants::WAD;
    let spoke_id = crate::spec::fixture::SPOKE_ID;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    assets.push_back((hub0(&asset), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, spoke_id, &assets);
    cvlr_satisfy!(true);
}

#[rule]
fn spoke_borrow_sanity(e: Env, caller: Address, asset: Address) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let amount = crate::constants::WAD;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::compat::supply_single(
        e.clone(),
        caller.clone(),
        account_id,
        asset.clone(),
        amount * 4,
    );

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let mut borrows: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    borrows.push_back((hub0(&asset), amount));
    crate::positions::borrow::process_borrow(&e, &caller, account_id, &borrows, None);
    cvlr_satisfy!(true);
}

#[rule]
fn deprecated_spoke_withdraw_sanity(e: Env, caller: Address, asset: Address) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let amount = crate::constants::WAD;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::compat::supply_single(
        e.clone(),
        caller.clone(),
        account_id,
        asset.clone(),
        amount * 2,
    );
    let mut deprecated = crate::storage::get_spoke(&e, crate::spec::fixture::SPOKE_ID);
    deprecated.is_deprecated = true;
    crate::storage::set_spoke(&e, crate::spec::fixture::SPOKE_ID, &deprecated);
    crate::spec::compat::withdraw_single(e, caller, account_id, asset, amount);
    cvlr_satisfy!(true);
}
