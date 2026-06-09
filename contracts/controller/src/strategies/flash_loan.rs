//! User-facing flash-loan entrypoint and settlement.
//!
//! The implementation follows ADR 0006: the pool itself snapshots its
//! balance, the receiver is called while the `FlashLoanOngoing` guard is
//! held, and repayment is proven by the pool's post-callback balance
//! (never by trusting a return value).
//!
//! The same guard flag is what makes strategy router callbacks
//! non-reentrant into the rest of the controller.

use common::errors::FlashLoanError;
use common::events::FlashLoanEvent;
use common::math::fp::Bps;
use soroban_sdk::{assert_with_error, contractimpl, Address, Bytes, Env};
use stellar_macros::when_not_paused;

use crate::cache::Cache;
use crate::cross_contract::pool::pool_flash_loan_call;
use crate::oracle::policy::OraclePolicy;
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

    let mut cache = Cache::new(env, OraclePolicy::RiskDecreasing);
    validation::require_market_active(env, &mut cache, asset);

    let asset_config = cache.cached_asset_config(asset);
    assert_with_error!(
        env,
        asset_config.is_flashloanable,
        FlashLoanError::FlashloanNotEnabled
    );
    validation::require_wasm_receiver(env, receiver);

    let fee = flash_loan_fee(env, asset_config.flashloan_fee, amount);
    let pool_addr = cache.cached_pool_address();

    // Reentrancy guard.
    storage::set_flash_loan_ongoing(env, true);

    let state = pool_flash_loan_call(env, &pool_addr, asset, caller, receiver, amount, fee, data);

    storage::set_flash_loan_ongoing(env, false);
    cache.record_market_update(&state);
    cache.emit_market_batch();

    FlashLoanEvent {
        asset: asset.clone(),
        receiver: receiver.clone(),
        caller: caller.clone(),
        amount,
        fee,
    }
    .publish(env);
}

fn flash_loan_fee(env: &Env, fee: Bps, amount: i128) -> i128 {
    let fee_amount = fee.apply_to(env, amount);
    if fee.raw() > 0 && fee_amount == 0 {
        1
    } else {
        fee_amount
    }
}
