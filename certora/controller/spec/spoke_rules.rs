use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Vec};

use crate::types::{AccountPositionType, HubAssetKey, SpokeAssetArgs};

fn hub0(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: crate::spec::fixture::HUB_ID,
        asset: asset.clone(),
    }
}

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
    crate::positions::process_borrow(&e, &caller, account_id, &borrows, None);

    cvlr_assert!(false);
}

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
    crate::positions::process_borrow(&e, &caller, account_id, &borrows, None);

    cvlr_assert!(false);
}

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
    crate::positions::process_borrow(&e, &caller, account_id, &borrows, None);

    cvlr_assert!(false);
}

#[rule]
fn deprecated_spoke_withdraw_does_not_increase_supply(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
    scaled_before: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(scaled_before > 0 && scaled_before <= 20 * common::constants::RAY);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::fixture::seed_supply_position(&e, account_id, &asset, scaled_before);
    let mut deprecated = crate::storage::get_spoke(&e, crate::spec::fixture::SPOKE_ID);
    deprecated.is_deprecated = true;
    crate::storage::set_spoke(&e, crate::spec::fixture::SPOKE_ID, &deprecated);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let spoke = crate::storage::get_spoke(&e, attrs.spoke_id);
    cvlr_assume!(spoke.is_deprecated);

    let mut withdrawals: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    withdrawals.push_back((hub0(&asset), amount));
    crate::positions::process_withdraw(&e, &caller, account_id, &withdrawals, None);

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

    cvlr_assume!(!spoke.is_deprecated);

    let hub_asset = hub0(&asset);
    let spoke_asset = crate::storage::get_spoke_asset(&e, category_id, &hub_asset);
    cvlr_assume!(spoke_asset.is_some());
    let cfg = spoke_asset.unwrap();

    let mut cache = crate::context::Cache::new(&e);
    let asset_config: common::types::AssetConfig =
        cache.require_spoke_asset(category_id, &hub_asset);

    cvlr_assert!(asset_config.loan_to_value.raw() == i128::from(cfg.loan_to_value));
    cvlr_assert!(asset_config.liquidation_threshold.raw() == i128::from(cfg.liquidation_threshold));
    cvlr_assert!(asset_config.liquidation_bonus.raw() == i128::from(cfg.liquidation_bonus));

    cvlr_assert!(asset_config.is_collateralizable == cfg.is_collateralizable);
    cvlr_assert!(asset_config.is_borrowable == cfg.is_borrowable);
}

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
            supply_cap: crate::spec::fixture::UNCONSTRAINED_CAP,
            borrow_cap: crate::spec::fixture::UNCONSTRAINED_CAP,
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
            supply_cap: crate::spec::fixture::UNCONSTRAINED_CAP,
            borrow_cap: crate::spec::fixture::UNCONSTRAINED_CAP,
        },
    );

    let cfg = crate::storage::get_spoke_asset(&e, category_id, &hub0(&asset)).unwrap();
    cvlr_assert!(cfg.liquidation_threshold > cfg.loan_to_value);
}

#[rule]
fn spoke_remove_category(e: Env) {
    let category_id = crate::spec::fixture::SPOKE_ID;
    crate::spec::fixture::seed_protocol(&e);

    let before = crate::storage::try_get_spoke(&e, category_id);
    cvlr_assume!(matches!(&before, Some(spoke) if !spoke.is_deprecated));

    crate::config::remove_spoke(&e, category_id);

    let spoke = crate::storage::get_spoke(&e, category_id);
    cvlr_assert!(spoke.is_deprecated);
}

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
            supply_cap: crate::spec::fixture::UNCONSTRAINED_CAP,
            borrow_cap: crate::spec::fixture::UNCONSTRAINED_CAP,
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
    crate::positions::process_borrow(&e, &caller, account_id, &borrows, None);
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

// ---------------------------------------------------------------------------
// Bulk position-limit proofs.
//
// `validate_bulk_position_limits` (risk/validation.rs:50) de-duplicates
// repeated assets *within one call* (`seen` map) before comparing the new
// unique-position count against the configured limits. These rules pin that
// the duplicated-leg bulk flow at the exact boundary succeeds (supply) or
// reverts only when a *second distinct* leg would exceed the cap (supply and
// borrow), and that a fresh multi-leg supply persists both records.
// ---------------------------------------------------------------------------

#[rule]
fn bulk_supply_duplicate_asset_counted_once(
    e: Env,
    caller: Address,
    account_id: u64,
    asset_a: Address,
    s1: Address,
    s2: Address,
    s3: Address,
    s4: Address,
    s5: Address,
    s6: Address,
    s7: Address,
    s8: Address,
    s9: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &caller);
    crate::spec::fixture::seed_market(&e, &asset_a);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let assets = [s1, s2, s3, s4, s5, s6, s7, s8, s9];
    cvlr_assume!(
        assets[0] != assets[1]
            && assets[0] != assets[2]
            && assets[0] != assets[3]
            && assets[0] != assets[4]
            && assets[0] != assets[5]
            && assets[0] != assets[6]
            && assets[0] != assets[7]
            && assets[0] != assets[8]
    );
    cvlr_assume!(
        assets[1] != assets[2]
            && assets[1] != assets[3]
            && assets[1] != assets[4]
            && assets[1] != assets[5]
            && assets[1] != assets[6]
            && assets[1] != assets[7]
            && assets[1] != assets[8]
    );
    cvlr_assume!(
        assets[2] != assets[3]
            && assets[2] != assets[4]
            && assets[2] != assets[5]
            && assets[2] != assets[6]
            && assets[2] != assets[7]
            && assets[2] != assets[8]
    );
    cvlr_assume!(
        assets[3] != assets[4]
            && assets[3] != assets[5]
            && assets[3] != assets[6]
            && assets[3] != assets[7]
            && assets[3] != assets[8]
    );
    cvlr_assume!(
        assets[4] != assets[5]
            && assets[4] != assets[6]
            && assets[4] != assets[7]
            && assets[4] != assets[8]
    );
    cvlr_assume!(assets[5] != assets[6] && assets[5] != assets[7] && assets[5] != assets[8]);
    cvlr_assume!(assets[6] != assets[7] && assets[6] != assets[8]);
    cvlr_assume!(assets[7] != assets[8]);
    cvlr_assume!(
        asset_a != assets[0]
            && asset_a != assets[1]
            && asset_a != assets[2]
            && asset_a != assets[3]
            && asset_a != assets[4]
            && asset_a != assets[5]
            && asset_a != assets[6]
            && asset_a != assets[7]
            && asset_a != assets[8]
    );

    let seeded = crate::spec::fixture::seed_supply_positions(&e, account_id, &assets);
    cvlr_assume!(seeded == 9);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((crate::spec::fixture::hub_asset(&asset_a), amount));
    legs.push_back((crate::spec::fixture::hub_asset(&asset_a), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &legs);

    // The duplicated leg is counted once for the limit check: at 9+1 unique
    // positions the call must reach the boundary without reverting, and the
    // new asset's record must be persisted.
    let book = crate::storage::get_supply_positions(&e, account_id);
    cvlr_assert!(book
        .get(crate::spec::fixture::hub_asset(&asset_a))
        .is_some());
    cvlr_assert!(book.len() == 10);
}

#[rule]
fn bulk_supply_distinct_legs_exceed_limit_reverts(
    e: Env,
    caller: Address,
    account_id: u64,
    asset_a: Address,
    asset_b: Address,
    s1: Address,
    s2: Address,
    s3: Address,
    s4: Address,
    s5: Address,
    s6: Address,
    s7: Address,
    s8: Address,
    s9: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(asset_a != asset_b);
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &caller);
    crate::spec::fixture::seed_market(&e, &asset_a);
    crate::spec::fixture::seed_market(&e, &asset_b);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let assets = [s1, s2, s3, s4, s5, s6, s7, s8, s9];
    cvlr_assume!(
        assets[0] != assets[1]
            && assets[0] != assets[2]
            && assets[0] != assets[3]
            && assets[0] != assets[4]
            && assets[0] != assets[5]
            && assets[0] != assets[6]
            && assets[0] != assets[7]
            && assets[0] != assets[8]
    );
    cvlr_assume!(
        assets[1] != assets[2]
            && assets[1] != assets[3]
            && assets[1] != assets[4]
            && assets[1] != assets[5]
            && assets[1] != assets[6]
            && assets[1] != assets[7]
            && assets[1] != assets[8]
    );
    cvlr_assume!(
        assets[2] != assets[3]
            && assets[2] != assets[4]
            && assets[2] != assets[5]
            && assets[2] != assets[6]
            && assets[2] != assets[7]
            && assets[2] != assets[8]
    );
    cvlr_assume!(
        assets[3] != assets[4]
            && assets[3] != assets[5]
            && assets[3] != assets[6]
            && assets[3] != assets[7]
            && assets[3] != assets[8]
    );
    cvlr_assume!(
        assets[4] != assets[5]
            && assets[4] != assets[6]
            && assets[4] != assets[7]
            && assets[4] != assets[8]
    );
    cvlr_assume!(assets[5] != assets[6] && assets[5] != assets[7] && assets[5] != assets[8]);
    cvlr_assume!(assets[6] != assets[7] && assets[6] != assets[8]);
    cvlr_assume!(assets[7] != assets[8]);
    cvlr_assume!(
        asset_a != assets[0]
            && asset_a != assets[1]
            && asset_a != assets[2]
            && asset_a != assets[3]
            && asset_a != assets[4]
            && asset_a != assets[5]
            && asset_a != assets[6]
            && asset_a != assets[7]
            && asset_a != assets[8]
    );
    cvlr_assume!(
        asset_b != assets[0]
            && asset_b != assets[1]
            && asset_b != assets[2]
            && asset_b != assets[3]
            && asset_b != assets[4]
            && asset_b != assets[5]
            && asset_b != assets[6]
            && asset_b != assets[7]
            && asset_b != assets[8]
    );

    let seeded = crate::spec::fixture::seed_supply_positions(&e, account_id, &assets);
    cvlr_assume!(seeded == 9);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((crate::spec::fixture::hub_asset(&asset_a), amount));
    legs.push_back((crate::spec::fixture::hub_asset(&asset_b), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &legs);

    // Two distinct new assets would bring the account to 11 > POSITION_LIMIT_MAX.
    cvlr_assert!(false);
}

#[rule]
fn bulk_supply_two_assets_both_persisted(
    e: Env,
    caller: Address,
    account_id: u64,
    asset_a: Address,
    asset_b: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(asset_a != asset_b);
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &caller);
    crate::spec::fixture::seed_market(&e, &asset_a);
    crate::spec::fixture::seed_market(&e, &asset_b);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((crate::spec::fixture::hub_asset(&asset_a), amount));
    legs.push_back((crate::spec::fixture::hub_asset(&asset_b), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &legs);

    let book = crate::storage::get_supply_positions(&e, account_id);
    cvlr_assert!(book.len() == 2);
    cvlr_assert!(book
        .get(crate::spec::fixture::hub_asset(&asset_a))
        .is_some());
    cvlr_assert!(book
        .get(crate::spec::fixture::hub_asset(&asset_b))
        .is_some());
}

#[rule]
fn bulk_borrow_distinct_legs_exceed_limit_reverts(
    e: Env,
    caller: Address,
    account_id: u64,
    asset_a: Address,
    asset_b: Address,
    s1: Address,
    s2: Address,
    s3: Address,
    s4: Address,
    s5: Address,
    s6: Address,
    s7: Address,
    s8: Address,
    s9: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(asset_a != asset_b);
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &caller);
    crate::spec::fixture::seed_market(&e, &asset_a);
    crate::spec::fixture::seed_market(&e, &asset_b);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let assets = [s1, s2, s3, s4, s5, s6, s7, s8, s9];
    cvlr_assume!(
        assets[0] != assets[1]
            && assets[0] != assets[2]
            && assets[0] != assets[3]
            && assets[0] != assets[4]
            && assets[0] != assets[5]
            && assets[0] != assets[6]
            && assets[0] != assets[7]
            && assets[0] != assets[8]
    );
    cvlr_assume!(
        assets[1] != assets[2]
            && assets[1] != assets[3]
            && assets[1] != assets[4]
            && assets[1] != assets[5]
            && assets[1] != assets[6]
            && assets[1] != assets[7]
            && assets[1] != assets[8]
    );
    cvlr_assume!(
        assets[2] != assets[3]
            && assets[2] != assets[4]
            && assets[2] != assets[5]
            && assets[2] != assets[6]
            && assets[2] != assets[7]
            && assets[2] != assets[8]
    );
    cvlr_assume!(
        assets[3] != assets[4]
            && assets[3] != assets[5]
            && assets[3] != assets[6]
            && assets[3] != assets[7]
            && assets[3] != assets[8]
    );
    cvlr_assume!(
        assets[4] != assets[5]
            && assets[4] != assets[6]
            && assets[4] != assets[7]
            && assets[4] != assets[8]
    );
    cvlr_assume!(assets[5] != assets[6] && assets[5] != assets[7] && assets[5] != assets[8]);
    cvlr_assume!(assets[6] != assets[7] && assets[6] != assets[8]);
    cvlr_assume!(assets[7] != assets[8]);
    cvlr_assume!(
        asset_a != assets[0]
            && asset_a != assets[1]
            && asset_a != assets[2]
            && asset_a != assets[3]
            && asset_a != assets[4]
            && asset_a != assets[5]
            && asset_a != assets[6]
            && asset_a != assets[7]
            && asset_a != assets[8]
    );
    cvlr_assume!(
        asset_b != assets[0]
            && asset_b != assets[1]
            && asset_b != assets[2]
            && asset_b != assets[3]
            && asset_b != assets[4]
            && asset_b != assets[5]
            && asset_b != assets[6]
            && asset_b != assets[7]
            && asset_b != assets[8]
    );

    let seeded = crate::spec::fixture::seed_debt_positions(&e, account_id, &assets);
    cvlr_assume!(seeded == 9);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((crate::spec::fixture::hub_asset(&asset_a), amount));
    legs.push_back((crate::spec::fixture::hub_asset(&asset_b), amount));
    // The borrow position-limit gate runs inside validate_position_entry_gates
    // before any health computation: 9 + 2 distinct -> PositionLimitExceeded.
    crate::positions::process_borrow(&e, &caller, account_id, &legs, None);

    cvlr_assert!(false);
}

#[rule]
fn bulk_borrow_duplicate_leg_not_double_counted(
    e: Env,
    caller: Address,
    account_id: u64,
    asset_a: Address,
    collateral: Address,
    s1: Address,
    s2: Address,
    s3: Address,
    s4: Address,
    s5: Address,
    s6: Address,
    s7: Address,
    s8: Address,
    s9: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(asset_a != collateral);
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &caller);
    crate::spec::fixture::seed_market(&e, &asset_a);
    crate::spec::fixture::seed_market(&e, &collateral);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let assets = [s1, s2, s3, s4, s5, s6, s7, s8, s9];
    cvlr_assume!(
        assets[0] != assets[1]
            && assets[0] != assets[2]
            && assets[0] != assets[3]
            && assets[0] != assets[4]
            && assets[0] != assets[5]
            && assets[0] != assets[6]
            && assets[0] != assets[7]
            && assets[0] != assets[8]
    );
    cvlr_assume!(
        assets[1] != assets[2]
            && assets[1] != assets[3]
            && assets[1] != assets[4]
            && assets[1] != assets[5]
            && assets[1] != assets[6]
            && assets[1] != assets[7]
            && assets[1] != assets[8]
    );
    cvlr_assume!(
        assets[2] != assets[3]
            && assets[2] != assets[4]
            && assets[2] != assets[5]
            && assets[2] != assets[6]
            && assets[2] != assets[7]
            && assets[2] != assets[8]
    );
    cvlr_assume!(
        assets[3] != assets[4]
            && assets[3] != assets[5]
            && assets[3] != assets[6]
            && assets[3] != assets[7]
            && assets[3] != assets[8]
    );
    cvlr_assume!(
        assets[4] != assets[5]
            && assets[4] != assets[6]
            && assets[4] != assets[7]
            && assets[4] != assets[8]
    );
    cvlr_assume!(assets[5] != assets[6] && assets[5] != assets[7] && assets[5] != assets[8]);
    cvlr_assume!(assets[6] != assets[7] && assets[6] != assets[8]);
    cvlr_assume!(assets[7] != assets[8]);
    cvlr_assume!(
        asset_a != assets[0]
            && asset_a != assets[1]
            && asset_a != assets[2]
            && asset_a != assets[3]
            && asset_a != assets[4]
            && asset_a != assets[5]
            && asset_a != assets[6]
            && asset_a != assets[7]
            && asset_a != assets[8]
    );
    cvlr_assume!(
        collateral != assets[0]
            && collateral != assets[1]
            && collateral != assets[2]
            && collateral != assets[3]
            && collateral != assets[4]
            && collateral != assets[5]
            && collateral != assets[6]
            && collateral != assets[7]
            && collateral != assets[8]
    );

    // Positive collateral book so a borrow path can pass the health gate.
    crate::spec::fixture::seed_supply_position(&e, account_id, &collateral, common::constants::RAY);
    let seeded = crate::spec::fixture::seed_debt_positions(&e, account_id, &assets);
    cvlr_assume!(seeded == 9);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((crate::spec::fixture::hub_asset(&asset_a), amount));
    legs.push_back((crate::spec::fixture::hub_asset(&asset_a), amount));
    crate::positions::process_borrow(&e, &caller, account_id, &legs, None);

    // A duplicated leg counts once: 9 + 1 unique positions stays within the
    // cap, so a borrow of the new asset is reachable.
    cvlr_satisfy!(crate::storage::get_debt_positions(&e, account_id)
        .get(crate::spec::fixture::hub_asset(&asset_a))
        .is_some());
}
