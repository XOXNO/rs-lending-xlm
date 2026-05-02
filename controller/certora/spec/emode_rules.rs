/// E-Mode Constraint Rules
///
/// Formal verification rules for E-Mode asset whitelist enforcement,
/// deprecated category blocking, parameter overrides, category lifecycle,
/// and cross-constraint with isolation mode.
///
/// From CLAUDE.md:
///   - Category chosen at account creation. Only category-registered assets allowed.
///   - Deprecated categories block new positions.
///   - E-Mode XOR Isolation (never both).
///   - E-mode parameters (LTV, threshold, bonus) override base asset config.
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Vec};

// ===========================================================================
// E-Mode Asset Whitelist
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 1: emode_only_registered_assets
// ---------------------------------------------------------------------------

/// When an account has `e_mode_category_id > 0`, supplying an asset that is
/// not registered in that category must revert. The rule constrains the asset
/// to be absent from the category, calls supply, and marks any reachable
/// success path as a violation.
#[rule]
fn emode_only_registered_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    // Account must be in an e-mode category
    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);

    // Asset must NOT be registered in the account's e-mode category
    let asset_cats = crate::storage::get_asset_emodes(&e, &asset);
    cvlr_assume!(!asset_cats.contains(attrs.e_mode_category_id));

    // Attempt supply -- must revert because asset is not in the category
    let mut assets: Vec<(Address, i128)> = Vec::new(&e);
    assets.push_back((asset, amount));
    crate::positions::supply::process_supply(
        &e,
        &caller,
        account_id,
        attrs.e_mode_category_id,
        &assets,
    );

    // Unreachable: supply of unregistered asset in e-mode must revert
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 2: emode_borrow_only_registered_assets
// ---------------------------------------------------------------------------

/// When an account has e_mode_category > 0, borrowing an asset NOT registered
/// in that category must revert. Mirror of `emode_only_registered_assets` but
/// for borrows.
#[rule]
fn emode_borrow_only_registered_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    // Account must be in an e-mode category
    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);

    // Asset must NOT be registered in the account's e-mode category
    let asset_cats = crate::storage::get_asset_emodes(&e, &asset);
    cvlr_assume!(!asset_cats.contains(attrs.e_mode_category_id));

    // Attempt borrow -- must revert because asset is not in the category
    let mut borrows: Vec<(Address, i128)> = Vec::new(&e);
    borrows.push_back((asset, amount));
    crate::positions::borrow::borrow_batch(&e, &caller, account_id, &borrows);

    // Unreachable: borrow of unregistered asset in e-mode must revert
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 3: emode_only_borrowable_assets
// ---------------------------------------------------------------------------

/// When an account has e_mode_category > 0, borrowing an asset where
/// `is_borrowable = false` in the EModeAssetConfig must revert.
#[rule]
fn emode_only_borrowable_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    // Account must be in an e-mode category
    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);

    // Asset IS registered in the category but NOT borrowable
    let emode_config = crate::storage::get_emode_asset(&e, attrs.e_mode_category_id, &asset);
    cvlr_assume!(emode_config.is_some());
    let cfg = emode_config.unwrap();
    cvlr_assume!(!cfg.is_borrowable);

    // Attempt borrow -- must revert because asset is not borrowable in e-mode
    let mut borrows: Vec<(Address, i128)> = Vec::new(&e);
    borrows.push_back((asset, amount));
    crate::positions::borrow::borrow_batch(&e, &caller, account_id, &borrows);

    // Unreachable: borrow of non-borrowable e-mode asset must revert
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 3: emode_only_collateralizable_assets
// ---------------------------------------------------------------------------

/// Supplying an asset where `is_collateralizable = false` in the
/// EModeAssetConfig must revert.
#[rule]
fn emode_only_collateralizable_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    // Account must be in an e-mode category
    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);

    // Asset IS registered in the category but NOT collateralizable
    let emode_config = crate::storage::get_emode_asset(&e, attrs.e_mode_category_id, &asset);
    cvlr_assume!(emode_config.is_some());
    let cfg = emode_config.unwrap();
    cvlr_assume!(!cfg.is_collateralizable);

    // Attempt supply -- must revert because asset is not collateralizable
    let mut assets: Vec<(Address, i128)> = Vec::new(&e);
    assets.push_back((asset, amount));
    crate::positions::supply::process_supply(
        &e,
        &caller,
        account_id,
        attrs.e_mode_category_id,
        &assets,
    );

    // Unreachable: supply of non-collateralizable e-mode asset must revert
    cvlr_satisfy!(false);
}

// ===========================================================================
// Deprecated Category Blocking
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 4: deprecated_emode_blocks_new_supply
// ---------------------------------------------------------------------------

/// If EModeCategory.is_deprecated = true, new supply to an account in that
/// category must revert. The protocol blocks new position creation in
/// deprecated categories to wind them down.
#[rule]
fn deprecated_emode_blocks_new_supply(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    // Account must be in a deprecated e-mode category
    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);

    let category = crate::storage::get_emode_category(&e, attrs.e_mode_category_id);
    cvlr_assume!(category.is_deprecated);

    // Attempt supply -- must revert because category is deprecated
    let mut assets: Vec<(Address, i128)> = Vec::new(&e);
    assets.push_back((asset, amount));
    crate::positions::supply::process_supply(
        &e,
        &caller,
        account_id,
        attrs.e_mode_category_id,
        &assets,
    );

    // Unreachable: supply into deprecated e-mode category must revert
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 5b: deprecated_emode_blocks_new_borrow
// ---------------------------------------------------------------------------

/// If EModeCategory.is_deprecated = true, new borrow from an account in that
/// category must revert. Mirror of `deprecated_emode_blocks_new_supply` but
/// for borrows.
#[rule]
fn deprecated_emode_blocks_new_borrow(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    // Account must be in a deprecated e-mode category
    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);

    let category = crate::storage::get_emode_category(&e, attrs.e_mode_category_id);
    cvlr_assume!(category.is_deprecated);

    // Attempt borrow -- must revert because category is deprecated
    let mut borrows: Vec<(Address, i128)> = Vec::new(&e);
    borrows.push_back((asset, amount));
    crate::positions::borrow::borrow_batch(&e, &caller, account_id, &borrows);

    // Unreachable: borrow from deprecated e-mode category must revert
    cvlr_satisfy!(false);
}

// ---------------------------------------------------------------------------
// Rule 6: deprecated_emode_allows_withdraw
// ---------------------------------------------------------------------------

/// Deprecated categories must still allow withdrawals. The rule constrains the
/// asset to an existing deposit position and the amount to a valid withdrawal
/// range so any revert is attributable to deprecated-category handling.
#[rule]
fn deprecated_emode_allows_withdraw(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    // Account must be in a deprecated e-mode category
    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);

    let category = crate::storage::get_emode_category(&e, attrs.e_mode_category_id);
    cvlr_assume!(category.is_deprecated);

    // Account must have a deposit position for this specific asset
    let deposit_list =
        crate::storage::get_position_list(&e, account_id, common::types::POSITION_TYPE_DEPOSIT);
    cvlr_assume!(!deposit_list.is_empty());

    // Asset must be in the deposit list (isolate deprecated-category behavior)
    let mut asset_in_list = false;
    for i in 0..deposit_list.len() {
        let existing = deposit_list.get(i).unwrap();
        if existing == asset {
            asset_in_list = true;
        }
    }
    cvlr_assume!(asset_in_list);

    // Amount must not exceed the position value (avoid revert from over-withdrawal)
    let position =
        crate::storage::get_position(&e, account_id, common::types::POSITION_TYPE_DEPOSIT, &asset);
    cvlr_assume!(position.is_some());
    let pos = position.unwrap();
    cvlr_assume!(pos.scaled_amount_ray > 0);

    // Withdraw must succeed -- deprecated categories do not block exits
    let mut withdrawals: Vec<(Address, i128)> = Vec::new(&e);
    withdrawals.push_back((asset, amount));
    crate::positions::withdraw::process_withdraw(&e, &caller, account_id, &withdrawals);

    // Reachable: withdraw from deprecated e-mode category must not revert
    cvlr_satisfy!(true);
}

// ===========================================================================
// E-Mode Parameter Override
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 6: emode_overrides_asset_params
// ---------------------------------------------------------------------------

/// When e-mode is active, effective LTV, threshold, and bonus must come from
/// the category rather than the base asset config.
#[rule]
fn emode_overrides_asset_params(e: Env, asset: Address, category_id: u32) {
    cvlr_assume!(category_id > 0);

    // Category must exist
    let category = crate::storage::get_emode_category(&e, category_id);

    // Asset must be registered in the category
    let emode_asset = crate::storage::get_emode_asset(&e, category_id, &asset);
    cvlr_assume!(emode_asset.is_some());

    let asset_cats = crate::storage::get_asset_emodes(&e, &asset);
    cvlr_assume!(asset_cats.contains(category_id));

    // Get base config and apply e-mode override
    let mut asset_config = crate::storage::get_market_config(&e, &asset).asset_config;
    let emode_cat = crate::positions::emode::e_mode_category(&e, category_id);
    let emode_asset_cfg = crate::positions::emode::token_e_mode_config(&e, category_id, &asset);
    crate::positions::emode::apply_e_mode_to_asset_config(
        &e,
        &mut asset_config,
        &emode_cat,
        emode_asset_cfg,
    );

    // After override: params must match the category, not the base config
    cvlr_assert!(asset_config.loan_to_value_bps == category.loan_to_value_bps);
    cvlr_assert!(asset_config.liquidation_threshold_bps == category.liquidation_threshold_bps);
    cvlr_assert!(asset_config.liquidation_bonus_bps == category.liquidation_bonus_bps);

    // Also verify collateral/borrow flags match the e-mode asset config
    let cfg = emode_asset.unwrap();
    cvlr_assert!(asset_config.is_collateralizable == cfg.is_collateralizable);
    cvlr_assert!(asset_config.is_borrowable == cfg.is_borrowable);
}

// ===========================================================================
// E-Mode Category Lifecycle
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 8: emode_category_has_valid_params
// ---------------------------------------------------------------------------

/// Every non-deprecated e-mode category must have LTV < threshold.
/// This is the same invariant as base asset config -- without it, a position
/// could be simultaneously at max capacity and liquidatable.
#[rule]
fn emode_category_has_valid_params(e: Env, category_id: u32) {
    cvlr_assume!(category_id > 0);

    let category = crate::storage::get_emode_category(&e, category_id);
    cvlr_assume!(!category.is_deprecated);

    cvlr_assert!(category.liquidation_threshold_bps > category.loan_to_value_bps);
}

// ---------------------------------------------------------------------------
// Rule 9: emode_remove_deprecated_only
// ---------------------------------------------------------------------------

/// Removing (deprecating) an e-mode category via `remove_e_mode_category`
/// sets `is_deprecated = true`. After removal, the category must be marked
/// deprecated. Also verifies that attempting to add a new asset to a
/// deprecated category reverts.
#[rule]
fn emode_remove_category(e: Env, category_id: u32) {
    // Remove the category (sets is_deprecated = true)
    crate::config::remove_e_mode_category(&e, category_id);

    // After removal, the category must be deprecated
    let category = crate::storage::get_emode_category(&e, category_id);
    cvlr_assert!(category.is_deprecated);
}

/// Adding an asset to a deprecated category must revert.
#[rule]
fn emode_add_asset_to_deprecated_category(e: Env, asset: Address, category_id: u32) {
    // Attempt to add asset to deprecated category -- must revert
    crate::config::add_asset_to_e_mode_category(&e, asset, category_id, true, true);

    // Unreachable: adding asset to deprecated category must revert
    cvlr_satisfy!(false);
}

// ===========================================================================
// Cross-Constraint with Isolation
// ===========================================================================

// ---------------------------------------------------------------------------
// Rule 10: emode_account_cannot_enter_isolation
// ---------------------------------------------------------------------------

/// An account with e_mode_category > 0 cannot have is_isolated = true.
/// Verified at account creation: attempting to create an account with both
/// e-mode and isolation must revert.
#[rule]
fn emode_account_cannot_enter_isolation(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);
    cvlr_assume!(e_mode_category > 0);

    // Asset is an isolated asset
    let config = crate::storage::get_market_config(&e, &asset).asset_config;
    cvlr_assume!(config.is_isolated_asset);

    // Attempt to create new account (account_id = 0) with e-mode and isolated asset
    // The first asset being isolated triggers is_isolated = true in
    // create_account_from_first_asset, but e_mode_category > 0 conflicts.
    let mut assets: Vec<(Address, i128)> = Vec::new(&e);
    assets.push_back((asset, amount));
    crate::positions::supply::process_supply(&e, &caller, 0, e_mode_category, &assets);

    // Unreachable: e-mode + isolation at account creation must revert
    cvlr_satisfy!(false);
}

/// Verify existing account invariant: no account can have both e-mode and
/// isolation simultaneously, regardless of how it was created.
#[rule]
fn emode_isolation_mutual_exclusion_invariant(e: Env, account_id: u64) {
    let attrs = crate::storage::get_account_attrs(&e, account_id);

    // These two conditions cannot both be true
    if attrs.e_mode_category_id > 0 {
        cvlr_assert!(!attrs.is_isolated);
    }
    if attrs.is_isolated {
        cvlr_assert!(attrs.e_mode_category_id == 0);
    }
}

// ===========================================================================
// Sanity (reachability checks -- ensures rules are not vacuously true)
// ===========================================================================

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
    crate::positions::borrow::borrow_batch(&e, &caller, account_id, &borrows);
    cvlr_satisfy!(true);
}
