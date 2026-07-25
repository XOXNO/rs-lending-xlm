//! User flash loans: pool pays `receiver`, exact principal+fee repaid in-callback.
//!
//! Caller auth; no account/HF. Reentrancy guard blocks nested controller entry
//! for the callback. See `docs/reference/invariants.md` §2.5.

use crate::events::FlashLoanEvent;
use common::types::HubAssetKey;
use common::validation::{require_positive_amount, require_wasm_receiver};
use soroban_sdk::{Address, Bytes, Env};

use crate::config;
use crate::context::Cache;
use crate::external::pool::pool_flash_loan_call;
use crate::{risk::validation, storage};

/// Pool flash loan to `receiver` with principal+fee repaid before return. No account.
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

    // Availability (`is_flashloanable`) and fee are pool-owned: the pool gates
    // the market, computes the fee from its `flashloan_fee` bps, and returns it
    // for the event. A non-market asset reverts pool-side. Flash loans never
    // price, so no oracle gate is needed. The guard blocks nested entry.
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
