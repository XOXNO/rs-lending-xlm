/// E-Mode Constraint Rules
///
/// Formal verification rules for E-Mode asset whitelist enforcement,
/// deprecated category blocking, parameter overrides, category lifecycle,
/// and cross-constraint with isolation mode.
///
/// Rules cover category selection, member-asset validation, deprecated-category
/// restrictions, isolation-mode exclusion, and E-mode risk-parameter overrides.
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

/// Deprecated categories must still allow withdrawals. The rule asserts that
/// the position changes: either the scaled amount decreases or the position is
/// fully closed.
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

    // Account must have a deposit position for this specific asset.
    let position =
        crate::storage::get_position(&e, account_id, common::types::POSITION_TYPE_DEPOSIT, &asset);
    cvlr_assume!(position.is_some());
    let pos_before = position.unwrap();
    cvlr_assume!(pos_before.scaled_amount_ray > 0);
    let scaled_before = pos_before.scaled_amount_ray;

    // Withdraw must succeed -- deprecated categories do not block exits
    let mut withdrawals: Vec<(Address, i128)> = Vec::new(&e);
    withdrawals.push_back((asset.clone(), amount));
    crate::positions::withdraw::process_withdraw(&e, &caller, account_id, &withdrawals);

    // Post-state must show the withdraw actually happened: either the
    // position is gone (full withdraw) or its scaled amount strictly
    // decreased (partial withdraw).
    let position_after =
        crate::storage::get_position(&e, account_id, common::types::POSITION_TYPE_DEPOSIT, &asset);
    match position_after {
        None => {
            cvlr_assert!(true);
        }
        Some(pos_after) => {
            cvlr_assert!(pos_after.scaled_amount_ray < scaled_before);
        }
    }
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

    // `apply_e_mode_to_asset_config` early-returns on deprecated categories
    // (`controller/src/positions/emode.rs:20-29`) and leaves the base config
    // untouched. Restrict the rule to the override branch.
    cvlr_assume!(!category.is_deprecated);

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
/// sets `is_deprecated = true`, walks the side map to clear each member's
/// reverse index entry, drops the entire `EModeAssets(category_id)` ledger
/// entry, and clears `e_mode_enabled` on orphaned markets
/// (`controller/src/config.rs:271-304`). The rule asserts:
///   1. category flagged deprecated;
///   2. side map empty after the call;
///   3. for the sampled pre-existing member, the reverse index
///      `AssetEModes(asset)` no longer contains `category_id`;
///   4. when the sampled member's reverse index becomes empty, the
///      `e_mode_enabled` flag is cleared on its market config.
#[rule]
fn emode_remove_category(e: Env, category_id: u32) {
    cvlr_assume!(category_id > 0);

    // Capture a member of the category before deprecation. Pin to the first
    // entry so the post-state assertion can target the same asset.
    let members_before = crate::storage::get_emode_assets(&e, category_id);
    cvlr_assume!(!members_before.is_empty());
    // Bound the side-map size: production semantics do not depend on map
    // size and the loop in `remove_e_mode_category` does N reads + up to
    // 2N writes per member. Without this cap the prover quantifies over
    // an unbounded `Map<Address, EModeAssetConfig>` and TAC-blows.
    cvlr_assume!(members_before.len() <= 5);
    let sample_asset = members_before.keys().get(0).unwrap();
    let cats_before = crate::storage::get_asset_emodes(&e, &sample_asset);
    cvlr_assume!(cats_before.contains(category_id));
    // Capture the pre-state `e_mode_enabled` flag and reverse-index length
    // so the post-condition can target the cleared-flag branch.
    let market_before = crate::storage::get_market_config(&e, &sample_asset);
    let was_e_mode_enabled = market_before.asset_config.e_mode_enabled;
    let cats_before_len = cats_before.len();

    // Remove the category. This sets `is_deprecated = true`, walks every
    // member of the side map to remove `category_id` from the reverse
    // index, drops `EModeAssets(category_id)` entirely, and clears
    // `e_mode_enabled` on assets whose reverse index becomes empty.
    crate::config::remove_e_mode_category(&e, category_id);

    // (1) Category is flagged deprecated.
    let category = crate::storage::get_emode_category(&e, category_id);
    cvlr_assert!(category.is_deprecated);

    // (2) Side map is absent: reads return an empty map.
    let members_after = crate::storage::get_emode_assets(&e, category_id);
    cvlr_assert!(members_after.is_empty());

    // (3) Reverse index for the sampled member no longer contains the id.
    let cats_after = crate::storage::get_asset_emodes(&e, &sample_asset);
    cvlr_assert!(!cats_after.contains(category_id));

    // (4) When the sampled member's reverse index becomes empty, the market
    // config's `e_mode_enabled` flag must be cleared.
    if cats_before_len == 1 && was_e_mode_enabled {
        let market_after = crate::storage::get_market_config(&e, &sample_asset);
        cvlr_assert!(!market_after.asset_config.e_mode_enabled);
    }
}

/// Adding an asset to a deprecated category must revert. Without the
/// existence + deprecation precondition the prover can satisfy the rule
/// trivially via the `EModeCategoryNotFound` revert path
/// (`config.rs:317-318`), which is a different gate than the
/// `EModeCategoryDeprecated` revert (`config.rs:319-320`) under test.
#[rule]
fn emode_add_asset_to_deprecated_category(e: Env, asset: Address, category_id: u32) {
    cvlr_assume!(category_id > 0);

    // Category must exist AND be deprecated, so the only reachable revert
    // path is the deprecated-category panic.
    let category = crate::storage::try_get_emode_category(&e, category_id);
    cvlr_assume!(category.is_some());
    cvlr_assume!(category.unwrap().is_deprecated);

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
/// The production gate is `ensure_e_mode_compatible_with_asset`
/// (`controller/src/positions/emode.rs:47-51`), invoked from
/// `prepare_deposit_plan` before any pool I/O. Calling it directly is
/// orders of magnitude cheaper than traversing the full `process_supply`
/// entry point and exercises the exact panic this rule is asserting.
#[rule]
fn emode_account_cannot_enter_isolation(e: Env, asset: Address, e_mode_category: u32) {
    cvlr_assume!(e_mode_category > 0);

    // Asset is an isolated asset.
    let config = crate::storage::get_market_config(&e, &asset).asset_config;
    cvlr_assume!(config.is_isolated_asset);

    // Calling the gate with `is_isolated_asset = true` and `e_mode_id > 0`
    // must panic with `EModeWithIsolated`.
    crate::positions::emode::ensure_e_mode_compatible_with_asset(&e, &config, e_mode_category);

    // Unreachable: the gate must panic.
    cvlr_satisfy!(false);
}

/// Inductive form of the mutual-exclusion invariant for the supply entry
/// point. `process_supply` only writes `AccountMeta` on the create-new
/// branch (`account_id == 0` -> `create_account_for_first_asset`); the
/// load-existing branch reads meta and never mutates `is_isolated` or
/// `e_mode_category_id`. Pinning `account_id == 0` exercises the only
/// branch where the invariant is non-trivially established and avoids
/// paying for a redundant load path.
#[rule]
fn emode_isolation_mutual_exclusion_after_supply(
    e: Env,
    caller: Address,
    account_id: u64,
    e_mode_category: u32,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);
    // Only the create-new branch writes meta. Pin to that branch so the
    // prover doesn't pay for the load-existing path that never mutates
    // `is_isolated` or `e_mode_category_id`.
    cvlr_assume!(account_id == 0);

    let mut assets: Vec<(Address, i128)> = Vec::new(&e);
    assets.push_back((asset, amount));
    let acct_id =
        crate::positions::supply::process_supply(&e, &caller, account_id, e_mode_category, &assets);

    // Post-state must respect the e-mode XOR isolation invariant.
    let attrs = crate::storage::get_account_attrs(&e, acct_id);
    cvlr_assert!(!(attrs.is_isolated && attrs.e_mode_category_id > 0));
}

/// Sibling of `emode_isolation_mutual_exclusion_after_supply` for the
/// `multiply` entry point. `process_multiply` also creates accounts via
/// the same `create_account_for_first_asset` path, so the mutual-
/// exclusion invariant must hold there too. The `compat::multiply` shim
/// havocs `account_id` internally; this rule constrains the e-mode
/// category and the collateral/debt pair via the shim's signature and
/// asserts the post-state invariant.
#[rule]
fn emode_isolation_mutual_exclusion_after_multiply(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    collateral_token: Address,
    debt_to_flash_loan: i128,
    debt_token: Address,
    mode: u32,
    steps: common::types::SwapSteps,
) {
    cvlr_assume!(debt_to_flash_loan > 0);
    // `compat::multiply` panics on `mode > 3`; constrain to a valid mode.
    cvlr_assume!(mode <= 3);

    let acct_id = crate::spec::compat::multiply(
        e.clone(),
        caller,
        e_mode_category,
        collateral_token,
        debt_to_flash_loan,
        debt_token,
        mode,
        steps,
    );

    // Post-state must respect the e-mode XOR isolation invariant.
    let attrs = crate::storage::get_account_attrs(&e, acct_id);
    cvlr_assert!(!(attrs.is_isolated && attrs.e_mode_category_id > 0));
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
