use crate::events::FlashLoanEvent;
use common::types::HubAssetKey;
use common::validation::{require_positive_amount, require_wasm_receiver};
use soroban_sdk::{Address, Bytes, Env};

use super::require_strategy_caller;
use crate::config;
use crate::context::Cache;
use crate::external::pool::pool_flash_loan_call;
use crate::storage;

/// Initiates a flash loan of `amount` of `hub_asset` through the pool: the
/// pool sends the funds to `receiver`, invokes its callback with `data`, and
/// pulls back principal plus fee before this call returns. Requires
/// `receiver` to be a WASM contract and marks flash-loan state for the
/// duration of the pool call, blocking nested strategy calls. Emits
/// `FlashLoanEvent` with the fee charged.
pub(crate) fn process_flash_loan(
    env: &Env,
    caller: &Address,
    hub_asset: &HubAssetKey,
    amount: i128,
    receiver: &Address,
    data: &Bytes,
) {
    require_strategy_caller(env, caller);
    require_positive_amount(env, amount);
    config::require_hub_active(env, hub_asset.hub_id);

    require_wasm_receiver(env, receiver);

    let mut cache = Cache::new(env);
    let pool_addr = cache.cached_pool_address();

    let fee = storage::with_flash_guard(env, || {
        pool_flash_loan_call(env, &pool_addr, hub_asset, caller, receiver, amount, data)
    });

    FlashLoanEvent {
        hub_id: hub_asset.hub_id,
        asset: hub_asset.asset.clone(),
        receiver: receiver.clone(),
        caller: caller.clone(),
        amount,
        fee,
    }
    .publish(env);
}
