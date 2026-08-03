use common::errors::{FlashLoanError, GenericError};
use common::math::fp::{Bps, Ray};
use common::types::HubAssetKey;
use common::validation::{require_positive_amount, require_wasm_receiver};

use soroban_sdk::{
    assert_with_error, panic_with_error, token, Address, Bytes, Env, IntoVal, Symbol,
};

use crate::cache::Cache;
use crate::{events, interest, ops};

pub(crate) struct FlashTerms {
    pub(crate) fee: i128,
    pub(crate) total_repayment: i128,
    pub(crate) balance_after_payout: i128,
    pub(crate) balance_after_repayment: i128,
}

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
    assert_with_error!(
        env,
        cache.params().is_flashloanable,
        FlashLoanError::FlashloanNotEnabled
    );

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

    payout(
        env,
        &asset,
        &pool,
        &receiver,
        amount,
        terms.balance_after_payout,
    );
    invoke_receiver(
        env, &cache, &receiver, initiator, amount, terms.fee, &pool, data,
    );

    require_balance(env, &asset, &pool, terms.balance_after_payout);
    collect_repayment(env, &asset, &pool, &receiver, &terms);

    book_fee(&mut cache, terms.fee);

    events::emit_market_state(env, cache.commit());
    terms.fee
}

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

pub(crate) fn book_fee(cache: &mut Cache, fee: i128) {
    let protocol_fee = Ray::from_asset(fee, cache.params().asset_decimals);
    interest::add_protocol_revenue(cache, protocol_fee);
    cache.credit_cash(fee);
}

fn payout(
    env: &Env,
    asset: &token::Client,
    pool: &Address,
    receiver: &Address,
    amount: i128,
    expected_balance: i128,
) {
    asset.transfer(pool, receiver, &amount);
    require_balance(env, asset, pool, expected_balance);
}

#[allow(clippy::too_many_arguments)]
fn invoke_receiver(
    env: &Env,
    cache: &Cache,
    receiver: &Address,
    initiator: Address,
    amount: i128,
    fee: i128,
    pool: &Address,
    data: Bytes,
) {
    env.invoke_contract::<()>(
        receiver,
        &Symbol::new(env, "execute_flash_loan"),
        (
            initiator,
            cache.params().asset_id.clone(),
            amount,
            fee,
            pool.clone(),
            data,
        )
            .into_val(env),
    );
}

fn collect_repayment(
    env: &Env,
    asset: &token::Client,
    pool: &Address,
    receiver: &Address,
    terms: &FlashTerms,
) {
    assert_with_error!(
        env,
        asset.allowance(receiver, pool) >= terms.total_repayment,
        FlashLoanError::InvalidFlashloanRepay
    );
    asset.transfer_from(pool, receiver, pool, &terms.total_repayment);
    require_balance(env, asset, pool, terms.balance_after_repayment);
}

fn require_balance(env: &Env, asset: &token::Client, pool: &Address, expected: i128) {
    assert_with_error!(
        env,
        asset.balance(pool) == expected,
        FlashLoanError::InvalidFlashloanRepay
    );
}
