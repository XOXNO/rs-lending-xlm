//! Flash-loan reentrancy guard rules.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Bytes, Env};

/// Guard panics when a flash loan is already in progress.
#[rule]
fn flash_loan_guard_blocks_callers(e: Env) {
    crate::storage::set_flash_loan_ongoing(&e, true);

    crate::validation::require_not_flash_loaning(&e);

    cvlr_satisfy!(false);
}

/// Guard returns when no flash loan is in progress.
#[rule]
fn flash_loan_guard_allows_when_clear(e: Env) {
    crate::storage::set_flash_loan_ongoing(&e, false);

    crate::validation::require_not_flash_loaning(&e);

    cvlr_satisfy!(true);
}

/// Successful flash loan clears the ongoing guard.
#[rule]
fn flash_loan_guard_cleared_after_completion(
    e: Env,
    caller: Address,
    receiver: Address,
    asset: Address,
    amount: i128,
    data: Bytes,
) {
    cvlr_assume!(amount > 0);
    cvlr_assume!(!crate::storage::is_flash_loan_ongoing(&e));

    let mut cache = crate::cache::Cache::new(&e);
    let cfg = cache.cached_asset_config(&asset);
    cvlr_assume!(cfg.is_flashloanable);
    let market = crate::storage::get_market_config(&e, &asset);
    cvlr_assume!(market.status == crate::types::MarketStatus::Active);
    drop(cache);

    crate::strategies::flash_loan::process_flash_loan(
        &e, &caller, &asset, amount, &receiver, &data,
    );

    cvlr_assert!(!crate::storage::is_flash_loan_ongoing(&e));
}

/// Reachability witness for the flash-loan guard-clear success path.
#[rule]
fn flash_loan_guard_cleared_sanity(
    e: Env,
    caller: Address,
    receiver: Address,
    asset: Address,
    amount: i128,
    data: Bytes,
) {
    cvlr_assume!(amount > 0);
    cvlr_assume!(!crate::storage::is_flash_loan_ongoing(&e));

    let mut cache = crate::cache::Cache::new(&e);
    let cfg = cache.cached_asset_config(&asset);
    cvlr_assume!(cfg.is_flashloanable);
    let market = crate::storage::get_market_config(&e, &asset);
    cvlr_assume!(market.status == crate::types::MarketStatus::Active);
    drop(cache);

    crate::strategies::flash_loan::process_flash_loan(
        &e, &caller, &asset, amount, &receiver, &data,
    );

    cvlr_satisfy!(!crate::storage::is_flash_loan_ongoing(&e));
}

#[rule]
fn flash_loan_sanity(
    e: Env,
    caller: Address,
    receiver: Address,
    asset: Address,
    amount: i128,
    data: Bytes,
) {
    cvlr_assume!(amount > 0);
    crate::Controller::flash_loan(e, caller, asset, amount, receiver, data);
    cvlr_satisfy!(true);
}
