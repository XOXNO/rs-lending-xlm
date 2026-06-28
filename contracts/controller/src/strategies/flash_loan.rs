//! User-facing flash-loan entrypoint and settlement.
//!
//! Repayment is checked by the pool after the receiver callback. The controller
//! holds `FlashLoanOngoing` during the callback to block re-entrant mutations.

use crate::events::FlashLoanEvent;
use common::errors::FlashLoanError;
use common::math::fp::Bps;
use soroban_sdk::{assert_with_error, contractimpl, Address, Bytes, Env};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::external::pool::pool_flash_loan_call;
use crate::helpers::utils::hub0;
use crate::{storage, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[when_not_paused]
    pub fn flash_loan(
        env: Env,
        caller: Address,
        asset: Address,
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
    asset: &Address,
    amount: i128,
    receiver: &Address,
    data: &Bytes,
) {
    caller.require_auth();

    validation::require_not_flash_loaning(env);
    validation::require_positive_amount(env, amount);

    let mut cache = Cache::new(env);
    let hub_asset = hub0(asset);
    validation::require_market_active(env, &mut cache, &hub_asset);

    // Flash-loan eligibility and fee live on the pool market params.
    let params = cache.cached_pool_sync_data(&hub_asset).params;
    assert_with_error!(
        env,
        params.is_flashloanable,
        FlashLoanError::FlashloanNotEnabled
    );
    validation::require_wasm_receiver(env, receiver);

    let fee = Bps::from(i128::from(params.flashloan_fee_bps)).flash_loan_fee_on(env, amount);
    let pool_addr = cache.cached_pool_address();

    // Reentrancy guard.
    storage::with_flash_guard(env, || {
        pool_flash_loan_call(env, &pool_addr, &hub_asset, caller, receiver, amount, fee, data);
    });

    FlashLoanEvent {
        asset: asset.clone(),
        receiver: receiver.clone(),
        caller: caller.clone(),
        amount,
        fee,
    }
    .publish(env);
}
