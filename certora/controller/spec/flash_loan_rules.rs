use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Bytes, Env};

use crate::spec::fixture;
use crate::types::HubAssetKey;
use controller_interface::ControllerInterface;

#[rule]
fn flash_loan_guard_blocks_callers(e: Env) {
    crate::storage::set_flash_loan_ongoing(&e, true);

    crate::risk::validation::require_not_flash_loaning(&e);

    cvlr_assert!(false);
}

#[rule]
fn flash_loan_guard_allows_when_clear(e: Env) {
    crate::storage::set_flash_loan_ongoing(&e, false);

    crate::risk::validation::require_not_flash_loaning(&e);

    cvlr_satisfy!(true);
}

#[rule]
fn flash_loan_guard_blocks_supply_entrypoint(e: Env, caller: Address, asset: Address) {
    crate::storage::set_flash_loan_ongoing(&e, true);

    crate::spec::compat::supply_single(
        e.clone(),
        caller,
        crate::spec::fixture::ACCOUNT_ID,
        asset,
        crate::constants::WAD,
    );

    cvlr_assert!(false);
}

/// Satisfy twin of `flash_loan_guard_blocks_supply_entrypoint`: the same call
/// on the same account with the guard clear completes, so the revert rule is a
/// statement about the guard rather than about an unreachable supply.
#[rule]
fn flash_loan_guard_blocks_supply_entrypoint_fixture_completes(
    e: Env,
    caller: Address,
    asset: Address,
) {
    let account_id = fixture::ACCOUNT_ID;
    crate::storage::set_flash_loan_ongoing(&e, false);
    fixture::seed_live_account(&e, account_id, &caller, &asset);

    crate::spec::compat::supply_single(
        e.clone(),
        caller,
        account_id,
        asset.clone(),
        crate::constants::WAD,
    );

    cvlr_satisfy!(crate::storage::get_supply_positions(&e, account_id)
        .get(fixture::hub_asset(&asset))
        .is_some());
}

#[rule]
fn flash_loan_guard_blocks_liquidation_entrypoint(
    e: Env,
    liquidator: Address,
    debt_asset: Address,
) {
    crate::storage::set_flash_loan_ongoing(&e, true);
    let mut payments = soroban_sdk::Vec::new(&e);
    payments.push_back((
        HubAssetKey {
            hub_id: crate::spec::fixture::HUB_ID,
            asset: debt_asset,
        },
        crate::constants::WAD,
    ));

    crate::Controller::liquidate(
        e.clone(),
        liquidator,
        crate::spec::fixture::ACCOUNT_ID,
        payments,
        crate::types::SeizeMode::Transfer,
    );

    cvlr_assert!(false);
}

/// Satisfy twin of `flash_loan_guard_blocks_liquidation_entrypoint`: a
/// collateralized, indebted account is liquidatable with the guard clear.
/// The fixture carries the collateral and debt the seize and repay legs need,
/// so completion is a real liquidation and not an early return.
#[rule]
fn flash_loan_guard_blocks_liquidation_entrypoint_fixture_completes(
    e: Env,
    liquidator: Address,
    owner: Address,
    debt_asset: Address,
    collateral_asset: Address,
) {
    let account_id = fixture::ACCOUNT_ID;
    cvlr_assume!(owner != liquidator);
    crate::storage::set_flash_loan_ongoing(&e, false);
    fixture::seed_live_account(&e, account_id, &owner, &debt_asset);
    fixture::seed_market(&e, &collateral_asset);
    fixture::seed_empty_books(&e, account_id);
    fixture::seed_debt_position(&e, account_id, &debt_asset, 10 * common::constants::RAY);
    fixture::seed_supply_position(
        &e,
        account_id,
        &collateral_asset,
        10 * common::constants::RAY,
    );

    let mut payments = soroban_sdk::Vec::new(&e);
    payments.push_back((
        HubAssetKey {
            hub_id: fixture::HUB_ID,
            asset: debt_asset,
        },
        crate::constants::WAD,
    ));

    crate::Controller::liquidate(
        e.clone(),
        liquidator,
        account_id,
        payments,
        crate::types::SeizeMode::Transfer,
    );

    cvlr_satisfy!(!crate::storage::is_flash_loan_ongoing(&e));
}

#[rule]
fn flash_loan_guard_cleared_after_summarized_pool_return(
    e: Env,
    caller: Address,
    receiver: Address,
    asset: Address,
    amount: i128,
) {
    let data = Bytes::new(&e);
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(!crate::storage::is_flash_loan_ongoing(&e));
    crate::spec::fixture::seed_market(&e, &asset);

    let hub_asset = HubAssetKey {
        hub_id: crate::spec::fixture::HUB_ID,
        asset: asset.clone(),
    };
    let mut cache = crate::context::Cache::new(&e);

    let sync = cache.cached_pool_sync_data(&hub_asset);
    cvlr_assume!(sync.params.is_flashloanable);
    cvlr_assume!(
        crate::storage::get_spoke_asset(&e, crate::spec::fixture::SPOKE_ID, &hub_asset).is_some()
    );
    drop(cache);

    crate::strategies::flash_loan::process_flash_loan(
        &e, &caller, &hub_asset, amount, &receiver, &data,
    );

    cvlr_assert!(!crate::storage::is_flash_loan_ongoing(&e));
}
