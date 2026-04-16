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
    // 1. Authenticate caller
    caller.require_auth();

    // 2. Pause + reentrancy guard
    validation::require_not_paused(env);
    validation::require_not_flash_loaning(env);

    // 3. Validate amount > 0
    validation::require_amount_positive(env, amount);
    validation::require_market_active(env, asset);

    let mut cache = ControllerCache::new(env, true); // flash loan config read is safe

    // 4. Get asset config and verify flash-loanable
    let asset_config = cache.cached_asset_config(asset);
    if !asset_config.is_flashloanable {
        panic_with_error!(env, FlashLoanError::FlashloanNotEnabled);
    }

    // 5. Calculate fee: fee = amount * flashloan_fee_bps / BPS
    let fee = Bps::from_raw(asset_config.flashloan_fee_bps).apply_to(env, amount);

    // 6. Get pool address
    let pool_addr = cache.cached_pool_address(asset);

    // 7. Set reentrancy guard
    storage::set_flash_loan_ongoing(env, true);

    // 8. Call pool.flash_loan_begin -- pool transfers tokens to receiver
    let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);
    pool_client.flash_loan_begin(asset, &amount, receiver);

    // 9. Call the receiver callback directly:
    //    execute_flash_loan(initiator, asset, amount, fee, data)
    env.invoke_contract::<()>(
        receiver,
        &Symbol::new(env, "execute_flash_loan"),
        (caller.clone(), asset.clone(), amount, fee, data.clone()).into_val(env),
    );

    // 10. Call pool.flash_loan_end -- pool pulls (amount + fee) from receiver
    pool_client.flash_loan_end(asset, &amount, &fee, receiver);

    // 11. Clear reentrancy guard
    storage::set_flash_loan_ongoing(env, false);

    // 12. Emit FlashLoanEvent
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
