use common::errors::FlashLoanError;
use common::events::{emit_flash_loan, FlashLoanEvent};
use common::fp::Bps;
use soroban_sdk::{panic_with_error, Address, Bytes, Env, IntoVal, Symbol};

use crate::cache::ControllerCache;
use crate::{storage, validation};

pub fn process_flash_loan(
    env: &Env,
    caller: &Address,
    asset: &Address,
    amount: i128,
    receiver: &Address,
    data: &Bytes,
) {
    caller.require_auth();

    validation::require_not_paused(env);
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

    let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);
    pool_client.flash_loan_begin(&amount, receiver);

    // Callback signature: execute_flash_loan(initiator, asset, amount, fee, data).
    env.invoke_contract::<()>(
        receiver,
        &Symbol::new(env, "execute_flash_loan"),
        (caller.clone(), asset.clone(), amount, fee, data.clone()).into_val(env),
    );

    pool_client.flash_loan_end(&amount, &fee, receiver);

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
