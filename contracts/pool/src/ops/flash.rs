//! Flash-loan flow: pays out principal, invokes the receiver callback, pulls
//! back principal plus fee, then books the fee.
//!
//! Asserts the pool SAC balance after principal payout, again after the
//! receiver callback (must still equal post-payout), and after principal+fee
//! collection.
//!
//! [`prepare`], [`terms`], [`book_fee`], and [`finalize`] are the accounting
//! helpers used by [`apply`]; this same set is the surface Certora verifies
//! without modeling SAC/callback hosts.

use common::errors::{FlashLoanError, GenericError};
use common::math::fp::{Bps, Ray};
use common::types::HubAssetKey;
use common::validation::{require_positive_amount, require_wasm_receiver};

use soroban_sdk::{
    assert_with_error, panic_with_error, token, Address, Bytes, Env, IntoVal, Symbol,
};

use crate::cache::Cache;
use crate::{events, interest, ops};

/// Precomputed fee and expected pool balances for a flash loan.
pub(crate) struct FlashTerms {
    /// Protocol fee in asset units.
    pub(crate) fee: i128,
    /// Principal + fee that must be repaid.
    pub(crate) total_repayment: i128,
    /// Expected token balance after paying out the principal.
    pub(crate) balance_after_payout: i128,
    /// Expected token balance after collecting principal + fee.
    pub(crate) balance_after_repayment: i128,
}

/// Executes a full flash loan of `amount` to `receiver` and returns the fee
/// charged. Requires the market to have `is_flashloanable` set and the
/// receiver to be a WASM contract implementing `execute_flash_loan`. Books
/// the fee as protocol revenue and cash.
pub(crate) fn apply(
    env: &Env,
    hub_asset: HubAssetKey,
    initiator: Address,
    receiver: Address,
    amount: i128,
    data: Bytes,
) -> i128 {
    let mut cache = prepare(env, hub_asset, amount);
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
    invoke_receiver(
        env, &cache, &receiver, initiator, amount, terms.fee, &pool, data,
    );

    require_balance(env, &asset, &pool, terms.balance_after_payout);
    collect_repayment(env, &asset, &pool, &receiver, &terms);

    finalize(env, &mut cache, terms.fee);
    terms.fee
}

/// Accrues interest, requires flash loans enabled, and requires cash reserves for `amount`.
///
/// Production front half of [`apply`] before SAC/callback steps.
pub(crate) fn prepare(env: &Env, hub_asset: HubAssetKey, amount: i128) -> Cache {
    require_positive_amount(env, amount);

    let cache = ops::renewed_market(env, &hub_asset);
    assert_with_error!(
        env,
        cache.params().is_flashloanable,
        FlashLoanError::FlashloanNotEnabled
    );
    cache.require_reserves(amount);
    cache
}

/// Accrues, requires flash loans enabled and cash for `amount`, then builds
/// repayment terms from a symbolic/pre-loan `pre_balance`.
///
/// Composes [`prepare`] + [`terms`] exactly as [`apply`] does after reading the
/// live SAC balance, letting a caller supply that balance instead of reading a
/// SAC. Nothing in the production path needs that -- [`apply`] reads the live
/// balance itself -- so this is compiled only for the Certora specs that drive
/// full successful-path accounting symbolically, and for the unit tests that
/// pin the composition. It is deliberately not part of the deployed contract.
#[cfg(any(test, feature = "certora"))]
pub(crate) fn prepare_with_balance(
    env: &Env,
    hub_asset: HubAssetKey,
    amount: i128,
    pre_balance: i128,
) -> (Cache, FlashTerms) {
    let cache = prepare(env, hub_asset, amount);
    let t = terms(env, amount, cache.params().flashloan_fee, pre_balance);
    (cache, t)
}

/// Computes fee and expected pool balances from the pre-loan token balance.
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

/// Credits cash and mints protocol revenue for the flash-loan fee.
pub(crate) fn book_fee(cache: &mut Cache, fee: i128) {
    let protocol_fee = Ray::from_asset(fee, cache.params().asset_decimals);
    interest::add_protocol_revenue(cache, protocol_fee);
    cache.credit_cash(fee);
}

/// Successful-path tail of [`apply`]: books the fee, commits the market, and emits market state.
///
/// Called only after SAC balance checks confirm principal+fee returned.
pub(crate) fn finalize(env: &Env, cache: &mut Cache, fee: i128) {
    book_fee(cache, fee);
    events::emit_market_state(env, cache.commit());
}

/// Calls `execute_flash_loan` on the receiver with loan parameters and callback data.
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

/// Pulls principal + fee via `transfer_from` after verifying allowance.
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

/// Asserts the pool's token balance equals `expected`.
fn require_balance(env: &Env, asset: &token::Client, pool: &Address, expected: i128) {
    assert_with_error!(
        env,
        asset.balance(pool) == expected,
        FlashLoanError::InvalidFlashloanRepay
    );
}
