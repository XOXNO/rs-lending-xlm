//! Flash-loan reentrancy guard rules.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Bytes, Env};

use crate::types::HubAssetKey;

/// Guard panics when a flash loan is already in progress.
#[rule]
fn flash_loan_guard_blocks_callers(e: Env) {
    crate::storage::set_flash_loan_ongoing(&e, true);

    crate::risk::validation::require_not_flash_loaning(&e);

    cvlr_satisfy!(false);
}

/// Guard returns when no flash loan is in progress.
#[rule]
fn flash_loan_guard_allows_when_clear(e: Env) {
    crate::storage::set_flash_loan_ongoing(&e, false);

    crate::risk::validation::require_not_flash_loaning(&e);

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

    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    };
    let mut cache = crate::context::Cache::new(&e);
    // Flash-loan eligibility now lives on the pool's `MarketParamsRaw`.
    let sync = cache.cached_pool_sync_data(&hub_asset);
    cvlr_assume!(sync.params.is_flashloanable);
    // "Active" market proxy under the token-rooted oracle: the asset is listed on
    // spoke 0 and has a configured `AssetOracle`.
    cvlr_assume!(crate::storage::get_spoke_asset(&e, 0, &hub_asset).is_some());
    cvlr_assume!(crate::storage::get_asset_oracle(&e, &asset).is_some());
    drop(cache);

    crate::strategies::flash_loan::process_flash_loan(
        &e, &caller, &hub_asset, amount, &receiver, &data,
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

    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    };
    let mut cache = crate::context::Cache::new(&e);
    let sync = cache.cached_pool_sync_data(&hub_asset);
    cvlr_assume!(sync.params.is_flashloanable);
    cvlr_assume!(crate::storage::get_spoke_asset(&e, 0, &hub_asset).is_some());
    cvlr_assume!(crate::storage::get_asset_oracle(&e, &asset).is_some());
    drop(cache);

    crate::strategies::flash_loan::process_flash_loan(
        &e, &caller, &hub_asset, amount, &receiver, &data,
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
    let hub_asset = HubAssetKey { hub_id: 0, asset };
    crate::Controller::flash_loan(e, caller, hub_asset, amount, receiver, data);
    cvlr_satisfy!(true);
}
