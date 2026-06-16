//! Isolation mode and E-mode risk-parameter invariants.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::BPS;

/// LTV is strictly below the liquidation threshold for every asset.
#[rule]
fn ltv_less_than_liquidation_threshold(e: Env, asset: Address) {
    let config = crate::storage::asset_config::get_asset_config(&e, &asset);

    cvlr_assert!(config.loan_to_value_bps < config.liquidation_threshold_bps);
}

/// Liquidation threshold times bonus does not exceed 100%.
#[rule]
fn liquidation_bonus_capped(e: Env, asset: Address) {
    let config = crate::storage::asset_config::get_asset_config(&e, &asset);

    cvlr_assert!(
        config.liquidation_threshold_bps * (BPS + config.liquidation_bonus_bps) <= BPS * BPS
    );
}

/// Reserve factor stays below 100%.
#[rule]
fn reserve_factor_bounded(e: Env, asset: Address) {
    let config = crate::storage::asset_config::get_asset_config(&e, &asset);

    cvlr_assert!(config.reserve_factor_bps < 10000);
}

// `utilization_params_ordered` removed: it asserted IRM ordering on params read
// via `get_sync_data_summary`, which havocs them with no ordering constraint, so
// the assert was not entailed. The ordering invariant is enforced by
// `MarketParamsRaw::verify` at config time and belongs in a rule that exercises
// that validator directly.

/// Isolated accounts hold at most one collateral asset.
#[rule]
fn isolated_single_collateral(e: Env, account_id: u64) {
    let account_data = crate::storage::accounts::get_account_data(&e, account_id);

    if account_data.is_isolated {
        let deposit_count = crate::storage::positions::count_positions(
            &e,
            account_id,
            crate::types::AccountPositionType::Deposit,
        );
        cvlr_assert!(deposit_count <= 1);

        if deposit_count == 1 {
            let deposit_list = crate::storage::positions::get_position_list(
                &e,
                account_id,
                crate::types::AccountPositionType::Deposit,
            );
            let deposit_asset = deposit_list.get(0).unwrap();
            cvlr_assert!(deposit_asset == account_data.isolated_asset);
        }
    }
}

/// Successful isolated borrow increases and respects the debt ceiling.
#[rule]
fn isolation_debt_ceiling_respected(
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

    let debt_before = crate::storage::get_isolated_debt(&e, &isolated_asset);

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset, amount);

    let market = crate::storage::get_market_config(&e, &isolated_asset);
    let debt_after = crate::storage::get_isolated_debt(&e, &isolated_asset);

    cvlr_assert!(debt_after > debt_before);
    cvlr_assert!(debt_after <= market.asset_config.isolation_debt_ceiling_usd_wad);
}

/// Successful isolated repay decreases the counter or snaps dust to zero.
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

    let borrow_pos = crate::storage::get_position(
        &e,
        account_id,
        crate::types::AccountPositionType::Borrow,
        &asset,
    );
    cvlr_assume!(borrow_pos.is_some());
    cvlr_assume!(borrow_pos.unwrap().scaled_amount_ray > 0);

    let debt_before = crate::storage::get_isolated_debt(&e, &isolated_asset);
    cvlr_assume!(debt_before > 0);

    crate::spec::compat::repay_single(e.clone(), caller, account_id, asset, amount);

    let debt_after = crate::storage::get_isolated_debt(&e, &isolated_asset);
    cvlr_assert!(debt_after < debt_before || debt_after == 0);
}

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
