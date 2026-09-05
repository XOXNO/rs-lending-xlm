use crate::events::FlashLoanEvent;
use common::types::HubAssetKey;
use common::validation::{require_positive_amount, require_wasm_receiver};
use soroban_sdk::{Address, Bytes, Env};

use crate::config;
use crate::context::Context;
use crate::external::pool::pool_flash_loan_call;
use crate::risk::validation::require_authorized_caller;
use crate::storage;

/// Runs a pool flash loan under the reentrancy guard. The pool funds the
/// receiver, calls it and collects principal plus fee before returning.
pub(crate) fn process_flash_loan(
    env: &Env,
    caller: &Address,
    hub_asset: &HubAssetKey,
    amount: i128,
    receiver: &Address,
    data: &Bytes,
) {
    require_authorized_caller(env, caller);
    require_positive_amount(env, amount);
    config::require_hub_active(env, hub_asset.hub_id);

    require_wasm_receiver(env, receiver);

    let mut cache = Context::new(env);
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
