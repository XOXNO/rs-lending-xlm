//! Flash-loan reentrancy guard rules.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Bytes, Env};

use crate::types::HubAssetKey;

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

/// The production supply entrypoint reaches the flash-loan guard before any
/// account or pool mutation, modeling a representative callback reentry.
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

/// The production liquidation entrypoint also rejects callback reentry while
/// the outer flash loan owns the guard.
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
    );

    cvlr_assert!(false);
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
    // Flash-loanable on pool `MarketParamsRaw`; oracle priceability is modeled
    // by the external price-aggregator harness.
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
