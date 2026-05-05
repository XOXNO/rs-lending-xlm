use common::errors::FlashLoanError;
use common::events::{emit_flash_loan, FlashLoanEvent};
use common::fp::Bps;
use soroban_sdk::{contractimpl, panic_with_error, Address, Bytes, Env, IntoVal, Symbol};
use stellar_macros::when_not_paused;

use crate::cache::ControllerCache;
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
    validation::require_market_active(env, asset);

    let mut cache = ControllerCache::new(env, true);

    let asset_config = cache.cached_asset_config(asset);
    if !asset_config.is_flashloanable {
        panic_with_error!(env, FlashLoanError::FlashloanNotEnabled);
    }

    let fee = Bps::from_raw(asset_config.flashloan_fee_bps).apply_to(env, amount);
    let pool_addr = cache.cached_pool_address(asset);

    // Engage reentrancy guard for the duration of the callback.
    storage::set_flash_loan_ongoing(env, true);

    pool_flash_loan_begin_call(env, &pool_addr, amount, receiver);

    // Callback signature: execute_flash_loan(initiator, asset, amount, fee, data).
    env.invoke_contract::<()>(
        receiver,
        &Symbol::new(env, "execute_flash_loan"),
        (caller.clone(), asset.clone(), amount, fee, data.clone()).into_val(env),
    );

    pool_flash_loan_end_call(env, &pool_addr, amount, fee, receiver);

    storage::set_flash_loan_ongoing(env, false);

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

crate::summarized!(
    pool::flash_loan_begin_summary,
    fn pool_flash_loan_begin_call(
        env: &Env,
        pool_addr: &Address,
        amount: i128,
        receiver: &Address,
    ) {
        pool_interface::LiquidityPoolClient::new(env, pool_addr).flash_loan_begin(&amount, receiver)
    }
);

crate::summarized!(
    pool::flash_loan_end_summary,
    fn pool_flash_loan_end_call(
        env: &Env,
        pool_addr: &Address,
        amount: i128,
        fee: i128,
        receiver: &Address,
    ) {
        pool_interface::LiquidityPoolClient::new(env, pool_addr)
            .flash_loan_end(&amount, &fee, receiver)
    }
);
