//! User flash loans with callback-scoped reentrancy guard.

use crate::events::FlashLoanEvent;
use common::errors::FlashLoanError;
use common::math::fp::Bps;
use common::types::HubAssetKey;
use soroban_sdk::{assert_with_error, contractimpl, Address, Bytes, Env};
use stellar_macros::when_not_paused;

use crate::context::Cache;
use crate::external::pool::pool_flash_loan_call;
use crate::{risk::validation, storage, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn flash_loan(
        env: Env,
        caller: Address,
        asset: HubAssetKey,
        amount: i128,
        receiver: Address,
        data: Bytes,
    ) {
        process_flash_loan(&env, &caller, &asset, amount, &receiver, &data);
    }
}

pub fn process_flash_loan(
    env: &Env,
    caller: &Address,
    hub_asset: &HubAssetKey,
    amount: i128,
    receiver: &Address,
    data: &Bytes,
) {
    caller.require_auth();

    validation::require_not_flash_loaning(env);
    validation::require_positive_amount(env, amount);
    validation::require_hub_active(env, hub_asset.hub_id);

    let mut cache = Cache::new(env);
    validation::require_market_active(env, &mut cache, hub_asset);

    // Flash-loan eligibility and fee live on the pool market params.
    let params = cache.cached_pool_sync_data(hub_asset).params;
    assert_with_error!(
        env,
        params.is_flashloanable,
        FlashLoanError::FlashloanNotEnabled
    );
    validation::require_wasm_receiver(env, receiver);

    let fee = Bps::from(i128::from(params.flashloan_fee)).flash_loan_fee_on(env, amount);
    let pool_addr = cache.cached_pool_address();

    // Reentrancy guard.
    storage::with_flash_guard(env, || {
        pool_flash_loan_call(
            env, &pool_addr, hub_asset, caller, receiver, amount, fee, data,
        );
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
