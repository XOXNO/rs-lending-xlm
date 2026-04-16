/// Isolation Mode & E-Mode Invariant Rules
///
/// From CLAUDE.md:
///   - LTV < liquidation_threshold always
///   - liquidation_bonus <= 15% (MAX_LIQUIDATION_BONUS = 1500 BPS)
///   - reserve_factor < 100%
///   - optimal_utilization > mid_utilization and < 1.0
///   - Single isolated collateral per account (no mixing)
///   - isolated_debt <= debt_ceiling enforced on every borrow
///   - Isolation and E-Mode are mutually exclusive
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use common::constants::{MAX_LIQUIDATION_BONUS, RAY};

// ---------------------------------------------------------------------------
// Rule 1: LTV < liquidation_threshold (always)
// ---------------------------------------------------------------------------

/// For every registered asset, LTV must be strictly less than the liquidation
/// threshold. Otherwise, a position could be simultaneously at max borrow
/// capacity AND eligible for liquidation — an impossible state.
#[rule]
fn ltv_less_than_liquidation_threshold(e: Env, asset: Address) {
    let config = crate::storage::asset_config::get_asset_config(&e, &asset);

    cvlr_assert!(config.loan_to_value_bps < config.liquidation_threshold_bps);
}

// ---------------------------------------------------------------------------
// Rule 2: Liquidation bonus <= 15% (1500 BPS)
// ---------------------------------------------------------------------------

#[rule]
fn liquidation_bonus_capped(e: Env, asset: Address) {
    let config = crate::storage::asset_config::get_asset_config(&e, &asset);

    cvlr_assert!(config.liquidation_bonus_bps <= MAX_LIQUIDATION_BONUS);
}

// ---------------------------------------------------------------------------
// Rule 3: Reserve factor < 100%
// ---------------------------------------------------------------------------

#[rule]
fn reserve_factor_bounded(e: Env, asset: Address) {
    let config = crate::storage::asset_config::get_asset_config(&e, &asset);

    cvlr_assert!(config.reserve_factor_bps < 10000); // < 100%
}

// ---------------------------------------------------------------------------
// Rule 4: Optimal utilization > mid utilization and < 100%
// ---------------------------------------------------------------------------

#[rule]
fn utilization_params_ordered(e: Env, asset: Address) {
    let params = crate::storage::market_params::get_market_params(&e, &asset);

    cvlr_assert!(params.mid_utilization_ray > 0);
    cvlr_assert!(params.optimal_utilization_ray > params.mid_utilization_ray);
    cvlr_assert!(params.optimal_utilization_ray < RAY); // < 100%
}

// ---------------------------------------------------------------------------
// Rule 5: Isolation and E-Mode are mutually exclusive
// ---------------------------------------------------------------------------

/// An account cannot have both an e-mode category AND be in isolation mode.
#[rule]
fn isolation_emode_exclusive(e: Env, account_id: u64) {
    let account_data = crate::storage::accounts::get_account_data(&e, account_id);

    // If e-mode category is set (> 0), isolation must be off
    if account_data.e_mode_category > 0 {
        cvlr_assert!(!account_data.is_isolated);
    }

    // If isolated, e-mode must be 0
    if account_data.is_isolated {
        cvlr_assert!(account_data.e_mode_category == 0);
    }
}

// ---------------------------------------------------------------------------
// Rule 6: Isolated accounts have at most one collateral asset
// ---------------------------------------------------------------------------

#[rule]
fn isolated_single_collateral(e: Env, account_id: u64) {
    let account_data = crate::storage::accounts::get_account_data(&e, account_id);

    if account_data.is_isolated {
        let deposit_count = crate::storage::positions::count_positions(
            &e,
            account_id,
            common::types::POSITION_TYPE_DEPOSIT,
        );
        cvlr_assert!(deposit_count <= 1);

        // If there is exactly one deposit, it must be the isolated asset
        if deposit_count == 1 {
            let deposit_list = crate::storage::positions::get_position_list(
                &e,
                account_id,
                common::types::POSITION_TYPE_DEPOSIT,
            );
            let deposit_asset = deposit_list.get(0).unwrap();
            cvlr_assert!(deposit_asset == account_data.isolated_asset);
        }
    }
}

// ---------------------------------------------------------------------------
// Rule 7: Borrow in isolation mode respects debt ceiling
// ---------------------------------------------------------------------------

/// After a borrow in isolation mode, the global isolated debt for that asset
/// must not exceed the configured debt ceiling.
#[rule]
fn isolation_debt_ceiling_respected(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let account_data = crate::storage::accounts::get_account_data(&e, account_id);
    cvlr_assume!(account_data.is_isolated);

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset.clone(), amount);

    // After borrow, check ceiling
    let isolated_asset = account_data.isolated_asset;
    let isolated_config = crate::storage::asset_config::get_asset_config(&e, &isolated_asset);
    let current_debt = crate::storage::isolation::get_isolated_debt(&e, &isolated_asset);

    cvlr_assert!(current_debt <= isolated_config.debt_ceiling_usd_wad);
}

// ---------------------------------------------------------------------------
// Sanity
// ---------------------------------------------------------------------------

#[rule]
fn isolation_sanity(e: Env, account_id: u64) {
    let data = crate::storage::accounts::get_account_data(&e, account_id);
    cvlr_satisfy!(data.is_isolated);
}

#[rule]
fn emode_sanity(e: Env, account_id: u64) {
    let data = crate::storage::accounts::get_account_data(&e, account_id);
    cvlr_satisfy!(data.e_mode_category > 0);
}
