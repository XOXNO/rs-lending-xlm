//! Flash loan strategy: validates a flash loan request, dispatches it to the
//! pool, and publishes the resulting event.

use crate::events::FlashLoanEvent;
use common::types::HubAssetKey;
use common::validation::{require_positive_amount, require_wasm_receiver};
use soroban_sdk::{Address, Bytes, Env};

use crate::config;
use crate::context::Cache;
use crate::external::pool::pool_flash_loan_call;
use crate::{risk::validation, storage};

/// Executes a flash loan of `amount` of `hub_asset` to `receiver`, invoking the
/// pool's flash loan callback with `data`. Requires `caller` authorization,
/// rejects a nested flash loan call, requires a positive `amount`, requires the
/// hub to be active, and requires `receiver` to be a WASM contract. Publishes a
/// `FlashLoanEvent` carrying the fee charged by the pool.
pub(crate) fn process_flash_loan(
    env: &Env,
    caller: &Address,
    hub_asset: &HubAssetKey,
    amount: i128,
    receiver: &Address,
    data: &Bytes,
) {
    caller.require_auth();

    validation::require_not_flash_loaning(env);
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
