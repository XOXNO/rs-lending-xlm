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
use common::events::{emit_flash_loan, FlashLoanEvent};
use common::math::fp::Bps;
use soroban_sdk::{assert_with_error, contractimpl, Address, Bytes, Env, Executable};
use stellar_macros::when_not_paused;

use crate::cache::ControllerCache;
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
    validation::require_amount_positive(env, amount);

    let mut cache = ControllerCache::new(env, OraclePolicy::RiskDecreasing);
    validation::require_market_active(env, &mut cache, asset);

    let asset_config = cache.cached_asset_config(asset);
    assert_with_error!(
        env,
        asset_config.is_flashloanable,
        FlashLoanError::FlashloanNotEnabled
    );
    require_wasm_receiver(env, receiver);

    let fee = flash_loan_fee(env, asset_config.flashloan_fee, amount);
    let pool_addr = cache.cached_pool_address(asset);

    // Reentrancy guard.
    storage::set_flash_loan_ongoing(env, true);

    let state = pool_flash_loan_call(env, &pool_addr, caller, receiver, amount, fee, data);

    storage::set_flash_loan_ongoing(env, false);
    cache.record_market_update(&state);
    cache.emit_market_batch();

    emit_flash_loan(
        env,
        FlashLoanEvent {
            asset: asset.clone(),
            receiver: receiver.clone(),
            caller: caller.clone(),
            amount,
            fee,
        },
    );
}

fn require_wasm_receiver(env: &Env, receiver: &Address) {
    assert_with_error!(
        env,
        matches!(receiver.executable(), Some(Executable::Wasm(_))),
        FlashLoanError::InvalidFlashloanReceiver
    );
}

fn flash_loan_fee(env: &Env, fee: Bps, amount: i128) -> i128 {
    let amount_after_fee = fee.apply_to(env, amount);
    if fee.raw() > 0 && amount_after_fee == 0 {
        1
    } else {
        amount_after_fee
    }
}
