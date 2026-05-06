use common::errors::FlashLoanError;
use common::events::{emit_flash_loan, FlashLoanEvent};
use common::fp::Bps;
use soroban_sdk::{contractimpl, panic_with_error, Address, Bytes, Env, Executable};
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
    require_wasm_receiver(env, receiver);

    let fee = flash_loan_fee(env, asset_config.flashloan_fee_bps, amount);
    let pool_addr = cache.cached_pool_address(asset);

    // Engage reentrancy guard before the pool calls the receiver callback.
    storage::set_flash_loan_ongoing(env, true);

    pool_flash_loan_call(env, &pool_addr, caller, receiver, amount, fee, data);

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

fn require_wasm_receiver(env: &Env, receiver: &Address) {
    if !matches!(receiver.executable(), Some(Executable::Wasm(_))) {
        panic_with_error!(env, FlashLoanError::InvalidFlashloanReceiver);
    }
}

fn flash_loan_fee(env: &Env, fee_bps: u32, amount: i128) -> i128 {
    let fee = Bps::from_raw(fee_bps).apply_to(env, amount);
    if fee_bps > 0 && fee == 0 {
        1
    } else {
        fee
    }
}

crate::summarized!(
    pool::flash_loan_summary,
    fn pool_flash_loan_call(
        env: &Env,
        pool_addr: &Address,
        initiator: &Address,
        receiver: &Address,
        amount: i128,
        fee: i128,
        data: &Bytes,
    ) {
        pool_interface::LiquidityPoolClient::new(env, pool_addr)
            .flash_loan(initiator, receiver, &amount, &fee, data)
    }
);
