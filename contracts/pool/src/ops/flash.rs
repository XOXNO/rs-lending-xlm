//! Flash loan: pays out, calls back, pulls back principal plus fee. Repayment
//! is enforced by bracketing the callback with exact loaned-token balance
//! checks, so the market asset must be a well-behaved SAC.

use common::errors::{FlashLoanError, GenericError};
use common::math::fp::{Bps, Ray};
use common::types::HubAssetKey;
use common::validation::{require_positive_amount, require_wasm_receiver};

use soroban_sdk::{
    assert_with_error, panic_with_error, token, Address, Bytes, Env, IntoVal, Symbol,
};

use crate::cache::Cache;
use crate::{events, interest, ops};

/// Exact balance targets the production checks compare against.
pub(crate) struct FlashTerms {
    pub(crate) fee: i128,
    pub(crate) total_repayment: i128,
    pub(crate) balance_after_payout: i128,
    pub(crate) balance_after_repayment: i128,
}

/// Lends `amount` to `receiver`, invokes its `execute_flash_loan` callback,
/// pulls back `amount + fee`, and books the fee as protocol revenue. Returns
/// the fee.
pub(crate) fn apply(
    env: &Env,
    hub_asset: HubAssetKey,
    initiator: Address,
    receiver: Address,
    amount: i128,
    data: Bytes,
) -> i128 {
    require_positive_amount(env, amount);

    let mut cache = ops::renewed_market(env, &hub_asset);

    // Availability and fee are pool-owned: the market must be flashloanable and
    // the fee derives from its `flashloan_fee` bps.
    assert_with_error!(
        env,
        cache.params().is_flashloanable,
        FlashLoanError::FlashloanNotEnabled
    );
    // Gated on tracked `cash` while the payout below moves live SAC balance, so
    // tokens donated straight to the pool are never loanable.
    cache.require_reserves(amount);
    require_wasm_receiver(env, &receiver);

    let pool = env.current_contract_address();
    let asset = token::Client::new(env, &cache.params().asset_id);
    let terms = terms(
        env,
        amount,
        cache.params().flashloan_fee,
        asset.balance(&pool),
    );

    asset.transfer(&pool, &receiver, &amount);
    require_balance(env, &asset, &pool, terms.balance_after_payout);

    env.invoke_contract::<()>(
        &receiver,
        &Symbol::new(env, "execute_flash_loan"),
        (
            initiator,
            cache.params().asset_id.clone(),
            amount,
            terms.fee,
            pool.clone(),
            data,
        )
            .into_val(env),
    );

    // The callback must not change the pool's loaned-token balance.
    require_balance(env, &asset, &pool, terms.balance_after_payout);

    assert_with_error!(
        env,
        asset.allowance(&receiver, &pool) >= terms.total_repayment,
        FlashLoanError::InvalidFlashloanRepay
    );
    asset.transfer_from(&pool, &receiver, &pool, &terms.total_repayment);
    require_balance(env, &asset, &pool, terms.balance_after_repayment);

    // CEI does not apply here: the callback is the point of the entrypoint, so
    // repayment is enforced by the balance checks bracketing it, not by ordering.
    // Net effect — the pool sent `amount` and got back `amount + fee`, so only the
    // fee touches tracked cash.
    book_fee(&mut cache, terms.fee);

    events::emit_market_state(env, cache.commit());
    terms.fee
}

/// Derives the fee and the three balances the loan must hit.
pub(crate) fn terms(env: &Env, amount: i128, fee_bps: u32, pre_balance: i128) -> FlashTerms {
    let fee = Bps::from(i128::from(fee_bps)).flash_loan_fee_on(env, amount);
    FlashTerms {
        fee,
        total_repayment: amount
            .checked_add(fee)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)),
        balance_after_payout: pre_balance
            .checked_sub(amount)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)),
        balance_after_repayment: pre_balance
            .checked_add(fee)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)),
    }
}

/// Books a settled flash-loan fee as protocol revenue and credits the cash.
pub(crate) fn book_fee(cache: &mut Cache, fee: i128) {
    let protocol_fee = Ray::from_asset(fee, cache.params().asset_decimals);
    interest::add_protocol_revenue(cache, protocol_fee);
    cache.credit_cash(fee);
}

/// Asserts the pool's loaned-token balance equals `expected`.
fn require_balance(env: &Env, asset: &token::Client, pool: &Address, expected: i128) {
    assert_with_error!(
        env,
        asset.balance(pool) == expected,
        FlashLoanError::InvalidFlashloanRepay
    );
}
