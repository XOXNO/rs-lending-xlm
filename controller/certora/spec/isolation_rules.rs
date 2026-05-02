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
/// capacity AND eligible for liquidation -- an impossible state.
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
//
// Inductive form lives in `emode_rules::emode_isolation_mutual_exclusion_after_supply`.
// A read-only rule over havoced storage is vacuous because storage can
// freely produce both flags set; the invariant only holds across the
// writing entry points (supply/multiply) that mutate AccountMeta.
// ---------------------------------------------------------------------------

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

/// On the success path of an isolated borrow, the global isolated debt
/// counter for the isolated collateral asset must remain at or below the
/// asset's `isolation_debt_ceiling_usd_wad`. Vacuity guards:
///   - require the account is isolated AND has a concrete `isolated_asset`;
///   - the borrow must execute successfully (control reaches the assertion).
#[rule]
fn isolation_debt_ceiling_respected(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    // Read the raw meta so we can require an actual isolated collateral
    // (the `accounts::get_account_data` shim defaults `isolated_asset` to
    // the account owner when None, which would let the rule pass against
    // an unrelated address).
    let meta = crate::storage::get_account_meta(&e, account_id);
    cvlr_assume!(meta.is_isolated);
    cvlr_assume!(meta.isolated_asset.is_some());
    let isolated_asset = meta.isolated_asset.unwrap();

    // Execute the borrow via the public auth path. If this reverts the
    // assertion below is unreachable, so the rule covers the success path
    // explicitly.
    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset, amount);

    // Read ceiling from the live MarketConfig and the isolated-debt counter.
    let market = crate::storage::get_market_config(&e, &isolated_asset);
    let current_debt = crate::storage::get_isolated_debt(&e, &isolated_asset);
    cvlr_assert!(current_debt <= market.asset_config.isolation_debt_ceiling_usd_wad);
}

// ---------------------------------------------------------------------------
// Rule 7b: Repay in isolation mode strictly decreases the isolated-debt counter
// ---------------------------------------------------------------------------

/// A repayment of an isolated borrow must strictly reduce the global
/// `IsolatedDebt(asset)` counter when the repayment amount is positive and
/// the borrow side actually held that asset.
#[rule]
fn isolation_repay_decreases_counter(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0);

    let meta = crate::storage::get_account_meta(&e, account_id);
    cvlr_assume!(meta.is_isolated);
    cvlr_assume!(meta.isolated_asset.is_some());
    let isolated_asset = meta.isolated_asset.unwrap();

    // The account must already owe the repaid asset, otherwise repay is a
    // no-op and the counter would not move.
    let borrow_pos = crate::storage::get_position(
        &e,
        account_id,
        common::types::POSITION_TYPE_BORROW,
        &asset,
    );
    cvlr_assume!(borrow_pos.is_some());
    cvlr_assume!(borrow_pos.unwrap().scaled_amount_ray > 0);

    let debt_before = crate::storage::get_isolated_debt(&e, &isolated_asset);
    cvlr_assume!(debt_before > 0);

    crate::spec::compat::repay_single(e.clone(), caller, account_id, asset, amount);

    let debt_after = crate::storage::get_isolated_debt(&e, &isolated_asset);
    cvlr_assert!(debt_after < debt_before);
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
